//! `settle_challenge` (Task 11): read the decision-market TWAP from the REAL
//! deployed MetaDAO `amm` v0.4 binary, apply the slash trigger, and resolve the
//! conditional-vault question — all driven against the real programs in LiteSVM.
//!
//! Each test composes the MetaDAO market exactly like `open_challenge.rs` (a
//! binary question whose resolver is the Kassandra oracle PDA + KASS/USDC
//! conditional vaults), then builds GENUINE pass/fail AMM pools via the real
//! `create_amm` + `add_liquidity` + `crank_that_twap` instructions so the TWAP
//! `settle_challenge` reads is produced by the real binary — not fabricated.
//! `open_challenge` records the real AMM addresses on the `Market`; `settle`
//! then HARD-binds each AMM to this market's conditional mint pair, reads the
//! TWAP, and slashes / resolves accordingly.

mod common;
use common::*;

use kassandra_program::{
    cpi::metadao,
    error::KassandraError,
    instruction::Ix,
    state::{AccountType, AiClaim, Market, Phase},
};
use solana_sdk::instruction::InstructionError;
use solana_sdk::{
    account::Account,
    compute_budget::ComputeBudgetInstruction,
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::TransactionError,
};
use spl_token::{
    solana_program::{program_option::COption, program_pack::Pack},
    state::{Account as TokenAccount, AccountState},
    ID as TOKEN_PROGRAM_ID,
};

const VAULT_SO: &[u8] = include_bytes!("fixtures/metadao_conditional_vault.so");
const AMM_SO: &[u8] = include_bytes!("fixtures/metadao_amm.so");

const ATA_PROGRAM_ID: Pubkey = solana_sdk::pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

/// Largest observation change per update — lets the recorded observation jump
/// straight to the pool's current price in a single crank, so a one-crank TWAP
/// equals the reserve-implied price (clean, deterministic test prices).
const MAX_PRICE: u128 = (u64::MAX as u128) * 1_000_000_000_000;

fn vault_id() -> Pubkey {
    Pubkey::new_from_array(metadao::CONDITIONAL_VAULT_ID)
}
fn amm_id() -> Pubkey {
    Pubkey::new_from_array(metadao::AMM_ID)
}

fn ata(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[owner.as_ref(), TOKEN_PROGRAM_ID.as_ref(), mint.as_ref()],
        &ATA_PROGRAM_ID,
    )
    .0
}

fn cond_mint(vault: &Pubkey, index: u8) -> Pubkey {
    Pubkey::find_program_address(
        &metadao::conditional_token_mint_seeds(&vault.to_bytes(), &[index]),
        &vault_id(),
    )
    .0
}

fn cu(ix: Instruction) -> [Instruction; 2] {
    [
        ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
        ix,
    ]
}

/// Fabricate a token account at `addr` holding `amount` of `mint`, owned by `owner`.
fn fabricate_token_account(
    ctx: &mut TestCtx,
    addr: Pubkey,
    mint: Pubkey,
    owner: Pubkey,
    amount: u64,
) {
    let state = TokenAccount {
        mint,
        owner,
        amount,
        delegate: COption::None,
        state: AccountState::Initialized,
        is_native: COption::None,
        delegated_amount: 0,
        close_authority: COption::None,
    };
    let mut data = vec![0u8; TokenAccount::LEN];
    state.pack_into_slice(&mut data);
    let lamports = ctx
        .svm
        .minimum_balance_for_rent_exemption(TokenAccount::LEN);
    ctx.svm
        .set_account(
            addr,
            Account {
                lamports,
                data,
                owner: TOKEN_PROGRAM_ID,
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();
}

/// Fabricate an account OWNED BY THE AMM PROGRAM with a valid `Amm`
/// discriminator but `base_mint`/`quote_mint` set to `base`/`quote` (which will
/// NOT match a real market's conditional mints). Used by the binding-attack
/// test: `open_challenge` records it (it only checks `owner == AMM_ID`), and
/// `settle` must reject it on the conditional mint-pair binding (the valid disc
/// proves the rejection is the MINT check, not the disc check).
fn fabricate_rogue_amm(ctx: &mut TestCtx, base: Pubkey, quote: Pubkey) -> Pubkey {
    let addr = Pubkey::new_unique();
    let mut data = vec![0u8; 256];
    data[..8].copy_from_slice(&metadao::AMM_ACCOUNT_DISCRIMINATOR);
    data[metadao::AMM_BASE_MINT_OFFSET..metadao::AMM_BASE_MINT_OFFSET + 32]
        .copy_from_slice(&base.to_bytes());
    data[metadao::AMM_QUOTE_MINT_OFFSET..metadao::AMM_QUOTE_MINT_OFFSET + 32]
        .copy_from_slice(&quote.to_bytes());
    let lamports = ctx.svm.minimum_balance_for_rent_exemption(256);
    ctx.svm
        .set_account(
            addr,
            Account {
                lamports,
                data,
                owner: amm_id(),
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();
    addr
}

struct MarketAccounts {
    question: Pubkey,
    kass_vault: Pubkey,
    kass_vault_underlying: Pubkey,
    usdc_vault: Pubkey,
    pass_mint: Pubkey, // pass-KASS  (cond(kass_vault, 0))
    fail_mint: Pubkey, // fail-KASS  (cond(kass_vault, 1))
    pass_usdc: Pubkey, // pass-USDC  (cond(usdc_vault, 0))
    fail_usdc: Pubkey, // fail-USDC  (cond(usdc_vault, 1))
}

/// Compose the MetaDAO market (binary question + KASS/USDC conditional vaults)
/// for `resolver` and return the bound mints/vaults plus the oracle-PDA-owned
/// conditional-KASS destinations for the proposer's split bond.
fn setup_market(ctx: &mut TestCtx, resolver: Pubkey) -> (MarketAccounts, Pubkey, Pubkey) {
    let kass = ctx.kass_mint;
    let usdc = ctx.usdc_mint;
    let resolver_arr = resolver.to_bytes();
    let kass_arr = kass.to_bytes();
    let usdc_arr = usdc.to_bytes();
    let num_outcomes: u8 = 2;
    let question_id = [7u8; 32];

    let (question, _) = Pubkey::find_program_address(
        &metadao::question_seeds(&question_id, &resolver_arr, &[num_outcomes]),
        &vault_id(),
    );
    let question_arr = question.to_bytes();
    let (kass_vault, _) =
        Pubkey::find_program_address(&metadao::vault_seeds(&question_arr, &kass_arr), &vault_id());
    let (usdc_vault, _) =
        Pubkey::find_program_address(&metadao::vault_seeds(&question_arr, &usdc_arr), &vault_id());

    let pass_mint = cond_mint(&kass_vault, 0);
    let fail_mint = cond_mint(&kass_vault, 1);
    let pass_usdc = cond_mint(&usdc_vault, 0);
    let fail_usdc = cond_mint(&usdc_vault, 1);
    let (event_authority, _) =
        Pubkey::find_program_address(&metadao::event_authority_seeds(), &vault_id());

    let kass_vault_underlying = ata(&kass_vault, &kass);
    let usdc_vault_underlying = ata(&usdc_vault, &usdc);

    let payer = ctx.payer.pubkey();
    let ix_q = Instruction {
        program_id: vault_id(),
        accounts: vec![
            AccountMeta::new(question, false),
            AccountMeta::new(payer, true),
            AccountMeta::new_readonly(solana_sdk::system_program::ID, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(vault_id(), false),
        ],
        data: metadao::initialize_question_data(&question_id, &resolver_arr, num_outcomes).to_vec(),
    };
    ctx.send_many(&cu(ix_q), &[])
        .expect("initialize_question failed");

    let ix_kv = Instruction {
        program_id: vault_id(),
        accounts: vec![
            AccountMeta::new(kass_vault, false),
            AccountMeta::new_readonly(question, false),
            AccountMeta::new_readonly(kass, false),
            AccountMeta::new(kass_vault_underlying, false),
            AccountMeta::new(payer, true),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(ATA_PROGRAM_ID, false),
            AccountMeta::new_readonly(solana_sdk::system_program::ID, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new(pass_mint, false),
            AccountMeta::new(fail_mint, false),
        ],
        data: metadao::initialize_conditional_vault_data().to_vec(),
    };
    ctx.send_many(&cu(ix_kv), &[])
        .expect("init KASS vault failed");

    let ix_uv = Instruction {
        program_id: vault_id(),
        accounts: vec![
            AccountMeta::new(usdc_vault, false),
            AccountMeta::new_readonly(question, false),
            AccountMeta::new_readonly(usdc, false),
            AccountMeta::new(usdc_vault_underlying, false),
            AccountMeta::new(payer, true),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(ATA_PROGRAM_ID, false),
            AccountMeta::new_readonly(solana_sdk::system_program::ID, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new(pass_usdc, false),
            AccountMeta::new(fail_usdc, false),
        ],
        data: metadao::initialize_conditional_vault_data().to_vec(),
    };
    ctx.send_many(&cu(ix_uv), &[])
        .expect("init USDC vault failed");

    let oracle_pass_kass = Pubkey::new_unique();
    let oracle_fail_kass = Pubkey::new_unique();
    fabricate_token_account(ctx, oracle_pass_kass, pass_mint, resolver, 0);
    fabricate_token_account(ctx, oracle_fail_kass, fail_mint, resolver, 0);

    (
        MarketAccounts {
            question,
            kass_vault,
            kass_vault_underlying,
            usdc_vault,
            pass_mint,
            fail_mint,
            pass_usdc,
            fail_usdc,
        },
        oracle_pass_kass,
        oracle_fail_kass,
    )
}

/// Build a GENUINE AMM pool via the real `amm` binary: `create_amm` (base/quote
/// = the given conditional mints) → `add_liquidity` (reserves) → warp ≥ 150
/// slots → `crank_that_twap`. After one crank with `MAX_PRICE` allowed change,
/// the recorded slot-weighted TWAP equals the reserve price
/// `quote_reserve * 1e12 / base_reserve`. Returns the AMM PDA.
fn build_amm(
    ctx: &mut TestCtx,
    base_mint: Pubkey,
    quote_mint: Pubkey,
    base_reserve: u64,
    quote_reserve: u64,
    crank: bool,
) -> Pubkey {
    let payer = ctx.payer.pubkey();
    let base_arr = base_mint.to_bytes();
    let quote_arr = quote_mint.to_bytes();
    let (amm, _) = Pubkey::find_program_address(
        &[b"amm__", base_arr.as_ref(), quote_arr.as_ref()],
        &amm_id(),
    );
    let amm_arr = amm.to_bytes();
    let (lp_mint, _) = Pubkey::find_program_address(&[b"amm_lp_mint", amm_arr.as_ref()], &amm_id());
    let vault_ata_base = ata(&amm, &base_mint);
    let vault_ata_quote = ata(&amm, &quote_mint);
    let (amm_event_auth, _) =
        Pubkey::find_program_address(&metadao::event_authority_seeds(), &amm_id());

    // Fund the user's base/quote with plenty for add_liquidity.
    let user_base = ata(&payer, &base_mint);
    let user_quote = ata(&payer, &quote_mint);
    fabricate_token_account(
        ctx,
        user_base,
        base_mint,
        payer,
        base_reserve.saturating_mul(4).max(1),
    );
    fabricate_token_account(
        ctx,
        user_quote,
        quote_mint,
        payer,
        quote_reserve.saturating_mul(4).max(1),
    );

    // --- create_amm (delayed-twap v0.4.1+): args =
    //     initial_observation:u128 ++ max_change:u128 ++ start_delay_slots:u64 ---
    let initial_obs: u128 = (quote_reserve as u128 * 1_000_000_000_000u128) / base_reserve as u128;
    let mut create_data = metadao::CREATE_AMM.to_vec();
    create_data.extend_from_slice(&initial_obs.to_le_bytes());
    create_data.extend_from_slice(&MAX_PRICE.to_le_bytes());
    create_data.extend_from_slice(&0u64.to_le_bytes()); // twap_start_delay_slots = 0
    let ix_create = Instruction {
        program_id: amm_id(),
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(amm, false),
            AccountMeta::new(lp_mint, false),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new(vault_ata_base, false),
            AccountMeta::new(vault_ata_quote, false),
            AccountMeta::new_readonly(ATA_PROGRAM_ID, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(solana_sdk::system_program::ID, false),
            AccountMeta::new_readonly(amm_event_auth, false),
            AccountMeta::new_readonly(amm_id(), false),
        ],
        data: create_data,
    };
    ctx.send_many(&cu(ix_create), &[])
        .expect("create_amm failed");

    // user LP account (created after create_amm so lp_mint exists).
    let user_lp = ata(&payer, &lp_mint);
    fabricate_token_account(ctx, user_lp, lp_mint, payer, 0);

    // --- add_liquidity: args = quote:u64 ++ max_base:u64 ++ min_lp:u64 ---
    let mut add_data = metadao::ADD_LIQUIDITY.to_vec();
    add_data.extend_from_slice(&quote_reserve.to_le_bytes());
    add_data.extend_from_slice(&base_reserve.to_le_bytes());
    add_data.extend_from_slice(&0u64.to_le_bytes());
    let ix_add = Instruction {
        program_id: amm_id(),
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(amm, false),
            AccountMeta::new(lp_mint, false),
            AccountMeta::new(user_lp, false),
            AccountMeta::new(user_base, false),
            AccountMeta::new(user_quote, false),
            AccountMeta::new(vault_ata_base, false),
            AccountMeta::new(vault_ata_quote, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(amm_event_auth, false),
            AccountMeta::new_readonly(amm_id(), false),
        ],
        data: add_data,
    };
    ctx.send_many(&cu(ix_add), &[])
        .expect("add_liquidity failed");

    // When `crank == false` the pool keeps reserves but NEVER records a TWAP
    // observation (aggregator stays 0 → settle reads its TWAP as 0). Used by the
    // pass_twap==0 survive test.
    if crank {
        // Advance > ONE_MINUTE_IN_SLOTS (150) so the crank records an observation.
        ctx.warp_slots(0, 300);

        let ix_crank = Instruction {
            program_id: amm_id(),
            accounts: vec![
                AccountMeta::new(amm, false),
                AccountMeta::new_readonly(amm_event_auth, false),
                AccountMeta::new_readonly(amm_id(), false),
            ],
            data: metadao::CRANK_THAT_TWAP.to_vec(),
        };
        ctx.send_many(&cu(ix_crank), &[])
            .expect("crank_that_twap failed");
    }

    amm
}

fn seed_ai_claim(ctx: &mut TestCtx, oracle: Pubkey, proposer: Pubkey, option: u8) -> Pubkey {
    let (claim, bump) = Pubkey::find_program_address(
        &[b"claim", oracle.as_ref(), proposer.as_ref()],
        &ctx.program_id,
    );
    let mut c: AiClaim = bytemuck::Zeroable::zeroed();
    c.account_type = AccountType::AiClaim.as_u8();
    c.oracle = oracle.to_bytes();
    c.proposer = proposer.to_bytes();
    c.option = option;
    c.challenged = 0;
    c.bump = bump;
    ctx.seed_program_account_at(claim, bytemuck::bytes_of(&c).to_vec());
    claim
}

#[allow(clippy::too_many_arguments)]
fn open_challenge_ix(
    ctx: &TestCtx,
    oracle: Pubkey,
    ai_claim: Pubkey,
    proposer: Pubkey,
    market: Pubkey,
    challenger: Pubkey,
    m: &MarketAccounts,
    pass_amm: Pubkey,
    fail_amm: Pubkey,
    stake_vault: Pubkey,
    oracle_pass_kass: Pubkey,
    oracle_fail_kass: Pubkey,
    kass_dao: Pubkey,
    challenger_usdc_src: Pubkey,
    nonce: u64,
) -> Instruction {
    let (cv_event_auth, _) =
        Pubkey::find_program_address(&metadao::event_authority_seeds(), &vault_id());
    let (protocol, _) = TestCtx::protocol_pda(&ctx.program_id);
    let (escrow_vault, _) = TestCtx::challenge_usdc_vault_pda(&ctx.program_id, &market);
    let mut data = vec![Ix::OpenChallenge as u8];
    data.extend_from_slice(&nonce.to_le_bytes());
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(ai_claim, false),
            AccountMeta::new(proposer, false),
            AccountMeta::new(market, false),
            AccountMeta::new(challenger, true),
            AccountMeta::new_readonly(m.question, false),
            AccountMeta::new(m.kass_vault, false),
            AccountMeta::new_readonly(m.usdc_vault, false),
            AccountMeta::new_readonly(pass_amm, false),
            AccountMeta::new_readonly(fail_amm, false),
            AccountMeta::new(stake_vault, false),
            AccountMeta::new(m.kass_vault_underlying, false),
            AccountMeta::new(m.pass_mint, false),
            AccountMeta::new(m.fail_mint, false),
            AccountMeta::new(oracle_pass_kass, false),
            AccountMeta::new(oracle_fail_kass, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(solana_sdk::system_program::ID, false),
            AccountMeta::new_readonly(cv_event_auth, false),
            AccountMeta::new_readonly(protocol, false),
            AccountMeta::new_readonly(kass_dao, false),
            AccountMeta::new_readonly(ctx.usdc_mint, false),
            AccountMeta::new(challenger_usdc_src, false),
            AccountMeta::new(escrow_vault, false),
        ],
        data,
    }
}

#[allow(clippy::too_many_arguments)]
fn settle_ix(
    ctx: &TestCtx,
    oracle: Pubkey,
    market: Pubkey,
    ai_claim: Pubkey,
    proposer: Pubkey,
    question: Pubkey,
    pass_amm: Pubkey,
    fail_amm: Pubkey,
    nonce: u64,
) -> Instruction {
    let (cv_event_auth, _) =
        Pubkey::find_program_address(&metadao::event_authority_seeds(), &vault_id());
    let mut data = vec![Ix::SettleChallenge as u8];
    data.extend_from_slice(&nonce.to_le_bytes());
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(market, false),
            AccountMeta::new_readonly(ai_claim, false),
            AccountMeta::new(proposer, false),
            AccountMeta::new(question, false),
            AccountMeta::new_readonly(pass_amm, false),
            AccountMeta::new_readonly(fail_amm, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new_readonly(cv_event_auth, false),
        ],
        data,
    }
}

/// Read a little-endian `u32` from a (host-side) account-data slice.
fn read_u32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(data[off..off + 4].try_into().unwrap())
}

/// Question payout numerators (`[n0, n1]`) + denominator, decoded host-side.
/// Layout: disc[8] question_id[32]@8 oracle@40 payout_numerators(Vec<u32>) len@72
/// vals@76,@80 payout_denominator@84.
fn question_resolution(ctx: &TestCtx, question: Pubkey) -> (u32, u32, u32) {
    let acc = ctx.svm.get_account(&question).expect("question missing");
    (
        read_u32(&acc.data, 76),
        read_u32(&acc.data, 80),
        read_u32(&acc.data, 84),
    )
}

/// Physical KASS-conservation invariant (design §9 #3) for the split path:
/// the bond that left `stake_vault` is exactly what is now escrowed in the
/// MetaDAO KASS conditional vault's underlying token account, and
/// `total_oracle_stake` stays the conserved accumulator (it is intentionally NOT
/// decremented by the split — the KASS is still in-system, just escrowed). The
/// challenge split is the ONLY path where KASS physically leaves `stake_vault`,
/// so this asserts nothing was created or destroyed there.
fn assert_kass_conserved(ctx: &TestCtx, oracle: Pubkey, kass_vault_underlying: Pubkey) {
    let stake_vault = ctx.seeded(oracle).stake_vault;
    let total = ctx.oracle(oracle).total_oracle_stake;
    assert_eq!(
        ctx.token_balance(stake_vault) + ctx.token_balance(kass_vault_underlying),
        total,
        "physical KASS conservation: stake_vault + conditional-vault underlying == total_oracle_stake",
    );
}

const BOND: u64 = 1_000_000_000;
/// Base reserve common to all pools: 100 KASS (9 dp).
const BASE_RESERVE: u64 = 100_000_000_000;
/// Quote reserve giving price 1e9 (100 USDC, 6 dp). add_liquidity needs ≥ 1e8.
const QUOTE_LOW: u64 = 100_000_000;
/// Quote reserve giving price 3e9 (300 USDC) — fail far above pass+threshold.
const QUOTE_HIGH: u64 = 300_000_000;

struct Fixture {
    oracle: Pubkey,
    nonce: u64,
    proposer: Pubkey,
    proposer_other: Pubkey,
    ai_claim: Pubkey,
    market: Pubkey,
    m: MarketAccounts,
    pass_amm: Pubkey,
    fail_amm: Pubkey,
}

/// What (if anything) to corrupt about the recorded AMMs, for the attack tests.
#[derive(Clone, Copy, PartialEq)]
enum AmmAttack {
    /// Record the canonical real pass/fail pools (honest path).
    None,
    /// Record a rogue AMM-owned account (wrong mints) as pass_amm.
    RoguePass,
    /// Record the SAME real pool as both pass_amm and fail_amm.
    AliasPassAsFail,
    /// Real pools, but the PASS pool is never cranked (pass_twap == 0). Used to
    /// prove an un-cranked pass side makes the claim survive regardless of fail.
    PassUncranked,
}

/// Full fixture: disputed oracle in Challenge, one challenged proposer with a
/// composed market and REAL pass/fail AMMs at the requested quote reserves, then
/// `open_challenge` run so the Market records everything. Leaves `now` unchanged
/// (only slots advanced), so the caller controls crossing `twap_end`.
fn fixture(pass_quote: u64, fail_quote: u64) -> (TestCtx, Fixture) {
    fixture_with_attack(pass_quote, fail_quote, AmmAttack::None)
}

fn fixture_with_attack(pass_quote: u64, fail_quote: u64, attack: AmmAttack) -> (TestCtx, Fixture) {
    let mut ctx = TestCtx::new();
    ctx.svm.add_program(vault_id(), VAULT_SO);
    ctx.svm.add_program(amm_id(), AMM_SO);

    // Protocol + governance with a deterministic kass_price so open_challenge
    // can size + escrow the challenger USDC.
    let kass_dao = ctx.bless_kass_price();

    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: BOND,
        },
        ProposerSpec {
            option: 1,
            bond: BOND,
        },
    ]);
    let seeded = ctx.seeded(oracle);
    let nonce = seeded.nonce;
    let stake_vault = seeded.stake_vault;
    let proposer = seeded.proposers[0].pda;
    let proposer_other = seeded.proposers[1].pda;

    ctx.set_phase(oracle, Phase::Challenge);
    let ai_claim = seed_ai_claim(&mut ctx, oracle, proposer, 0);

    let (m, oracle_pass_kass, oracle_fail_kass) = setup_market(&mut ctx, oracle);

    // Real pass/fail AMMs over the conditional (KASS, USDC) mint pairs. The PASS
    // pool is left un-cranked only for the PassUncranked case.
    let crank_pass = attack != AmmAttack::PassUncranked;
    let real_pass = build_amm(
        &mut ctx,
        m.pass_mint,
        m.pass_usdc,
        BASE_RESERVE,
        pass_quote,
        crank_pass,
    );
    let real_fail = build_amm(
        &mut ctx,
        m.fail_mint,
        m.fail_usdc,
        BASE_RESERVE,
        fail_quote,
        true,
    );
    let (pass_amm, fail_amm) = match attack {
        AmmAttack::None | AmmAttack::PassUncranked => (real_pass, real_fail),
        AmmAttack::RoguePass => (
            fabricate_rogue_amm(&mut ctx, Pubkey::new_unique(), Pubkey::new_unique()),
            real_fail,
        ),
        // Record the SAME pool for both sides (open_challenge does not forbid it;
        // settle must, via its pass != fail guard).
        AmmAttack::AliasPassAsFail => (real_pass, real_pass),
    };

    let (market, _) =
        Pubkey::find_program_address(&[b"market", ai_claim.as_ref()], &ctx.program_id);

    let challenger = Keypair::new();
    ctx.svm
        .airdrop(&challenger.pubkey(), 1_000_000_000)
        .unwrap();
    let challenger_usdc_src = ctx.fund_usdc(&challenger, 5_000_000);

    let ix = open_challenge_ix(
        &ctx,
        oracle,
        ai_claim,
        proposer,
        market,
        challenger.pubkey(),
        &m,
        pass_amm,
        fail_amm,
        stake_vault,
        oracle_pass_kass,
        oracle_fail_kass,
        kass_dao,
        challenger_usdc_src,
        nonce,
    );
    ctx.send_many(&cu(ix), &[&challenger])
        .expect("open_challenge should succeed");

    (
        ctx,
        Fixture {
            oracle,
            nonce,
            proposer,
            proposer_other,
            ai_claim,
            market,
            m,
            pass_amm,
            fail_amm,
        },
    )
}

#[test]
fn settle_fraud_disqualifies_and_resolves_fail_side() {
    // fail TWAP (price 3e9) >> pass TWAP (1e9) * 1.1 → fraud → disqualify.
    let (mut ctx, f) = fixture(QUOTE_LOW, QUOTE_HIGH);
    let bond_pool_before = ctx.oracle(f.oracle).bond_pool;
    let surviving_before = ctx.oracle(f.oracle).surviving_count;
    // open_challenge bumped the open-market counter 0 → 1.
    assert_eq!(ctx.oracle(f.oracle).open_challenge_count, 1);
    // Physical KASS conservation across the split path holds right after open.
    assert_kass_conserved(&ctx, f.oracle, f.m.kass_vault_underlying);

    ctx.warp(TWAP_WINDOW + 1); // cross market.twap_end

    let ix = settle_ix(
        &ctx,
        f.oracle,
        f.market,
        f.ai_claim,
        f.proposer,
        f.m.question,
        f.pass_amm,
        f.fail_amm,
        f.nonce,
    );
    ctx.send_many(&cu(ix), &[]).expect("settle should succeed");

    let p = ctx.proposer(f.proposer);
    assert_eq!(p.disqualified, 1, "fraud proposer disqualified");
    assert_eq!(p.slashed, 1);
    assert_eq!(p.slashed_amount, BOND, "full bond forfeit");

    let o = ctx.oracle(f.oracle);
    assert_eq!(o.surviving_count, surviving_before - 1);
    assert_eq!(o.bond_pool, bond_pool_before + BOND);
    assert_eq!(o.phase, Phase::Challenge as u8, "phase stays Challenge");
    // settle decremented the open-market counter 1 → 0.
    assert_eq!(
        o.open_challenge_count, 0,
        "challenge settled, counter back to 0"
    );

    let market: Market = ctx.read_pod(f.market);
    assert_eq!(market.settled, 1);

    // Question resolved FAIL-side [0,1], denominator 1.
    let (n0, n1, denom) = question_resolution(&ctx, f.m.question);
    assert_eq!((n0, n1, denom), (0, 1, 1), "fail-side resolution");

    // The other proposer is untouched.
    assert_eq!(ctx.proposer(f.proposer_other).disqualified, 0);

    // Conservation still holds after settle: the resolution + accounting move no
    // KASS, so stake_vault + conditional-vault underlying == total_oracle_stake.
    assert_kass_conserved(&ctx, f.oracle, f.m.kass_vault_underlying);
}

#[test]
fn settle_honest_survives_and_resolves_pass_side() {
    // pass TWAP == fail TWAP (both 1e9) → within threshold → survives.
    let (mut ctx, f) = fixture(QUOTE_LOW, QUOTE_LOW);
    let bond_pool_before = ctx.oracle(f.oracle).bond_pool;
    let surviving_before = ctx.oracle(f.oracle).surviving_count;
    // Physical KASS conservation across the split path holds right after open.
    assert_kass_conserved(&ctx, f.oracle, f.m.kass_vault_underlying);

    ctx.warp(TWAP_WINDOW + 1);

    let ix = settle_ix(
        &ctx,
        f.oracle,
        f.market,
        f.ai_claim,
        f.proposer,
        f.m.question,
        f.pass_amm,
        f.fail_amm,
        f.nonce,
    );
    ctx.send_many(&cu(ix), &[]).expect("settle should succeed");

    let p = ctx.proposer(f.proposer);
    assert_eq!(p.disqualified, 0, "honest proposer survives");
    assert_eq!(p.slashed, 0);
    assert_eq!(p.slashed_amount, 0);

    let o = ctx.oracle(f.oracle);
    assert_eq!(o.surviving_count, surviving_before, "no slash");
    assert_eq!(o.bond_pool, bond_pool_before);

    assert_eq!(ctx.read_pod::<Market>(f.market).settled, 1);

    // Question resolved PASS-side [1,0].
    let (n0, n1, denom) = question_resolution(&ctx, f.m.question);
    assert_eq!((n0, n1, denom), (1, 0, 1), "pass-side resolution");

    // Conservation still holds after a survive-side settle (no KASS moved).
    assert_kass_conserved(&ctx, f.oracle, f.m.kass_vault_underlying);
}

#[test]
fn settle_before_twap_end_fails() {
    let (mut ctx, f) = fixture(QUOTE_LOW, QUOTE_HIGH);
    // Do NOT warp past twap_end.
    let ix = settle_ix(
        &ctx,
        f.oracle,
        f.market,
        f.ai_claim,
        f.proposer,
        f.m.question,
        f.pass_amm,
        f.fail_amm,
        f.nonce,
    );
    let err = ctx.send_many(&cu(ix), &[]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(KassandraError::TwapWindowOpen as u32),
        ),
    );
}

#[test]
fn settle_twice_is_already_settled() {
    let (mut ctx, f) = fixture(QUOTE_LOW, QUOTE_HIGH);
    ctx.warp(TWAP_WINDOW + 1);

    let ix = settle_ix(
        &ctx,
        f.oracle,
        f.market,
        f.ai_claim,
        f.proposer,
        f.m.question,
        f.pass_amm,
        f.fail_amm,
        f.nonce,
    );
    ctx.send_many(&cu(ix.clone()), &[])
        .expect("first settle ok");

    let err = ctx.send_many(&cu(ix), &[]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(KassandraError::AlreadySettled as u32),
        ),
    );
}

#[test]
fn settle_with_unbound_amm_fails() {
    // REQUIRED binding-attack test: the recorded pass_amm is a rogue AMM-owned
    // account whose base/quote mints are NOT this market's conditional mints
    // (e.g. a pool the challenger controls). open_challenge records it (it only
    // checks owner == AMM_ID); settle MUST reject it on the hard mint binding.
    let (mut ctx, f) = fixture_with_attack(QUOTE_LOW, QUOTE_HIGH, AmmAttack::RoguePass);
    ctx.warp(TWAP_WINDOW + 1);

    let ix = settle_ix(
        &ctx,
        f.oracle,
        f.market,
        f.ai_claim,
        f.proposer,
        f.m.question,
        f.pass_amm,
        f.fail_amm,
        f.nonce,
    );
    let err = ctx.send_many(&cu(ix), &[]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(KassandraError::InvalidAccount as u32),
        ),
        "settle must reject an AMM not bound to this market's conditional mints",
    );
}

#[test]
fn settle_with_aliased_amms_fails() {
    // pass_amm == fail_amm must be rejected (a challenger cannot collapse the two
    // pools into one they steer). open_challenge recorded the SAME pool for both
    // sides; settle's `pass_amm != fail_amm` guard rejects it.
    let (mut ctx, f) = fixture_with_attack(QUOTE_LOW, QUOTE_HIGH, AmmAttack::AliasPassAsFail);
    assert_eq!(f.pass_amm, f.fail_amm, "fixture recorded aliased pools");
    ctx.warp(TWAP_WINDOW + 1);

    let ix = settle_ix(
        &ctx,
        f.oracle,
        f.market,
        f.ai_claim,
        f.proposer,
        f.m.question,
        f.pass_amm,
        f.fail_amm,
        f.nonce,
    );
    let err = ctx.send_many(&cu(ix), &[]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(KassandraError::InvalidAccount as u32),
        ),
    );
}

#[test]
fn settle_last_block_swap_does_not_flip_outcome() {
    // TWAP manipulation resistance: an honest market (pass == fail == 1e9) is
    // cranked over the window. A single large last-moment BUY on the fail pool
    // swings its INSTANTANEOUS price far above threshold, but the AMM only
    // records a new observation once per ONE_MINUTE_IN_SLOTS, so within the same
    // minute the stored slot-weighted TWAP is unchanged → the claim still
    // SURVIVES. This is exactly the time-weighting the design relies on.
    let (mut ctx, f) = fixture(QUOTE_LOW, QUOTE_LOW);

    // Large BUY on the fail pool (quote in, base out) — drives fail price up.
    // Fund the payer with fail-USDC and a fail-KASS receive account.
    let payer = ctx.payer.pubkey();
    let user_base = ata(&payer, &f.m.fail_mint);
    let user_quote = ata(&payer, &f.m.fail_usdc);
    // (user_base/user_quote already exist from build_amm; top up quote.)
    fabricate_token_account(&mut ctx, user_quote, f.m.fail_usdc, payer, 10_000_000_000);
    fabricate_token_account(&mut ctx, user_base, f.m.fail_mint, payer, 0);
    let vault_ata_base = ata(&f.fail_amm, &f.m.fail_mint);
    let vault_ata_quote = ata(&f.fail_amm, &f.m.fail_usdc);
    let (amm_event_auth, _) =
        Pubkey::find_program_address(&metadao::event_authority_seeds(), &amm_id());

    // SwapArgs { swap_type: Buy(0), input_amount, output_amount_min: 0 }.
    let mut swap_data = metadao::SWAP.to_vec();
    swap_data.push(0u8); // SwapType::Buy
    swap_data.extend_from_slice(&80_000_000u64.to_le_bytes()); // 80 USDC in
    swap_data.extend_from_slice(&0u64.to_le_bytes());
    let ix_swap = Instruction {
        program_id: amm_id(),
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(f.fail_amm, false),
            AccountMeta::new(user_base, false),
            AccountMeta::new(user_quote, false),
            AccountMeta::new(vault_ata_base, false),
            AccountMeta::new(vault_ata_quote, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(amm_event_auth, false),
            AccountMeta::new_readonly(amm_id(), false),
        ],
        data: swap_data,
    };
    // Only a few slots after the crank (< 150), so update_twap records nothing.
    ctx.warp_slots(0, 5);
    ctx.send_many(&cu(ix_swap), &[])
        .expect("swap should succeed");

    ctx.warp(TWAP_WINDOW + 1);
    let ix = settle_ix(
        &ctx,
        f.oracle,
        f.market,
        f.ai_claim,
        f.proposer,
        f.m.question,
        f.pass_amm,
        f.fail_amm,
        f.nonce,
    );
    ctx.send_many(&cu(ix), &[]).expect("settle should succeed");

    // Despite the huge last-moment swing, the TWAP didn't move → survives.
    assert_eq!(
        ctx.proposer(f.proposer).disqualified,
        0,
        "TWAP resists last-block swap"
    );
    let (n0, n1, _) = question_resolution(&ctx, f.m.question);
    assert_eq!((n0, n1), (1, 0), "pass-side: claim survived manipulation");
}

// NOTE on a stronger "crank-folded spike dilutes over the window" test (review
// item 6): the precise diluted average depends on the exact slot deltas between
// the honest cranks and the post-manipulation crank, plus the v0.4.2
// `max_observation_change_per_update` clamp. Making the pass/fail margin land
// deterministically on the survive side requires choreographing an honest window
// many multiples of `ONE_MINUTE_IN_SLOTS` long against a single 150-slot spike
// window — brittle against LiteSVM's slot accounting. The once-per-minute
// observation gate (the realistic last-block attack) is covered deterministically
// by `settle_last_block_swap_does_not_flip_outcome` above; the longer-window
// dilution is a direct consequence of settle dividing the aggregator by the FULL
// elapsed window in `verify_and_read_twap`, exercised by the honest/fraud tests.

#[test]
fn settle_uncranked_pass_pool_survives() {
    // pass pool is NEVER cranked (pass_twap == 0) while the fail pool is cranked
    // to a high price. A pass_twap of 0 means NO counter-trading on the pass side
    // (design §7 → survive), so even with fail far above threshold the claim must
    // SURVIVE — otherwise a challenger could crank only the fail pool to cheaply
    // disqualify an honest proposer.
    let (mut ctx, f) = fixture_with_attack(QUOTE_LOW, QUOTE_HIGH, AmmAttack::PassUncranked);
    let surviving_before = ctx.oracle(f.oracle).surviving_count;
    ctx.warp(TWAP_WINDOW + 1);

    let ix = settle_ix(
        &ctx,
        f.oracle,
        f.market,
        f.ai_claim,
        f.proposer,
        f.m.question,
        f.pass_amm,
        f.fail_amm,
        f.nonce,
    );
    ctx.send_many(&cu(ix), &[]).expect("settle should succeed");

    let p = ctx.proposer(f.proposer);
    assert_eq!(
        p.disqualified, 0,
        "pass_twap==0 must survive, not disqualify"
    );
    assert_eq!(p.slashed, 0);
    let o = ctx.oracle(f.oracle);
    assert_eq!(o.surviving_count, surviving_before, "no slash");
    assert_eq!(o.open_challenge_count, 0);
    assert_eq!(ctx.read_pod::<Market>(f.market).settled, 1);
    let (n0, n1, _) = question_resolution(&ctx, f.m.question);
    assert_eq!((n0, n1), (1, 0), "pass-side resolution");
}

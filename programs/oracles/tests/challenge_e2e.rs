//! End-to-end CHALLENGE lifecycle (Task C3).
//!
//! Drives the FULL challenge market against the REAL deployed MetaDAO v0.4
//! `amm` + `conditional_vault` binaries in LiteSVM, for BOTH outcomes
//! (fraud → disqualified, honest → survives), and asserts the physical
//! settlement + directional fees + KASS/USDC conservation against an
//! INDEPENDENT reference computation ([`ConservationModel`]) derived from the
//! bond + the governable fee config alone (it never trusts the program's own
//! accounting).
//!
//! # What is REAL vs SEEDED (honest split)
//! * **Dispute core (front door) — REAL.** [`front_door_to_challenge`] drives
//!   `create_oracle → propose×2 (conflict) → finalize_proposals → submit_fact →
//!   advance_phase → vote_fact → finalize_facts → submit_ai_claim×2 →
//!   finalize_ai_claims` through the genuine instructions to land the oracle in
//!   [`Phase::Challenge`] with a real [`AiClaim`] for a surviving, UN-slashed
//!   proposer (option-0 proposer claims option 0 → no flip). No `set_phase`
//!   shortcut is used in the e2e tests; only `warp`/`warp_slots` advance time.
//! * **MetaDAO market — REAL.** The challenger composes the binary question +
//!   KASS/USDC conditional vaults + pass/fail AMMs via real CPIs (exactly how a
//!   real challenger composes the market off-chain), then `open_challenge`
//!   verifies + records them, escrows the challenger USDC, and program-signs the
//!   bond split — all real instructions.
//! * **TWAP — REAL, swap-driven on the fraud path.** The fraud test pushes the
//!   FAIL pool's price up with a genuine `swap` (BUY) and accumulates it into the
//!   slot-weighted TWAP across TWO `crank_that_twap` calls 300 slots apart, so
//!   the disqualify decision is driven by real trading moving the TWAP past the
//!   `pass + threshold` margin (not by a fabricated price). The honest test
//!   leaves both pools at their seeded neutral price (pass == fail → survives).
//! * **`settle_challenge` — REAL.** `resolve_question` + `redeem_tokens` +
//!   directional-fee transfers are all program-signed real CPIs.
//!
//! The conservation FUZZ arm at the bottom uses a FABRICATED AMM account with a
//! chosen aggregator (a stubbed/known TWAP) so it can cheaply sweep both
//! outcomes × fee rates × bond sizes against [`ConservationModel`] while still
//! driving the REAL `open_challenge` (split + escrow) and `settle_challenge`
//! (redeem + fees) — the real-AMM *TWAP-production* path is covered by the two
//! deterministic e2e tests above + `settle_challenge.rs`. See the module-level
//! note at the bottom for why the heavy real-AMM path is not itself fuzzed.

mod common;
use common::*;

use kassandra_oracles_program::{
    config::{
        CHALLENGE_FAIL_USDC_FEE_DEN, CHALLENGE_FAIL_USDC_FEE_NUM, CHALLENGE_SUCCESS_KASS_FEE_DEN,
        CHALLENGE_SUCCESS_KASS_FEE_NUM, MARKET_THRESHOLD_DEN, MARKET_THRESHOLD_NUM, PHASE_WINDOW,
    },
    cpi::metadao,
    instruction::Ix,
    state::{AccountType, AiClaim, Market, Phase, VOTE_APPROVE},
};
use proptest::prelude::*;
use solana_account::Account;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_sdk_ids::system_program;
use solana_signer::Signer;
use spl_token::{
    solana_program::{program_option::COption, program_pack::Pack},
    state::{Account as TokenAccount, AccountState},
    ID as TOKEN_PROGRAM_ID,
};

const VAULT_SO: &[u8] = include_bytes!("fixtures/metadao_conditional_vault.so");
const AMM_SO: &[u8] = include_bytes!("fixtures/metadao_amm.so");
const ATA_PROGRAM_ID: Pubkey =
    solana_pubkey::pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

/// Largest observation change per crank — lets a single crank fold the pool's
/// current price straight into the TWAP (no per-update clamp), so test TWAPs are
/// deterministic.
const MAX_PRICE: u128 = (u64::MAX as u128) * 1_000_000_000_000;

const BOND: u64 = 1_000_000_000; // 1 KASS bond on the challenged proposer.
/// Base reserve: 100 KASS (9 dp).
const BASE_RESERVE: u64 = 100_000_000_000;
/// Quote reserve: 100 USDC (6 dp) → seeded price 1e9 (scaled). add_liquidity
/// needs the quote ≥ 1e8.
const QUOTE_NEUTRAL: u64 = 100_000_000;

fn vault_id() -> Pubkey {
    Pubkey::new_from_array(metadao::CONDITIONAL_VAULT_ID.to_bytes())
}
fn amm_id() -> Pubkey {
    Pubkey::new_from_array(metadao::AMM_ID.to_bytes())
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
        &metadao::conditional_token_mint_seeds(&vault.to_bytes().into(), &[index]),
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

// ---------------------------------------------------------------------------
// Independent conservation reference (never reads the program's accounting)
// ---------------------------------------------------------------------------

/// Predicts, from `bond` + escrow + the governable fee config alone, every
/// post-settle token delta the two conservation equations rest on.
#[derive(Clone, Copy, Debug)]
struct ConservationModel {
    disqualify: bool,
    bond: u64,
    escrow: u64,
    /// KASS fee → challenger on a successful challenge (`bond × succ_num/den`),
    /// capped at the proposer's remaining un-slashed bond. The e2e/fuzz proposer
    /// is challenged UN-slashed, so the cap is a no-op (`prior_slash == 0`).
    kass_fee: u64,
    /// USDC fee → proposer on a failed challenge (`escrow × fail_num/den`).
    usdc_fee: u64,
}

impl ConservationModel {
    #[allow(clippy::too_many_arguments)]
    fn compute(
        disqualify: bool,
        bond: u64,
        escrow: u64,
        prior_slash: u64,
        succ_num: u64,
        succ_den: u64,
        fail_num: u64,
        fail_den: u64,
    ) -> Self {
        let raw_kass_fee = (bond as u128 * succ_num as u128 / succ_den as u128) as u64;
        let kass_fee = raw_kass_fee.min(bond - prior_slash);
        let usdc_fee = (escrow as u128 * fail_num as u128 / fail_den as u128) as u64;
        Self {
            disqualify,
            bond,
            escrow,
            kass_fee,
            usdc_fee,
        }
    }

    /// Expected `challenger_kass` balance after settle.
    fn challenger_kass(&self) -> u64 {
        if self.disqualify {
            self.kass_fee
        } else {
            0
        }
    }
    /// Expected stake_vault DELTA across settle (redeem in, fee out).
    fn stake_vault_delta(&self) -> u64 {
        if self.disqualify {
            self.bond - self.kass_fee
        } else {
            self.bond
        }
    }
    /// Expected `proposer_usdc` balance after settle.
    fn proposer_usdc(&self) -> u64 {
        if self.disqualify {
            0
        } else {
            self.usdc_fee
        }
    }
    /// Expected `challenger_usdc_dest` balance after settle.
    fn challenger_usdc(&self) -> u64 {
        if self.disqualify {
            self.escrow
        } else {
            self.escrow - self.usdc_fee
        }
    }
}

/// Independent slash decision (a fresh copy of the on-chain rule): disqualify iff
/// `pass_twap > 0` AND `fail_twap * DEN > pass_twap * (DEN + NUM)`.
fn ref_disqualify(pass_twap: u128, fail_twap: u128) -> bool {
    if pass_twap == 0 {
        return false;
    }
    fail_twap * MARKET_THRESHOLD_DEN > pass_twap * (MARKET_THRESHOLD_DEN + MARKET_THRESHOLD_NUM)
}

// ---------------------------------------------------------------------------
// Dispute-core instruction builders (mirror lifecycle_e2e.rs)
// ---------------------------------------------------------------------------

fn finalize_facts_ix(ctx: &TestCtx, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
    ctx.finalize_facts_ix(oracle, tail)
}

fn claim_pda(program_id: &Pubkey, oracle: &Pubkey, proposer: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"claim", oracle.as_ref(), proposer.as_ref()], program_id)
}

/// What the front door hands back: a real oracle sitting in `Challenge` with a
/// real `AiClaim` for an un-slashed, surviving proposer ready to be challenged.
struct Challenged {
    oracle: Pubkey,
    nonce: u64,
    stake_vault: Pubkey,
    proposer: Pubkey,
    proposer_authority: Pubkey,
    ai_claim: Pubkey,
}

/// Drive the REAL dispute core to `Phase::Challenge` (see module header). The
/// returned proposer is the option-0 proposer, who claims option 0 (no flip), so
/// it is surviving with `slashed_amount == 0` — a clean bond to challenge.
fn front_door_to_challenge(ctx: &mut TestCtx) -> Challenged {
    // create_oracle → propose×2 (conflict) → finalize_proposals => FactProposal.
    let oracle = ctx.dispute_via_real_flow(&[
        ProposerSpec {
            option: 0,
            bond: BOND,
        },
        ProposerSpec {
            option: 1,
            bond: BOND,
        },
    ]);
    let (vault, _) = TestCtx::stake_vault_pda(&ctx.program_id, &oracle);
    let nonce = ctx.seeded(oracle).nonce;
    let proposer_pdas: Vec<Pubkey> = ctx.proposers(oracle).iter().map(|p| p.pda).collect();
    let authorities: Vec<Keypair> = ctx
        .proposers(oracle)
        .iter()
        .map(|p| p.authority.insecure_clone())
        .collect();

    // submit_fact (FactProposal still open).
    let submitter = Keypair::new();
    ctx.svm.airdrop(&submitter.pubkey(), 1_000_000_000).unwrap();
    let submitter_kass = ctx.fund_kass(&submitter, 1_000_000);
    let content_hash = [0x07u8; 32];
    let (fact, _) = TestCtx::fact_pda(&ctx.program_id, &oracle, &content_hash);
    ctx.send(
        submit_fact_ix(
            ctx,
            oracle,
            fact,
            submitter.pubkey(),
            submitter_kass,
            vault,
            submit_fact_payload(&content_hash, 100, b"ipfs://fact"),
        ),
        &[&submitter],
    )
    .expect("submit_fact");

    // warp past FactProposal, advance_phase => FactVoting.
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(advance_phase_ix(ctx, oracle), &[])
        .expect("advance_phase");

    // vote approve well past the 2/3 quorum of dispute_bond_total (== 2*BOND).
    let voter = Keypair::new();
    ctx.svm.airdrop(&voter.pubkey(), 1_000_000_000).unwrap();
    let voter_kass = ctx.fund_kass(&voter, 2 * BOND);
    let (fact_vote, _) = TestCtx::vote_pda(&ctx.program_id, &fact, &voter.pubkey());
    ctx.send(
        vote_fact_ix(
            ctx,
            oracle,
            fact,
            fact_vote,
            voter.pubkey(),
            voter_kass,
            vault,
            vote_payload(VOTE_APPROVE, 2 * BOND),
        ),
        &[&voter],
    )
    .expect("vote_fact");

    // warp past voting, finalize_facts => AiClaim.
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(finalize_facts_ix(ctx, oracle, &[fact]), &[])
        .expect("finalize_facts");
    assert_eq!(ctx.oracle(oracle).phase, Phase::AiClaim.as_u8());

    // Both proposers claim option 0: proposer[0] (orig 0) does NOT flip (survives
    // un-slashed); proposer[1] (orig 1) flips (partial slash, still surviving).
    for (auth, pda) in authorities.iter().zip(&proposer_pdas) {
        ctx.svm.airdrop(&auth.pubkey(), 1_000_000_000).unwrap();
        let (claim, _) = claim_pda(&ctx.program_id, &oracle, pda);
        ctx.send(
            submit_ai_claim_ix(
                ctx,
                oracle,
                *pda,
                claim,
                auth.pubkey(),
                submit_ai_payload(0),
            ),
            &[auth],
        )
        .expect("submit_ai_claim");
    }

    // warp past AiClaim, finalize_ai_claims => Challenge.
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(finalize_ai_claims_ix(ctx, oracle, &proposer_pdas), &[])
        .expect("finalize_ai_claims");
    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::Challenge.as_u8());
    assert_eq!(
        o.surviving_count, 2,
        "both proposers survive into Challenge"
    );

    let proposer = proposer_pdas[0];
    let proposer_authority = authorities[0].pubkey();
    assert_eq!(
        ctx.proposer(proposer).slashed_amount,
        0,
        "the challenged (option-0) proposer is un-slashed"
    );
    let (ai_claim, _) = claim_pda(&ctx.program_id, &oracle, &proposer);

    Challenged {
        oracle,
        nonce,
        stake_vault: vault,
        proposer,
        proposer_authority,
        ai_claim,
    }
}

// ---------------------------------------------------------------------------
// MetaDAO market composition (mirror settle_challenge.rs)
// ---------------------------------------------------------------------------

struct MarketAccounts {
    question: Pubkey,
    kass_vault: Pubkey,
    kass_vault_underlying: Pubkey,
    usdc_vault: Pubkey,
    pass_mint: Pubkey,
    fail_mint: Pubkey,
    pass_usdc: Pubkey,
    fail_usdc: Pubkey,
}

/// Compose the binary question + KASS/USDC conditional vaults for `resolver`,
/// plus the oracle-PDA-owned pass/fail conditional-KASS holders.
fn setup_market(ctx: &mut TestCtx, resolver: Pubkey) -> (MarketAccounts, Pubkey, Pubkey) {
    let kass = ctx.kass_mint;
    let usdc = ctx.usdc_mint;
    let resolver_arr = resolver.to_bytes();
    let kass_arr = kass.to_bytes();
    let usdc_arr = usdc.to_bytes();
    let num_outcomes: u8 = 2;
    let question_id = [7u8; 32];

    let (question, _) = Pubkey::find_program_address(
        &metadao::question_seeds(&question_id, &resolver_arr.into(), &[num_outcomes]),
        &vault_id(),
    );
    let question_arr = question.to_bytes();
    let (kass_vault, _) = Pubkey::find_program_address(
        &metadao::vault_seeds(&question_arr.into(), &kass_arr.into()),
        &vault_id(),
    );
    let (usdc_vault, _) = Pubkey::find_program_address(
        &metadao::vault_seeds(&question_arr.into(), &usdc_arr.into()),
        &vault_id(),
    );

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
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(vault_id(), false),
        ],
        data: metadao::initialize_question_data(&question_id, &resolver_arr.into(), num_outcomes)
            .to_vec(),
    };
    ctx.send_many(&cu(ix_q), &[]).expect("initialize_question");

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
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new(pass_mint, false),
            AccountMeta::new(fail_mint, false),
        ],
        data: metadao::initialize_conditional_vault_data().to_vec(),
    };
    ctx.send_many(&cu(ix_kv), &[]).expect("init KASS vault");

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
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new(pass_usdc, false),
            AccountMeta::new(fail_usdc, false),
        ],
        data: metadao::initialize_conditional_vault_data().to_vec(),
    };
    ctx.send_many(&cu(ix_uv), &[]).expect("init USDC vault");

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

// ---------------------------------------------------------------------------
// Real AMM pool driving (create + add, then swap / crank separately)
// ---------------------------------------------------------------------------

/// `create_amm` + `add_liquidity` (NO crank yet). Returns the AMM PDA; funds the
/// payer's base/quote generously (4× reserve) so later swaps have headroom.
fn build_pool(
    ctx: &mut TestCtx,
    base_mint: Pubkey,
    quote_mint: Pubkey,
    base_reserve: u64,
    quote_reserve: u64,
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

    let user_base = ata(&payer, &base_mint);
    let user_quote = ata(&payer, &quote_mint);
    fabricate_token_account(
        ctx,
        user_base,
        base_mint,
        payer,
        base_reserve.saturating_mul(4),
    );
    fabricate_token_account(
        ctx,
        user_quote,
        quote_mint,
        payer,
        quote_reserve.saturating_mul(4),
    );

    let initial_obs: u128 = (quote_reserve as u128 * 1_000_000_000_000u128) / base_reserve as u128;
    let mut create_data = metadao::CREATE_AMM.to_vec();
    create_data.extend_from_slice(&initial_obs.to_le_bytes());
    create_data.extend_from_slice(&MAX_PRICE.to_le_bytes());
    create_data.extend_from_slice(&0u64.to_le_bytes());
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
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(amm_event_auth, false),
            AccountMeta::new_readonly(amm_id(), false),
        ],
        data: create_data,
    };
    ctx.send_many(&cu(ix_create), &[]).expect("create_amm");

    let user_lp = ata(&payer, &lp_mint);
    fabricate_token_account(ctx, user_lp, lp_mint, payer, 0);

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
    ctx.send_many(&cu(ix_add), &[]).expect("add_liquidity");
    amm
}

/// A genuine BUY (quote in, base out) that pushes the pool's price UP.
fn swap_buy(ctx: &mut TestCtx, amm: Pubkey, base_mint: Pubkey, quote_mint: Pubkey, amount_in: u64) {
    let payer = ctx.payer.pubkey();
    let user_base = ata(&payer, &base_mint);
    let user_quote = ata(&payer, &quote_mint);
    let vault_ata_base = ata(&amm, &base_mint);
    let vault_ata_quote = ata(&amm, &quote_mint);
    let (amm_event_auth, _) =
        Pubkey::find_program_address(&metadao::event_authority_seeds(), &amm_id());

    let mut swap_data = metadao::SWAP.to_vec();
    swap_data.push(0u8); // SwapType::Buy
    swap_data.extend_from_slice(&amount_in.to_le_bytes());
    swap_data.extend_from_slice(&0u64.to_le_bytes());
    let ix_swap = Instruction {
        program_id: amm_id(),
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(amm, false),
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
    ctx.warp_slots(0, 5);
    ctx.send_many(&cu(ix_swap), &[]).expect("swap buy");
}

/// Advance ≥ ONE_MINUTE_IN_SLOTS (150) slots, then `crank_that_twap` once.
fn crank_pool(ctx: &mut TestCtx, amm: Pubkey) {
    ctx.warp_slots(0, 300);
    let (amm_event_auth, _) =
        Pubkey::find_program_address(&metadao::event_authority_seeds(), &amm_id());
    let ix_crank = Instruction {
        program_id: amm_id(),
        accounts: vec![
            AccountMeta::new(amm, false),
            AccountMeta::new_readonly(amm_event_auth, false),
            AccountMeta::new_readonly(amm_id(), false),
        ],
        data: metadao::CRANK_THAT_TWAP.to_vec(),
    };
    ctx.send_many(&cu(ix_crank), &[]).expect("crank_that_twap");
}

// ---------------------------------------------------------------------------
// open_challenge / settle_challenge instruction builders
// ---------------------------------------------------------------------------

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
            AccountMeta::new_readonly(system_program::ID, false),
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

struct SettleExtras {
    stake_vault: Pubkey,
    kass_vault: Pubkey,
    kass_vault_underlying: Pubkey,
    pass_mint: Pubkey,
    fail_mint: Pubkey,
    oracle_pass_kass: Pubkey,
    oracle_fail_kass: Pubkey,
    escrow_vault: Pubkey,
    proposer_usdc: Pubkey,
    challenger_usdc_dest: Pubkey,
    challenger_kass: Pubkey,
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
    x: &SettleExtras,
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
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new(x.stake_vault, false),
            AccountMeta::new(x.kass_vault, false),
            AccountMeta::new(x.kass_vault_underlying, false),
            AccountMeta::new(x.pass_mint, false),
            AccountMeta::new(x.fail_mint, false),
            AccountMeta::new(x.oracle_pass_kass, false),
            AccountMeta::new(x.oracle_fail_kass, false),
            AccountMeta::new(x.escrow_vault, false),
            AccountMeta::new(x.proposer_usdc, false),
            AccountMeta::new(x.challenger_usdc_dest, false),
            AccountMeta::new(x.challenger_kass, false),
        ],
        data,
    }
}

fn read_u32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(data[off..off + 4].try_into().unwrap())
}

fn question_resolution(ctx: &TestCtx, question: Pubkey) -> (u32, u32, u32) {
    let acc = ctx.svm.get_account(&question).expect("question missing");
    (
        read_u32(&acc.data, 76),
        read_u32(&acc.data, 80),
        read_u32(&acc.data, 84),
    )
}

// ---------------------------------------------------------------------------
// The two deterministic, real-AMM e2e lifecycle tests
// ---------------------------------------------------------------------------

/// Settle-side payout destinations + the escrow vault, fabricated empty.
struct Payouts {
    escrow_vault: Pubkey,
    proposer_usdc: Pubkey,
    challenger_usdc_dest: Pubkey,
    challenger_kass: Pubkey,
}

fn fabricate_payouts(
    ctx: &mut TestCtx,
    market: Pubkey,
    proposer_authority: Pubkey,
    challenger: Pubkey,
) -> Payouts {
    let usdc = ctx.usdc_mint;
    let kass = ctx.kass_mint;
    let proposer_usdc = Pubkey::new_unique();
    let challenger_usdc_dest = Pubkey::new_unique();
    let challenger_kass = Pubkey::new_unique();
    fabricate_token_account(ctx, proposer_usdc, usdc, proposer_authority, 0);
    fabricate_token_account(ctx, challenger_usdc_dest, usdc, challenger, 0);
    fabricate_token_account(ctx, challenger_kass, kass, challenger, 0);
    let (escrow_vault, _) = TestCtx::challenge_usdc_vault_pda(&ctx.program_id, &market);
    Payouts {
        escrow_vault,
        proposer_usdc,
        challenger_usdc_dest,
        challenger_kass,
    }
}

#[test]
fn e2e_honest_full_lifecycle_survives() {
    let mut ctx = TestCtx::new();
    ctx.svm.add_program(vault_id(), VAULT_SO).unwrap();
    ctx.svm.add_program(amm_id(), AMM_SO).unwrap();
    let kass_dao = ctx.bless_kass_price();

    // REAL front door → Challenge with an un-slashed proposer + real AiClaim.
    let c = front_door_to_challenge(&mut ctx);
    let (m, oracle_pass_kass, oracle_fail_kass) = setup_market(&mut ctx, c.oracle);

    // Both pools at the neutral seeded price (1e9) → pass == fail → survives.
    let pass_amm = build_pool(
        &mut ctx,
        m.pass_mint,
        m.pass_usdc,
        BASE_RESERVE,
        QUOTE_NEUTRAL,
    );
    let fail_amm = build_pool(
        &mut ctx,
        m.fail_mint,
        m.fail_usdc,
        BASE_RESERVE,
        QUOTE_NEUTRAL,
    );
    crank_pool(&mut ctx, pass_amm);
    crank_pool(&mut ctx, fail_amm);

    let (market, _) =
        Pubkey::find_program_address(&[b"market", c.ai_claim.as_ref()], &ctx.program_id);
    let challenger = Keypair::new();
    ctx.svm
        .airdrop(&challenger.pubkey(), 1_000_000_000)
        .unwrap();
    let challenger_usdc_src = ctx.fund_usdc(&challenger, 5_000_000);

    // REAL open_challenge: escrow + program-signed bond split.
    let ix = open_challenge_ix(
        &ctx,
        c.oracle,
        c.ai_claim,
        c.proposer,
        market,
        challenger.pubkey(),
        &m,
        pass_amm,
        fail_amm,
        c.stake_vault,
        oracle_pass_kass,
        oracle_fail_kass,
        kass_dao,
        challenger_usdc_src,
        c.nonce,
    );
    ctx.send_many(&cu(ix), &[&challenger])
        .expect("open_challenge");

    let payouts = fabricate_payouts(&mut ctx, market, c.proposer_authority, challenger.pubkey());
    let escrow = required_escrow_usdc(BOND);
    assert_eq!(
        ctx.token_balance(payouts.escrow_vault),
        escrow,
        "escrow funded"
    );

    // Emission is ON by default: the real-flow oracle's stake_vault also holds
    // the creation-time `reward_emission` (untouched until finalize_oracle). The
    // KASS-conservation baseline must therefore include it alongside Σ stakes.
    let total_before =
        ctx.oracle(c.oracle).total_oracle_stake + ctx.oracle(c.oracle).reward_emission;
    let bond_pool_before = ctx.oracle(c.oracle).bond_pool;
    let stake_before = ctx.token_balance(c.stake_vault);

    ctx.warp(TWAP_WINDOW + 1);
    let extras = SettleExtras {
        stake_vault: c.stake_vault,
        kass_vault: m.kass_vault,
        kass_vault_underlying: m.kass_vault_underlying,
        pass_mint: m.pass_mint,
        fail_mint: m.fail_mint,
        oracle_pass_kass,
        oracle_fail_kass,
        escrow_vault: payouts.escrow_vault,
        proposer_usdc: payouts.proposer_usdc,
        challenger_usdc_dest: payouts.challenger_usdc_dest,
        challenger_kass: payouts.challenger_kass,
    };
    let ix = settle_ix(
        &ctx, c.oracle, market, c.ai_claim, c.proposer, m.question, pass_amm, fail_amm, &extras,
        c.nonce,
    );
    ctx.send_many(&cu(ix), &[]).expect("settle_challenge");

    // Independent reference (survive path; proposer un-slashed).
    let model = ConservationModel::compute(
        false,
        BOND,
        escrow,
        0,
        CHALLENGE_SUCCESS_KASS_FEE_NUM,
        CHALLENGE_SUCCESS_KASS_FEE_DEN,
        CHALLENGE_FAIL_USDC_FEE_NUM,
        CHALLENGE_FAIL_USDC_FEE_DEN,
    );

    assert_resolution_and_conservation(
        &ctx,
        c.oracle,
        market,
        c.proposer,
        m.question,
        &extras,
        &model,
        total_before,
        bond_pool_before,
        stake_before,
    );
    // Survive specifics.
    assert_eq!(ctx.proposer(c.proposer).disqualified, 0, "honest survives");
    assert_eq!(ctx.proposer(c.proposer).slashed_amount, 0);
    assert_eq!(ctx.oracle(c.oracle).bond_pool, bond_pool_before, "no slash");
    let (n0, n1, denom) = question_resolution(&ctx, m.question);
    assert_eq!((n0, n1, denom), (1, 0, 1), "pass-side resolution");
}

#[test]
fn e2e_fraud_full_lifecycle_swap_driven_disqualifies() {
    let mut ctx = TestCtx::new();
    ctx.svm.add_program(vault_id(), VAULT_SO).unwrap();
    ctx.svm.add_program(amm_id(), AMM_SO).unwrap();
    let kass_dao = ctx.bless_kass_price();

    let c = front_door_to_challenge(&mut ctx);
    let (m, oracle_pass_kass, oracle_fail_kass) = setup_market(&mut ctx, c.oracle);

    // PASS pool stays neutral (1e9). FAIL pool: a genuine BUY swap pushes its
    // price up, and TWO cranks 300 slots apart accumulate the post-swap price
    // into the slot-weighted TWAP — so the disqualify decision is driven by REAL
    // trading moving the TWAP past `pass + 10% threshold`, not a seeded price.
    let pass_amm = build_pool(
        &mut ctx,
        m.pass_mint,
        m.pass_usdc,
        BASE_RESERVE,
        QUOTE_NEUTRAL,
    );
    let fail_amm = build_pool(
        &mut ctx,
        m.fail_mint,
        m.fail_usdc,
        BASE_RESERVE,
        QUOTE_NEUTRAL,
    );
    crank_pool(&mut ctx, pass_amm);
    // 90 USDC BUY drains the fail pool's base hard → instantaneous price ≈ 3.5e9.
    swap_buy(&mut ctx, fail_amm, m.fail_mint, m.fail_usdc, 90_000_000);
    crank_pool(&mut ctx, fail_amm); // records the post-swap price
    crank_pool(&mut ctx, fail_amm); // accumulates it: TWAP ≈ (1e9 + 3.5e9)/2 ≫ 1.1e9

    let (market, _) =
        Pubkey::find_program_address(&[b"market", c.ai_claim.as_ref()], &ctx.program_id);
    let challenger = Keypair::new();
    ctx.svm
        .airdrop(&challenger.pubkey(), 1_000_000_000)
        .unwrap();
    let challenger_usdc_src = ctx.fund_usdc(&challenger, 5_000_000);

    let ix = open_challenge_ix(
        &ctx,
        c.oracle,
        c.ai_claim,
        c.proposer,
        market,
        challenger.pubkey(),
        &m,
        pass_amm,
        fail_amm,
        c.stake_vault,
        oracle_pass_kass,
        oracle_fail_kass,
        kass_dao,
        challenger_usdc_src,
        c.nonce,
    );
    ctx.send_many(&cu(ix), &[&challenger])
        .expect("open_challenge");

    let payouts = fabricate_payouts(&mut ctx, market, c.proposer_authority, challenger.pubkey());
    let escrow = required_escrow_usdc(BOND);

    // Emission is ON by default: the real-flow oracle's stake_vault also holds
    // the creation-time `reward_emission` (untouched until finalize_oracle). The
    // KASS-conservation baseline must therefore include it alongside Σ stakes.
    let total_before =
        ctx.oracle(c.oracle).total_oracle_stake + ctx.oracle(c.oracle).reward_emission;
    let bond_pool_before = ctx.oracle(c.oracle).bond_pool;
    let surviving_before = ctx.oracle(c.oracle).surviving_count;
    let stake_before = ctx.token_balance(c.stake_vault);

    ctx.warp(TWAP_WINDOW + 1);
    let extras = SettleExtras {
        stake_vault: c.stake_vault,
        kass_vault: m.kass_vault,
        kass_vault_underlying: m.kass_vault_underlying,
        pass_mint: m.pass_mint,
        fail_mint: m.fail_mint,
        oracle_pass_kass,
        oracle_fail_kass,
        escrow_vault: payouts.escrow_vault,
        proposer_usdc: payouts.proposer_usdc,
        challenger_usdc_dest: payouts.challenger_usdc_dest,
        challenger_kass: payouts.challenger_kass,
    };
    let ix = settle_ix(
        &ctx, c.oracle, market, c.ai_claim, c.proposer, m.question, pass_amm, fail_amm, &extras,
        c.nonce,
    );
    ctx.send_many(&cu(ix), &[]).expect("settle_challenge");

    let model = ConservationModel::compute(
        true,
        BOND,
        escrow,
        0,
        CHALLENGE_SUCCESS_KASS_FEE_NUM,
        CHALLENGE_SUCCESS_KASS_FEE_DEN,
        CHALLENGE_FAIL_USDC_FEE_NUM,
        CHALLENGE_FAIL_USDC_FEE_DEN,
    );

    assert_resolution_and_conservation(
        &ctx,
        c.oracle,
        market,
        c.proposer,
        m.question,
        &extras,
        &model,
        total_before,
        bond_pool_before,
        stake_before,
    );
    // Disqualify specifics: bond − kass_fee to bond_pool, surviving -= 1.
    let p = ctx.proposer(c.proposer);
    assert_eq!(p.disqualified, 1, "fraud → disqualified (swap-driven TWAP)");
    assert_eq!(p.slashed, 1);
    assert_eq!(p.slashed_amount, BOND - model.kass_fee);
    let o = ctx.oracle(c.oracle);
    assert_eq!(o.surviving_count, surviving_before - 1);
    assert_eq!(o.bond_pool, bond_pool_before + (BOND - model.kass_fee));
    let (n0, n1, denom) = question_resolution(&ctx, m.question);
    assert_eq!((n0, n1, denom), (0, 1, 1), "fail-side resolution");
}

/// The shared cross-outcome assertions: question settled, conditional KASS fully
/// redeemed + holders burned, and BOTH conservation equations against the
/// INDEPENDENT [`ConservationModel`].
#[allow(clippy::too_many_arguments)]
fn assert_resolution_and_conservation(
    ctx: &TestCtx,
    oracle: Pubkey,
    market: Pubkey,
    _proposer: Pubkey,
    _question: Pubkey,
    x: &SettleExtras,
    model: &ConservationModel,
    total_before: u64,
    _bond_pool_before: u64,
    stake_before: u64,
) {
    assert_eq!(ctx.read_pod::<Market>(market).settled, 1, "market settled");
    assert_eq!(
        ctx.oracle(oracle).open_challenge_count,
        0,
        "counter back to 0"
    );

    // Physical redeem drained the conditional KASS vault + burned both holders.
    assert_eq!(
        ctx.token_balance(x.kass_vault_underlying),
        0,
        "underlying drained"
    );
    assert_eq!(ctx.token_balance(x.oracle_pass_kass), 0, "pass-KASS burned");
    assert_eq!(ctx.token_balance(x.oracle_fail_kass), 0, "fail-KASS burned");
    // No donation present in these e2e flows: the holders carried EXACTLY the
    // bond-derived balance (see the dedicated donation test for the griefing edge).

    // KASS routing vs the independent reference.
    assert_eq!(
        ctx.token_balance(x.challenger_kass),
        model.challenger_kass()
    );
    assert_eq!(
        ctx.token_balance(x.stake_vault),
        stake_before + model.stake_vault_delta(),
        "stake_vault delta == redeem − kass_fee carve-out"
    );
    // KASS conservation: stake_vault + underlying + challenger_kass == total
    // (the kass_fee carve-out left the system to the challenger on disqualify; on
    // survive challenger_kass == 0 and it reduces to the idle-bond conservation).
    assert_eq!(
        ctx.token_balance(x.stake_vault)
            + ctx.token_balance(x.kass_vault_underlying)
            + ctx.token_balance(x.challenger_kass),
        total_before,
        "KASS conservation incl. the kass_fee carve-out",
    );

    // USDC routing + conservation vs the independent reference.
    assert_eq!(ctx.token_balance(x.proposer_usdc), model.proposer_usdc());
    assert_eq!(
        ctx.token_balance(x.challenger_usdc_dest),
        model.challenger_usdc()
    );
    assert_eq!(
        ctx.token_balance(x.proposer_usdc) + ctx.token_balance(x.challenger_usdc_dest),
        model.escrow,
        "USDC escrow fully accounted (fee + return == escrow)",
    );
    assert_eq!(ctx.token_balance(x.escrow_vault), 0, "escrow drained");
}

// ---------------------------------------------------------------------------
// Donation edge (C2 review heads-up): anyone can SPL-transfer extra conditional
// KASS into the oracle-PDA-owned holder before settle; redeem burns the FULL
// balance, pulling the extra underlying into stake_vault. This documents that the
// donation only INFLATES stake_vault (the donor forfeits their own KASS) — it is
// NOT theft (no protocol funds leave to the donor), so production is unchanged.
// ---------------------------------------------------------------------------

#[test]
fn donation_into_holder_inflates_stake_vault_not_theft() {
    let mut ctx = TestCtx::new();
    ctx.svm.add_program(vault_id(), VAULT_SO).unwrap();
    ctx.svm.add_program(amm_id(), AMM_SO).unwrap();
    let kass_dao = ctx.bless_kass_price();

    // Lighter seeded-Challenge setup (the donation mechanic is orthogonal to how
    // we reach Challenge); real open/settle + real conditional vault throughout.
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
    let proposer_authority = seeded.proposers[0].authority.pubkey();
    ctx.set_phase(oracle, Phase::Challenge);

    let (claim, bump) = Pubkey::find_program_address(
        &[b"claim", oracle.as_ref(), proposer.as_ref()],
        &ctx.program_id,
    );
    let mut a: AiClaim = bytemuck::Zeroable::zeroed();
    a.account_type = AccountType::AiClaim.as_u8();
    a.oracle = oracle.to_bytes().into();
    a.proposer = proposer.to_bytes().into();
    a.option = 0;
    a.bump = bump;
    ctx.seed_program_account_at(claim, bytemuck::bytes_of(&a).to_vec());

    let (m, oracle_pass_kass, oracle_fail_kass) = setup_market(&mut ctx, oracle);
    // Honest (survive) market: both pools neutral → pass-side resolution.
    let pass_amm = build_pool(
        &mut ctx,
        m.pass_mint,
        m.pass_usdc,
        BASE_RESERVE,
        QUOTE_NEUTRAL,
    );
    let fail_amm = build_pool(
        &mut ctx,
        m.fail_mint,
        m.fail_usdc,
        BASE_RESERVE,
        QUOTE_NEUTRAL,
    );
    crank_pool(&mut ctx, pass_amm);
    crank_pool(&mut ctx, fail_amm);

    let (market, _) = Pubkey::find_program_address(&[b"market", claim.as_ref()], &ctx.program_id);
    let challenger = Keypair::new();
    ctx.svm
        .airdrop(&challenger.pubkey(), 1_000_000_000)
        .unwrap();
    let challenger_usdc_src = ctx.fund_usdc(&challenger, 5_000_000);
    let ix = open_challenge_ix(
        &ctx,
        oracle,
        claim,
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
        .expect("open_challenge");

    // --- the donation: a third party splits D KASS in THIS market's KASS vault
    // (minting D pass-KASS + D fail-KASS to themselves) then SPL-transfers the D
    // pass-KASS into the oracle-PDA-owned pass holder. ----------------------
    let donation: u64 = 250_000_000;
    let donor = Keypair::new();
    ctx.svm.airdrop(&donor.pubkey(), 1_000_000_000).unwrap();
    let donor_kass_src = ctx.fund_kass(&donor, donation);
    let donor_pass = Pubkey::new_unique();
    let donor_fail = Pubkey::new_unique();
    fabricate_token_account(&mut ctx, donor_pass, m.pass_mint, donor.pubkey(), 0);
    fabricate_token_account(&mut ctx, donor_fail, m.fail_mint, donor.pubkey(), 0);

    let (cv_event_auth, _) =
        Pubkey::find_program_address(&metadao::event_authority_seeds(), &vault_id());
    let split_ix = Instruction {
        program_id: vault_id(),
        accounts: vec![
            AccountMeta::new_readonly(m.question, false),
            AccountMeta::new(m.kass_vault, false),
            AccountMeta::new(m.kass_vault_underlying, false),
            AccountMeta::new(donor.pubkey(), true),
            AccountMeta::new(donor_kass_src, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(cv_event_auth, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new(m.pass_mint, false),
            AccountMeta::new(m.fail_mint, false),
            AccountMeta::new(donor_pass, false),
            AccountMeta::new(donor_fail, false),
        ],
        data: metadao::split_tokens_data(donation).to_vec(),
    };
    ctx.send_many(&cu(split_ix), &[&donor])
        .expect("donor split");
    // SPL-transfer the donated pass-KASS into the oracle-PDA-owned holder.
    let xfer = spl_token::instruction::transfer(
        &TOKEN_PROGRAM_ID,
        &donor_pass,
        &oracle_pass_kass,
        &donor.pubkey(),
        &[],
        donation,
    )
    .unwrap();
    ctx.send_many(&cu(xfer), &[&donor])
        .expect("donate transfer");

    assert_eq!(
        ctx.token_balance(oracle_pass_kass),
        BOND + donation,
        "holder now carries bond + donated conditional KASS"
    );

    let payouts = fabricate_payouts(&mut ctx, market, proposer_authority, challenger.pubkey());
    let total_before = ctx.oracle(oracle).total_oracle_stake;
    let stake_before = ctx.token_balance(stake_vault);

    ctx.warp(TWAP_WINDOW + 1);
    let extras = SettleExtras {
        stake_vault,
        kass_vault: m.kass_vault,
        kass_vault_underlying: m.kass_vault_underlying,
        pass_mint: m.pass_mint,
        fail_mint: m.fail_mint,
        oracle_pass_kass,
        oracle_fail_kass,
        escrow_vault: payouts.escrow_vault,
        proposer_usdc: payouts.proposer_usdc,
        challenger_usdc_dest: payouts.challenger_usdc_dest,
        challenger_kass: payouts.challenger_kass,
    };
    let ix = settle_ix(
        &ctx, oracle, market, claim, proposer, m.question, pass_amm, fail_amm, &extras, nonce,
    );
    ctx.send_many(&cu(ix), &[]).expect("settle_challenge");

    // Redeem burned the FULL pass holder (bond + donation) → pulled bond +
    // donation into stake_vault. The proposer is NOT slashed (survive), so the
    // donation is pure inflation of stake_vault.
    assert_eq!(ctx.token_balance(oracle_pass_kass), 0, "holder burned");
    assert_eq!(
        ctx.token_balance(m.kass_vault_underlying),
        0,
        "underlying drained"
    );
    assert_eq!(
        ctx.token_balance(stake_vault),
        stake_before + BOND + donation,
        "stake_vault inflated by bond + DONATION (donor forfeited their KASS)"
    );
    // The naive conservation equation now carries the donation on top of the
    // conserved total: stake_vault + underlying == total + donation. The donor's
    // own KASS was pulled in and is NOT recoverable by them (their fail-KASS is
    // worthless) — external griefing that only ADDS KASS to the protocol, never
    // theft, so production is intentionally NOT guarded against it.
    assert_eq!(
        ctx.token_balance(stake_vault) + ctx.token_balance(m.kass_vault_underlying),
        total_before + donation,
        "donation inflates stake_vault beyond the conserved total (no funds stolen)"
    );
    assert_eq!(
        ctx.proposer(proposer).disqualified,
        0,
        "honest still survives"
    );
}

// ---------------------------------------------------------------------------
// Conservation FUZZ (deliverable 2)
//
// Sweeps both outcomes × fee rates × bond sizes × pass/fail TWAP relation and
// asserts the KASS + USDC conservation equations across REAL open_challenge +
// settle_challenge against the INDEPENDENT ConservationModel. To keep each case
// cheap (the real-AMM TWAP-production path is heavy — and is covered by the two
// e2e tests above + settle_challenge.rs), the pass/fail AMMs are FABRICATED
// AMM-program-owned accounts carrying a chosen aggregator, so `verify_and_read_twap`
// reads a known pass_twap/fail_twap. open_challenge (split + escrow) and
// settle_challenge (redeem + directional fees) are the REAL instructions under test.
// ---------------------------------------------------------------------------

/// Fabricate an `Amm`-program-owned account that `verify_and_read_twap` accepts
/// (correct discriminator + base/quote conditional mints) and whose stored
/// aggregator/slots yield exactly `twap` (`twap == 0` ⇒ no observation).
fn fabricate_amm_with_twap(ctx: &mut TestCtx, base: Pubkey, quote: Pubkey, twap: u128) -> Pubkey {
    let addr = Pubkey::new_unique();
    let mut data = vec![0u8; metadao::AMM_MIN_LEN.max(256)];
    data[..8].copy_from_slice(&metadao::AMM_ACCOUNT_DISCRIMINATOR);
    let slots: u64 = 1_000;
    let (created_at, last_updated, aggregator): (u64, u64, u128) = if twap == 0 {
        (0, 0, 0)
    } else {
        (0, slots, twap * slots as u128)
    };
    data[metadao::AMM_CREATED_AT_SLOT_OFFSET..metadao::AMM_CREATED_AT_SLOT_OFFSET + 8]
        .copy_from_slice(&created_at.to_le_bytes());
    data[metadao::AMM_BASE_MINT_OFFSET..metadao::AMM_BASE_MINT_OFFSET + 32]
        .copy_from_slice(&base.to_bytes());
    data[metadao::AMM_QUOTE_MINT_OFFSET..metadao::AMM_QUOTE_MINT_OFFSET + 32]
        .copy_from_slice(&quote.to_bytes());
    data[metadao::AMM_LAST_UPDATED_SLOT_OFFSET..metadao::AMM_LAST_UPDATED_SLOT_OFFSET + 8]
        .copy_from_slice(&last_updated.to_le_bytes());
    data[metadao::AMM_AGGREGATOR_OFFSET..metadao::AMM_AGGREGATOR_OFFSET + 16]
        .copy_from_slice(&aggregator.to_le_bytes());
    data[metadao::AMM_START_DELAY_SLOTS_OFFSET..metadao::AMM_START_DELAY_SLOTS_OFFSET + 8]
        .copy_from_slice(&0u64.to_le_bytes());
    let lamports = ctx.svm.minimum_balance_for_rent_exemption(data.len());
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

#[derive(Clone, Copy, Debug)]
struct FuzzCase {
    bond: u64,
    pass_twap: u128,
    fail_twap: u128,
    succ_num: u64,
    succ_den: u64,
    fail_num: u64,
    fail_den: u64,
}

fn fuzz_case_strategy() -> impl Strategy<Value = FuzzCase> {
    (
        1_000_000u64..5_000_000_000u64, // bond (escrow = bond * 5e8 / 1e12 > 0)
        0u128..4_000_000_000u128,       // pass_twap (incl. 0 → always survive)
        100_000_000u128..12_000_000_000u128, // fail_twap
        // Fee rates within bounds (num ≤ den, den > 0). Keep succ_num/den ≤ ~50%
        // so the kass_fee never collides with the (here-zero) prior slash.
        (1u64..=50u64, 100u64..=100u64),
        (1u64..=50u64, 100u64..=100u64),
    )
        .prop_map(
            |(bond, pass_twap, fail_twap, (succ_num, succ_den), (fail_num, fail_den))| FuzzCase {
                bond,
                pass_twap,
                fail_twap,
                succ_num,
                succ_den,
                fail_num,
                fail_den,
            },
        )
}

fn run_fuzz_case(fc: &FuzzCase) -> Result<(), TestCaseError> {
    let mut ctx = TestCtx::new();
    ctx.svm.add_program(vault_id(), VAULT_SO).unwrap();
    ctx.svm.add_program(amm_id(), AMM_SO).unwrap();
    let kass_dao = ctx.bless_kass_price();

    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: fc.bond,
        },
        ProposerSpec {
            option: 1,
            bond: fc.bond,
        },
    ]);
    // Retune the per-oracle fee snapshot to the fuzzed rates.
    ctx.set_challenge_fees(oracle, fc.fail_num, fc.fail_den, fc.succ_num, fc.succ_den);
    let seeded = ctx.seeded(oracle);
    let nonce = seeded.nonce;
    let stake_vault = seeded.stake_vault;
    let proposer = seeded.proposers[0].pda;
    let proposer_authority = seeded.proposers[0].authority.pubkey();
    ctx.set_phase(oracle, Phase::Challenge);

    let (claim, bump) = Pubkey::find_program_address(
        &[b"claim", oracle.as_ref(), proposer.as_ref()],
        &ctx.program_id,
    );
    let mut a: AiClaim = bytemuck::Zeroable::zeroed();
    a.account_type = AccountType::AiClaim.as_u8();
    a.oracle = oracle.to_bytes().into();
    a.proposer = proposer.to_bytes().into();
    a.option = 0;
    a.bump = bump;
    ctx.seed_program_account_at(claim, bytemuck::bytes_of(&a).to_vec());

    let (m, oracle_pass_kass, oracle_fail_kass) = setup_market(&mut ctx, oracle);
    // Stubbed-TWAP AMMs (see module note): known pass/fail TWAP, real binding.
    let pass_amm = fabricate_amm_with_twap(&mut ctx, m.pass_mint, m.pass_usdc, fc.pass_twap);
    let fail_amm = fabricate_amm_with_twap(&mut ctx, m.fail_mint, m.fail_usdc, fc.fail_twap);

    let (market, _) = Pubkey::find_program_address(&[b"market", claim.as_ref()], &ctx.program_id);
    let challenger = Keypair::new();
    ctx.svm
        .airdrop(&challenger.pubkey(), 1_000_000_000)
        .unwrap();
    let challenger_usdc_src = ctx.fund_usdc(&challenger, 1_000_000_000);

    let ix = open_challenge_ix(
        &ctx,
        oracle,
        claim,
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
        .map_err(|e| TestCaseError::fail(format!("open_challenge: {e:?}")))?;

    let payouts = fabricate_payouts(&mut ctx, market, proposer_authority, challenger.pubkey());
    let escrow = ctx.token_balance(payouts.escrow_vault);
    prop_assert!(escrow > 0, "escrow must be funded");

    let total_before = ctx.oracle(oracle).total_oracle_stake;
    let bond_pool_before = ctx.oracle(oracle).bond_pool;
    let stake_before = ctx.token_balance(stake_vault);

    ctx.warp(TWAP_WINDOW + 1);
    let extras = SettleExtras {
        stake_vault,
        kass_vault: m.kass_vault,
        kass_vault_underlying: m.kass_vault_underlying,
        pass_mint: m.pass_mint,
        fail_mint: m.fail_mint,
        oracle_pass_kass,
        oracle_fail_kass,
        escrow_vault: payouts.escrow_vault,
        proposer_usdc: payouts.proposer_usdc,
        challenger_usdc_dest: payouts.challenger_usdc_dest,
        challenger_kass: payouts.challenger_kass,
    };
    let ix = settle_ix(
        &ctx, oracle, market, claim, proposer, m.question, pass_amm, fail_amm, &extras, nonce,
    );
    ctx.send_many(&cu(ix), &[])
        .map_err(|e| TestCaseError::fail(format!("settle_challenge: {e:?}")))?;

    // Independent reference: outcome + conservation.
    let disqualify = ref_disqualify(fc.pass_twap, fc.fail_twap);
    let model = ConservationModel::compute(
        disqualify,
        fc.bond,
        escrow,
        0,
        fc.succ_num,
        fc.succ_den,
        fc.fail_num,
        fc.fail_den,
    );

    let (n0, n1, _) = question_resolution(&ctx, m.question);
    if disqualify {
        prop_assert_eq!((n0, n1), (0, 1));
    } else {
        prop_assert_eq!((n0, n1), (1, 0));
    }
    prop_assert_eq!(ctx.proposer(proposer).disqualified != 0, disqualify);

    // KASS.
    prop_assert_eq!(
        ctx.token_balance(payouts.challenger_kass),
        model.challenger_kass()
    );
    prop_assert_eq!(
        ctx.token_balance(stake_vault),
        stake_before + model.stake_vault_delta()
    );
    prop_assert_eq!(ctx.token_balance(m.kass_vault_underlying), 0);
    prop_assert_eq!(
        ctx.token_balance(stake_vault)
            + ctx.token_balance(m.kass_vault_underlying)
            + ctx.token_balance(payouts.challenger_kass),
        total_before,
        "KASS conservation incl. the kass_fee carve-out"
    );
    // USDC.
    prop_assert_eq!(
        ctx.token_balance(payouts.proposer_usdc),
        model.proposer_usdc()
    );
    prop_assert_eq!(
        ctx.token_balance(payouts.challenger_usdc_dest),
        model.challenger_usdc()
    );
    prop_assert_eq!(
        ctx.token_balance(payouts.proposer_usdc) + ctx.token_balance(payouts.challenger_usdc_dest),
        escrow,
        "USDC escrow fully accounted"
    );
    // bond_pool identity.
    if disqualify {
        prop_assert_eq!(
            ctx.oracle(oracle).bond_pool,
            bond_pool_before + model.stake_vault_delta()
        );
    } else {
        prop_assert_eq!(ctx.oracle(oracle).bond_pool, bond_pool_before);
    }
    Ok(())
}

proptest! {
    // Each case rebuilds LiteSVM, loads the vault + amm binaries, composes the
    // real conditional vaults, and drives real open + settle (~7 txs/case), so
    // the count is kept modest to stay fast and non-flaky.
    #![proptest_config(ProptestConfig {
        cases: 24,
        max_shrink_iters: 32,
        .. ProptestConfig::default()
    })]

    #[test]
    fn challenge_conservation_fuzz(fc in fuzz_case_strategy()) {
        run_fuzz_case(&fc)?;
    }
}

//! `open_challenge` (Task 10): verify a decomposed MetaDAO decision market,
//! program-sign the proposer-KASS split, and record a [`Market`] PDA.
//!
//! The challenger composes the MetaDAO accounts (binary question with
//! resolver == the Kassandra oracle PDA, a KASS conditional vault, a USDC
//! conditional vault, and pass/fail AMMs) by driving the REAL deployed
//! conditional_vault binary in-test (same wire format as `metadao_cpi.rs`).
//! `open_challenge` then verifies + records them and splits the proposer's
//! escrowed KASS into pass/fail conditional KASS, all program-signed.

mod common;
use common::*;

use kassandra_oracles_program::{
    cpi::metadao,
    error::KassandraError,
    instruction::Ix,
    state::{AccountType, AiClaim, Market, Phase},
};
use solana_account::Account;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_instruction::{AccountMeta, Instruction};
use solana_instruction_error::InstructionError;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction_error::TransactionError;
use spl_token::{
    solana_program::{program_option::COption, program_pack::Pack},
    state::{Account as TokenAccount, AccountState},
    ID as TOKEN_PROGRAM_ID,
};

const VAULT_SO: &[u8] = include_bytes!("fixtures/metadao_conditional_vault.so");
const AMM_SO: &[u8] = include_bytes!("fixtures/metadao_amm.so");

const ATA_PROGRAM_ID: Pubkey =
    solana_pubkey::pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

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

/// Fabricate an `Amm`-program-owned account carrying the `Amm` account
/// discriminator and the given `base`/`quote` conditional mints at the layout
/// offsets `open_challenge` (and `settle_challenge`) bind against. The TWAP
/// fields are left zero — irrelevant to open_challenge's mint-pair binding.
fn fabricate_amm_account(ctx: &mut TestCtx, base: Pubkey, quote: Pubkey) -> Pubkey {
    let addr = Pubkey::new_unique();
    let mut data = vec![0u8; metadao::AMM_MIN_LEN];
    data[..8].copy_from_slice(&metadao::AMM_ACCOUNT_DISCRIMINATOR);
    data[metadao::AMM_BASE_MINT_OFFSET..metadao::AMM_BASE_MINT_OFFSET + 32]
        .copy_from_slice(&base.to_bytes());
    data[metadao::AMM_QUOTE_MINT_OFFSET..metadao::AMM_QUOTE_MINT_OFFSET + 32]
        .copy_from_slice(&quote.to_bytes());
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

/// Every MetaDAO account a challenge market binds to.
struct MarketAccounts {
    question: Pubkey,
    kass_vault: Pubkey,
    kass_vault_underlying: Pubkey,
    pass_mint: Pubkey,
    fail_mint: Pubkey,
    usdc_vault: Pubkey,
    pass_amm: Pubkey,
    fail_amm: Pubkey,
}

fn cu(ix: Instruction) -> [Instruction; 2] {
    [
        ComputeBudgetInstruction::set_compute_unit_limit(600_000),
        ix,
    ]
}

/// Compose the MetaDAO market for `resolver` (the question's oracle/resolver):
/// initialize_question(num_outcomes=2) → KASS conditional vault → USDC
/// conditional vault → pass/fail AMM stubs. Returns the bound accounts plus the
/// oracle-PDA-owned conditional KASS destinations.
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
    let kass_vault_arr = kass_vault.to_bytes();
    let (pass_mint, _) = Pubkey::find_program_address(
        &metadao::conditional_token_mint_seeds(&kass_vault_arr.into(), &[0u8]),
        &vault_id(),
    );
    let (fail_mint, _) = Pubkey::find_program_address(
        &metadao::conditional_token_mint_seeds(&kass_vault_arr.into(), &[1u8]),
        &vault_id(),
    );
    let usdc_vault_arr = usdc_vault.to_bytes();
    let (usdc_pass, _) = Pubkey::find_program_address(
        &metadao::conditional_token_mint_seeds(&usdc_vault_arr.into(), &[0u8]),
        &vault_id(),
    );
    let (usdc_fail, _) = Pubkey::find_program_address(
        &metadao::conditional_token_mint_seeds(&usdc_vault_arr.into(), &[1u8]),
        &vault_id(),
    );
    let (event_authority, _) =
        Pubkey::find_program_address(&metadao::event_authority_seeds(), &vault_id());

    let kass_vault_underlying = ata(&kass_vault, &kass);
    let usdc_vault_underlying = ata(&usdc_vault, &usdc);

    // --- initialize_question ---
    let payer = ctx.payer.pubkey();
    let ix_q = Instruction {
        program_id: vault_id(),
        accounts: vec![
            AccountMeta::new(question, false),
            AccountMeta::new(payer, true),
            AccountMeta::new_readonly(solana_sdk_ids::system_program::ID, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(vault_id(), false),
        ],
        data: metadao::initialize_question_data(&question_id, &resolver_arr.into(), num_outcomes)
            .to_vec(),
    };
    ctx.send_many(&cu(ix_q), &[])
        .expect("initialize_question failed");

    // --- KASS conditional vault ---
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
            AccountMeta::new_readonly(solana_sdk_ids::system_program::ID, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new(pass_mint, false),
            AccountMeta::new(fail_mint, false),
        ],
        data: metadao::initialize_conditional_vault_data().to_vec(),
    };
    ctx.send_many(&cu(ix_kv), &[])
        .expect("init KASS vault failed");

    // --- USDC conditional vault ---
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
            AccountMeta::new_readonly(solana_sdk_ids::system_program::ID, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new(usdc_pass, false),
            AccountMeta::new(usdc_fail, false),
        ],
        data: metadao::initialize_conditional_vault_data().to_vec(),
    };
    ctx.send_many(&cu(ix_uv), &[])
        .expect("init USDC vault failed");

    let pass_amm = fabricate_amm_account(ctx, pass_mint, usdc_pass);
    let fail_amm = fabricate_amm_account(ctx, fail_mint, usdc_fail);

    // Oracle-PDA-owned destinations for the minted pass/fail conditional KASS.
    let oracle_pass_kass = Pubkey::new_unique();
    let oracle_fail_kass = Pubkey::new_unique();
    fabricate_token_account(ctx, oracle_pass_kass, pass_mint, resolver, 0);
    fabricate_token_account(ctx, oracle_fail_kass, fail_mint, resolver, 0);

    (
        MarketAccounts {
            question,
            kass_vault,
            kass_vault_underlying,
            pass_mint,
            fail_mint,
            usdc_vault,
            pass_amm,
            fail_amm,
        },
        oracle_pass_kass,
        oracle_fail_kass,
    )
}

/// Fabricate an `AiClaim` at its `[b"claim", oracle, proposer]` PDA, bound to
/// the given oracle/proposer, with `option` and `challenged == 0`.
fn seed_ai_claim(ctx: &mut TestCtx, oracle: Pubkey, proposer: Pubkey, option: u8) -> Pubkey {
    let (claim, bump) = Pubkey::find_program_address(
        &[b"claim", oracle.as_ref(), proposer.as_ref()],
        &ctx.program_id,
    );
    let mut c: AiClaim = bytemuck::Zeroable::zeroed();
    c.account_type = AccountType::AiClaim.as_u8();
    c.oracle = oracle.to_bytes().into();
    c.proposer = proposer.to_bytes().into();
    c.option = option;
    c.challenged = 0;
    c.bump = bump;
    ctx.seed_program_account_at(claim, bytemuck::bytes_of(&c).to_vec());
    claim
}

/// Build the full `open_challenge` instruction. The challenger USDC escrow size
/// is computed on-chain (no payload amount); the caller passes the challenger's
/// USDC source account + the blessed `kass_dao`, and the protocol/escrow-vault
/// PDAs are derived here.
#[allow(clippy::too_many_arguments)]
fn open_challenge_ix(
    ctx: &TestCtx,
    oracle: Pubkey,
    ai_claim: Pubkey,
    proposer: Pubkey,
    market: Pubkey,
    challenger: Pubkey,
    m: &MarketAccounts,
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
            AccountMeta::new_readonly(m.pass_amm, false),
            AccountMeta::new_readonly(m.fail_amm, false),
            AccountMeta::new(stake_vault, false),
            AccountMeta::new(m.kass_vault_underlying, false),
            AccountMeta::new(m.pass_mint, false),
            AccountMeta::new(m.fail_mint, false),
            AccountMeta::new(oracle_pass_kass, false),
            AccountMeta::new(oracle_fail_kass, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(solana_sdk_ids::system_program::ID, false),
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

/// Common fixture: a disputed oracle in the `Challenge` phase, with one
/// surviving challenged-proposer's AiClaim and a fully composed MetaDAO market.
struct Fixture {
    oracle: Pubkey,
    nonce: u64,
    stake_vault: Pubkey,
    proposer: Pubkey,
    bond: u64,
    ai_claim: Pubkey,
    market: Pubkey,
    challenger: Keypair,
    m: MarketAccounts,
    oracle_pass_kass: Pubkey,
    oracle_fail_kass: Pubkey,
    kass_dao: Pubkey,
    challenger_usdc_src: Pubkey,
}

fn fixture() -> (TestCtx, Fixture) {
    fixture_with_bond(1_000_000_000)
}

fn fixture_with_bond(bond0: u64) -> (TestCtx, Fixture) {
    let mut ctx = TestCtx::new();
    ctx.svm.add_program(vault_id(), VAULT_SO).unwrap();
    ctx.svm.add_program(amm_id(), AMM_SO).unwrap();

    // Protocol + governance handoff with a deterministic kass_price so the
    // on-chain escrow sizing is computable.
    let kass_dao = ctx.bless_kass_price();

    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: bond0,
        },
        ProposerSpec {
            option: 1,
            bond: 1_000_000_000,
        },
    ]);
    let seeded = ctx.seeded(oracle);
    let nonce = seeded.nonce;
    let stake_vault = seeded.stake_vault;
    let proposer = seeded.proposers[0].pda;
    let bond = seeded.proposers[0].bond;

    ctx.set_phase(oracle, Phase::Challenge);
    let ai_claim = seed_ai_claim(&mut ctx, oracle, proposer, 0);

    let (m, oracle_pass_kass, oracle_fail_kass) = setup_market(&mut ctx, oracle);

    let (market, _) =
        Pubkey::find_program_address(&[b"market", ai_claim.as_ref()], &ctx.program_id);

    let challenger = Keypair::new();
    ctx.svm
        .airdrop(&challenger.pubkey(), 1_000_000_000)
        .unwrap();
    // Fund the challenger's USDC source generously (escrow needs bond×price).
    let challenger_usdc_src = ctx.fund_usdc(&challenger, 5_000_000);

    (
        ctx,
        Fixture {
            oracle,
            nonce,
            stake_vault,
            proposer,
            bond,
            ai_claim,
            market,
            challenger,
            m,
            oracle_pass_kass,
            oracle_fail_kass,
            kass_dao,
            challenger_usdc_src,
        },
    )
}

#[test]
fn open_challenge_happy_path() {
    let (mut ctx, f) = fixture();

    let stake_before = ctx.token_balance(f.stake_vault);
    let now_before = ctx.now();
    let src_before = ctx.token_balance(f.challenger_usdc_src);
    let expected_usdc = required_escrow_usdc(f.bond);
    let (escrow_vault, _) = TestCtx::challenge_usdc_vault_pda(&ctx.program_id, &f.market);

    let ix = open_challenge_ix(
        &ctx,
        f.oracle,
        f.ai_claim,
        f.proposer,
        f.market,
        f.challenger.pubkey(),
        &f.m,
        f.stake_vault,
        f.oracle_pass_kass,
        f.oracle_fail_kass,
        f.kass_dao,
        f.challenger_usdc_src,
        f.nonce,
    );
    ctx.send_many(&cu(ix), &[&f.challenger])
        .expect("open_challenge should succeed");

    // Market PDA created + populated.
    let market: Market = ctx.read_pod(f.market);
    assert_eq!(market.account_type, AccountType::Market.as_u8());
    assert_eq!(market.oracle, f.oracle.to_bytes().into());
    assert_eq!(market.ai_claim, f.ai_claim.to_bytes().into());
    assert_eq!(market.proposer, f.proposer.to_bytes().into());
    assert_eq!(market.challenger, f.challenger.pubkey().to_bytes().into());
    assert_eq!(market.question, f.m.question.to_bytes().into());
    assert_eq!(market.kass_vault, f.m.kass_vault.to_bytes().into());
    assert_eq!(market.usdc_vault, f.m.usdc_vault.to_bytes().into());
    assert_eq!(market.pass_amm, f.m.pass_amm.to_bytes().into());
    assert_eq!(market.fail_amm, f.m.fail_amm.to_bytes().into());
    assert_eq!(
        market.oracle_pass_kass,
        f.oracle_pass_kass.to_bytes().into()
    );
    assert_eq!(
        market.oracle_fail_kass,
        f.oracle_fail_kass.to_bytes().into()
    );
    assert_eq!(market.challenger_usdc_vault, escrow_vault.to_bytes().into());
    assert_eq!(market.twap_end, now_before + TWAP_WINDOW);
    assert_eq!(market.settled, 0);

    // Escrow: exactly bond × kass_price USDC moved challenger → market vault.
    assert!(
        expected_usdc > 0,
        "sanity: nonzero escrow at the test price"
    );
    assert_eq!(
        market.challenger_usdc, expected_usdc,
        "Market records the on-chain-computed escrow size"
    );
    assert_eq!(
        ctx.token_balance(escrow_vault),
        expected_usdc,
        "escrow vault holds exactly bond × kass_price USDC"
    );
    assert_eq!(
        ctx.token_balance(f.challenger_usdc_src),
        src_before - expected_usdc,
        "challenger's USDC source debited by the escrow amount"
    );
    // Escrow vault is on the USDC mint, token authority == oracle PDA.
    let (mint, owner, _amt) = ctx.token_account(escrow_vault);
    assert_eq!(mint, ctx.usdc_mint.to_bytes());
    assert_eq!(owner, f.oracle.to_bytes());

    // Claim flipped to challenged.
    assert_eq!(ctx.ai_claim(f.ai_claim).challenged, 1);

    // Program-signed split moved exactly the bond out of the stake vault into
    // the KASS conditional vault, minting pass/fail conditional KASS to the
    // oracle-PDA-owned destinations.
    assert_eq!(ctx.token_balance(f.stake_vault), stake_before - f.bond);
    assert_eq!(ctx.token_balance(f.m.kass_vault_underlying), f.bond);
    assert_eq!(ctx.token_balance(f.oracle_pass_kass), f.bond);
    assert_eq!(ctx.token_balance(f.oracle_fail_kass), f.bond);
}

#[test]
fn open_challenge_insufficient_usdc_fails() {
    let (mut ctx, f) = fixture();

    // A USDC source holding far less than the required escrow (bond × price).
    // The escrow Transfer must fail, reverting the whole instruction — no Market
    // and no challenged flip persist.
    let expected_usdc = required_escrow_usdc(f.bond);
    assert!(expected_usdc > 1, "test price requires a real escrow");
    let poor_src = ctx.fund_usdc(&f.challenger, 1);

    let ix = open_challenge_ix(
        &ctx,
        f.oracle,
        f.ai_claim,
        f.proposer,
        f.market,
        f.challenger.pubkey(),
        &f.m,
        f.stake_vault,
        f.oracle_pass_kass,
        f.oracle_fail_kass,
        f.kass_dao,
        poor_src,
        f.nonce,
    );
    let res = ctx.send_many(&cu(ix), &[&f.challenger]);
    assert!(
        res.is_err(),
        "an under-funded challenger must fail the escrow Transfer: {res:?}"
    );
    // Whole tx reverted: no Market account, claim not challenged.
    assert!(
        ctx.svm.get_account(&f.market).is_none(),
        "failed escrow must not leave a Market account"
    );
    assert_eq!(ctx.ai_claim(f.ai_claim).challenged, 0);
}

#[test]
fn open_challenge_zero_escrow_fails() {
    // A sub-micro bond (1 base unit) prices to `1 × 5e8 / 1e12 == 0` USDC escrow.
    // A zero-escrow challenge has no skin-in-the-game and no source for the
    // directional USDC fee at settle, so open_challenge must reject it (ZeroStake)
    // BEFORE moving any funds — no Market, no challenged flip.
    let (mut ctx, f) = fixture_with_bond(1);
    assert_eq!(
        required_escrow_usdc(f.bond),
        0,
        "sanity: escrow truncates to 0"
    );

    let ix = open_challenge_ix(
        &ctx,
        f.oracle,
        f.ai_claim,
        f.proposer,
        f.market,
        f.challenger.pubkey(),
        &f.m,
        f.stake_vault,
        f.oracle_pass_kass,
        f.oracle_fail_kass,
        f.kass_dao,
        f.challenger_usdc_src,
        f.nonce,
    );
    let err = ctx.send_many(&cu(ix), &[&f.challenger]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(KassandraError::ZeroStake as u32),
        ),
    );
    assert!(
        ctx.svm.get_account(&f.market).is_none(),
        "zero-escrow reject must not leave a Market account"
    );
    assert_eq!(ctx.ai_claim(f.ai_claim).challenged, 0);
}

/// Regression: an AMM that can't bind to this market's conditional (KASS, USDC)
/// mint pair must be REJECTED at open — never recorded on the Market. Recording
/// an unbindable AMM would make `settle_challenge` (which pins to the recorded
/// address) revert forever: `open_challenge_count` would stay > 0, blocking
/// `finalize_oracle` and permanently locking every stake in the oracle.
#[test]
fn open_challenge_unbindable_amm_rejected() {
    let (mut ctx, mut f) = fixture();
    // An AMM owned by the AMM program with the right discriminator but the WRONG
    // (base, quote) mints: passes the owner check the old code relied on, fails
    // the mint-pair binding the fix now enforces at open.
    let bogus = fabricate_amm_account(&mut ctx, Pubkey::new_unique(), Pubkey::new_unique());
    f.m.pass_amm = bogus;

    let ix = open_challenge_ix(
        &ctx,
        f.oracle,
        f.ai_claim,
        f.proposer,
        f.market,
        f.challenger.pubkey(),
        &f.m,
        f.stake_vault,
        f.oracle_pass_kass,
        f.oracle_fail_kass,
        f.kass_dao,
        f.challenger_usdc_src,
        f.nonce,
    );
    let err = ctx.send_many(&cu(ix), &[&f.challenger]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(KassandraError::InvalidAccount as u32),
        ),
    );
    // The brick precondition must be impossible: no Market, claim not flipped.
    assert!(
        ctx.svm.get_account(&f.market).is_none(),
        "an unbindable AMM must not create a Market"
    );
    assert_eq!(
        ctx.ai_claim(f.ai_claim).challenged,
        0,
        "claim must not be flipped to challenged"
    );
}

/// Regression: `pass_amm == fail_amm` must be rejected at open — a challenger
/// cannot collapse the two outcome pools into one it steers. A single pool
/// cannot bind to both outcomes' (KASS, USDC) mint pairs, and the explicit
/// `pass_amm != fail_amm` guard backs it up. (Previously only `settle` caught
/// this — too late, after the Market was already recorded.)
#[test]
fn open_challenge_aliased_amms_rejected() {
    let (mut ctx, mut f) = fixture();
    f.m.fail_amm = f.m.pass_amm;

    let ix = open_challenge_ix(
        &ctx,
        f.oracle,
        f.ai_claim,
        f.proposer,
        f.market,
        f.challenger.pubkey(),
        &f.m,
        f.stake_vault,
        f.oracle_pass_kass,
        f.oracle_fail_kass,
        f.kass_dao,
        f.challenger_usdc_src,
        f.nonce,
    );
    let err = ctx.send_many(&cu(ix), &[&f.challenger]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(KassandraError::InvalidAccount as u32),
        ),
    );
    assert!(
        ctx.svm.get_account(&f.market).is_none(),
        "aliased AMMs must not create a Market"
    );
}

#[test]
fn open_challenge_twice_is_already_challenged() {
    let (mut ctx, f) = fixture();

    let ix = open_challenge_ix(
        &ctx,
        f.oracle,
        f.ai_claim,
        f.proposer,
        f.market,
        f.challenger.pubkey(),
        &f.m,
        f.stake_vault,
        f.oracle_pass_kass,
        f.oracle_fail_kass,
        f.kass_dao,
        f.challenger_usdc_src,
        f.nonce,
    );
    ctx.send_many(&cu(ix.clone()), &[&f.challenger])
        .expect("first open_challenge should succeed");

    // Second attempt: the claim is now challenged.
    let err = ctx.send_many(&cu(ix), &[&f.challenger]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(KassandraError::AlreadyChallenged as u32),
        ),
    );
}

#[test]
fn open_challenge_against_disqualified_proposer_fails() {
    let (mut ctx, f) = fixture();
    ctx.set_proposer_disqualified(f.proposer);

    let ix = open_challenge_ix(
        &ctx,
        f.oracle,
        f.ai_claim,
        f.proposer,
        f.market,
        f.challenger.pubkey(),
        &f.m,
        f.stake_vault,
        f.oracle_pass_kass,
        f.oracle_fail_kass,
        f.kass_dao,
        f.challenger_usdc_src,
        f.nonce,
    );
    let err = ctx.send_many(&cu(ix), &[&f.challenger]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(KassandraError::Unauthorized as u32),
        ),
    );
}

#[test]
fn open_challenge_wrong_phase_fails() {
    let (mut ctx, f) = fixture();
    // Knock the oracle out of Challenge.
    ctx.set_phase(f.oracle, Phase::AiClaim);

    let ix = open_challenge_ix(
        &ctx,
        f.oracle,
        f.ai_claim,
        f.proposer,
        f.market,
        f.challenger.pubkey(),
        &f.m,
        f.stake_vault,
        f.oracle_pass_kass,
        f.oracle_fail_kass,
        f.kass_dao,
        f.challenger_usdc_src,
        f.nonce,
    );
    let err = ctx.send_many(&cu(ix), &[&f.challenger]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(KassandraError::WrongPhase as u32),
        ),
    );
}

#[test]
fn open_challenge_after_window_fails() {
    let (mut ctx, f) = fixture();
    ctx.warp(WINDOW + 1);

    let ix = open_challenge_ix(
        &ctx,
        f.oracle,
        f.ai_claim,
        f.proposer,
        f.market,
        f.challenger.pubkey(),
        &f.m,
        f.stake_vault,
        f.oracle_pass_kass,
        f.oracle_fail_kass,
        f.kass_dao,
        f.challenger_usdc_src,
        f.nonce,
    );
    let err = ctx.send_many(&cu(ix), &[&f.challenger]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(KassandraError::WindowClosed as u32),
        ),
    );
}

#[test]
fn open_challenge_question_not_bound_to_oracle_fails() {
    let mut ctx = TestCtx::new();
    ctx.svm.add_program(vault_id(), VAULT_SO).unwrap();
    ctx.svm.add_program(amm_id(), AMM_SO).unwrap();

    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: 1_000_000_000,
        },
        ProposerSpec {
            option: 1,
            bond: 1_000_000_000,
        },
    ]);
    let seeded = ctx.seeded(oracle);
    let nonce = seeded.nonce;
    let stake_vault = seeded.stake_vault;
    let proposer = seeded.proposers[0].pda;
    ctx.set_phase(oracle, Phase::Challenge);
    let ai_claim = seed_ai_claim(&mut ctx, oracle, proposer, 0);

    // Build the market with a DIFFERENT resolver — its question.oracle will not
    // equal the Kassandra oracle PDA, so settle could never resolve it.
    let bogus_resolver = Pubkey::new_unique();
    let (m, oracle_pass_kass, oracle_fail_kass) = setup_market(&mut ctx, bogus_resolver);

    let (market, _) =
        Pubkey::find_program_address(&[b"market", ai_claim.as_ref()], &ctx.program_id);
    let challenger = Keypair::new();
    ctx.svm
        .airdrop(&challenger.pubkey(), 1_000_000_000)
        .unwrap();

    let ix = open_challenge_ix(
        &ctx,
        oracle,
        ai_claim,
        proposer,
        market,
        challenger.pubkey(),
        &m,
        stake_vault,
        oracle_pass_kass,
        oracle_fail_kass,
        // The question.oracle binding fails before escrow pricing, so these
        // escrow accounts are never read (placeholders).
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        nonce,
    );
    let err = ctx.send_many(&cu(ix), &[&challenger]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(KassandraError::InvalidAccount as u32),
        ),
    );
}

#[test]
fn unchallenged_claim_has_no_market_account() {
    // Dormant by default: seeding a disputed oracle + an AiClaim creates NO
    // Market account — zero cost on the uncontested happy path.
    let mut ctx = TestCtx::new();
    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: 1_000_000_000,
        },
        ProposerSpec {
            option: 1,
            bond: 1_000_000_000,
        },
    ]);
    let proposer = ctx.seeded(oracle).proposers[0].pda;
    ctx.set_phase(oracle, Phase::Challenge);
    let ai_claim = seed_ai_claim(&mut ctx, oracle, proposer, 0);

    let (market, _) =
        Pubkey::find_program_address(&[b"market", ai_claim.as_ref()], &ctx.program_id);
    assert!(
        ctx.svm.get_account(&market).is_none(),
        "no challenge means no Market account"
    );
}

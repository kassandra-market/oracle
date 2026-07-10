//! Integration tests for `resolve_market` (Ix 8): bridge a terminal Kassandra
//! oracle result into the market's MetaDAO `resolve_question`, then let users
//! redeem their winning conditional tokens.
//!
//! Drives the REAL deployed MetaDAO v0.4 `conditional_vault` + `amm` binaries in
//! LiteSVM (via `ctx.load_metadao()`), reusing the full `activate` flow.
//!
//! The `#[test]`s are split across sibling modules (`binary`, `categorical`);
//! this file keeps the shared constants and setup/holder helpers.

mod common;
use common::*;
use kassandra_markets_program::error::MarketError;
use kassandra_markets_program::state::{Market, MarketStatus};
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

#[path = "resolve_market/binary.rs"]
mod binary;
#[path = "resolve_market/categorical.rs"]
mod categorical;

pub(crate) const PROPOSAL: u8 = 1; // kassandra Phase::Proposal (non-terminal)
pub(crate) const INVALID_DEADEND: u8 = 8; // kassandra Phase::InvalidDeadend (terminal void)
pub(crate) const MIN_LIQ: u64 = 1_000_000_000; // 1 KASS (9 dp)
pub(crate) const SPLIT_AMT: u64 = 2_000_000_000; // 2 KASS a test user splits for redemption

/// `Question` field byte offsets (after the 8-byte Anchor disc): the two binary
/// payout numerators and the payout denominator (`is_resolved ⇔ denominator != 0`).
pub(crate) const Q_NUM0_OFFSET: usize = 76;
pub(crate) const Q_NUM1_OFFSET: usize = 80;
pub(crate) const Q_DENOMINATOR_OFFSET: usize = 84;

/// Stand up an ACTIVE market: fund to `MIN_LIQ`, compose the MetaDAO market, and
/// `activate`. Returns the context, KASS mint, market PDA, oracle, MetaDAO refs.
pub(crate) fn setup_active() -> (TestCtx, Pubkey, Pubkey, Pubkey, MetaDaoRefs) {
    let mut ctx = TestCtx::new();
    ctx.load_metadao();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    let (_cfg, res) = ctx.init_config(authority.pubkey(), kass, MIN_LIQ);
    assert!(res.is_ok(), "{res:?}");

    let oracle = ctx.seed_kass_oracle(2, PROPOSAL);
    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 5_000_000_000);
    let (market, res) = ctx.create_market(&creator, oracle, kass, creator_ata, MIN_LIQ);
    assert!(res.is_ok(), "{res:?}");

    let refs = ctx.compose_metadao_market(market, oracle, kass);
    let res = ctx.activate(oracle, kass);
    assert!(res.is_ok(), "activate: {res:?}");
    (ctx, kass, market, oracle, refs)
}

/// Read a little-endian `u32` from the `Question` account at `off`.
pub(crate) fn question_u32(ctx: &TestCtx, question: Pubkey, off: usize) -> u32 {
    let acc = ctx.svm.get_account(&question).expect("question exists");
    u32::from_le_bytes(acc.data[off..off + 4].try_into().unwrap())
}

/// Create a test user who holds ONLY cYES: split `amount` KASS into cYES+cNO, then
/// transfer the cNO leg out to a sink so the user's cNO balance is 0. Returns the
/// user keypair and its (kass_ata, cyes, cno) accounts.
pub(crate) fn holder_of_yes_only(
    ctx: &mut TestCtx,
    kass: Pubkey,
    refs: &MetaDaoRefs,
    amount: u64,
) -> (Keypair, Pubkey, Pubkey, Pubkey) {
    let user = Keypair::new();
    ctx.svm_airdrop(&user.pubkey());
    let user_kass = ctx.create_token_account(kass, user.pubkey(), amount);
    let user_cyes = ctx.create_token_account(refs.yes_mint, user.pubkey(), 0);
    let user_cno = ctx.create_token_account(refs.no_mint, user.pubkey(), 0);
    let res = ctx.user_split(&user, refs, user_kass, user_cyes, user_cno, amount);
    assert!(res.is_ok(), "user_split: {res:?}");
    assert_eq!(ctx.token_balance(user_cyes), amount, "split minted cYES");
    assert_eq!(ctx.token_balance(user_cno), amount, "split minted cNO");

    // Drain the cNO leg to a sink so the user holds only cYES.
    let sink = ctx.create_token_account(refs.no_mint, Pubkey::new_unique(), 0);
    let ix = spl_token::instruction::transfer(
        &spl_token::ID,
        &user_cno,
        &sink,
        &user.pubkey(),
        &[],
        amount,
    )
    .unwrap();
    let res = ctx.send(ix, &[&user]);
    assert!(res.is_ok(), "drain cNO: {res:?}");
    assert_eq!(ctx.token_balance(user_cno), 0, "cNO drained");
    (user, user_kass, user_cyes, user_cno)
}

/// Mirror of [`holder_of_yes_only`] for the LOSING leg: split `amount` KASS then
/// drain the cYES leg out, so the user holds ONLY cNO. Returns the user keypair
/// and its (kass_ata, cyes, cno) accounts.
pub(crate) fn holder_of_no_only(
    ctx: &mut TestCtx,
    kass: Pubkey,
    refs: &MetaDaoRefs,
    amount: u64,
) -> (Keypair, Pubkey, Pubkey, Pubkey) {
    let user = Keypair::new();
    ctx.svm_airdrop(&user.pubkey());
    let user_kass = ctx.create_token_account(kass, user.pubkey(), amount);
    let user_cyes = ctx.create_token_account(refs.yes_mint, user.pubkey(), 0);
    let user_cno = ctx.create_token_account(refs.no_mint, user.pubkey(), 0);
    let res = ctx.user_split(&user, refs, user_kass, user_cyes, user_cno, amount);
    assert!(res.is_ok(), "user_split: {res:?}");

    // Drain the cYES leg to a sink so the user holds only cNO.
    let sink = ctx.create_token_account(refs.yes_mint, Pubkey::new_unique(), 0);
    let ix = spl_token::instruction::transfer(
        &spl_token::ID,
        &user_cyes,
        &sink,
        &user.pubkey(),
        &[],
        amount,
    )
    .unwrap();
    let res = ctx.send(ix, &[&user]);
    assert!(res.is_ok(), "drain cYES: {res:?}");
    assert_eq!(ctx.token_balance(user_cyes), 0, "cYES drained");
    (user, user_kass, user_cyes, user_cno)
}

// ---------------------------------------------------------------------------
// Categorical (N > 2) sub-markets: each binds to (oracle, outcome_index) and
// resolves YES iff the oracle resolves to that outcome.
// ---------------------------------------------------------------------------

/// Base context for a 3-option categorical oracle: SVM + MetaDAO loaded, KASS
/// mint, config, and a 3-option oracle in Proposal. Returns (ctx, kass, oracle).
pub(crate) fn setup_categorical() -> (TestCtx, Pubkey, Pubkey) {
    let mut ctx = TestCtx::new();
    ctx.load_metadao();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    let (_cfg, res) = ctx.init_config(authority.pubkey(), kass, MIN_LIQ);
    assert!(res.is_ok(), "{res:?}");
    let oracle = ctx.seed_kass_oracle(3, PROPOSAL);
    (ctx, kass, oracle)
}

/// Fund + compose + `activate` the `outcome_index` sub-market on `oracle`.
/// Returns its market PDA + MetaDAO refs.
pub(crate) fn activate_sub_market(
    ctx: &mut TestCtx,
    kass: Pubkey,
    oracle: Pubkey,
    outcome_index: u8,
) -> (Pubkey, MetaDaoRefs) {
    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 5_000_000_000);
    let (market, res) =
        ctx.create_market_full(&creator, oracle, kass, creator_ata, MIN_LIQ, outcome_index);
    assert!(res.is_ok(), "create sub-market {outcome_index}: {res:?}");
    let refs = ctx.compose_metadao_market(market, oracle, kass);
    let res = ctx.activate_at(oracle, kass, outcome_index);
    assert!(res.is_ok(), "activate sub-market {outcome_index}: {res:?}");
    (market, refs)
}

//! Integration tests for `resolve_market` (Ix 8): bridge a terminal Kassandra
//! oracle result into the market's MetaDAO `resolve_question`, then let users
//! redeem their winning conditional tokens.
//!
//! Drives the REAL deployed MetaDAO v0.4 `conditional_vault` + `amm` binaries in
//! LiteSVM (via `ctx.load_metadao()`), reusing the full `activate` flow.

mod common;
use common::*;
use kassandra_market_program::error::MarketError;
use kassandra_market_program::state::{Market, MarketStatus};
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

const PROPOSAL: u8 = 1; // kassandra Phase::Proposal (non-terminal)
const INVALID_DEADEND: u8 = 8; // kassandra Phase::InvalidDeadend (terminal void)
const MIN_LIQ: u64 = 1_000_000_000; // 1 KASS (9 dp)
const SPLIT_AMT: u64 = 2_000_000_000; // 2 KASS a test user splits for redemption

/// `Question` field byte offsets (after the 8-byte Anchor disc): the two binary
/// payout numerators and the payout denominator (`is_resolved ⇔ denominator != 0`).
const Q_NUM0_OFFSET: usize = 76;
const Q_NUM1_OFFSET: usize = 80;
const Q_DENOMINATOR_OFFSET: usize = 84;

/// Stand up an ACTIVE market: fund to `MIN_LIQ`, compose the MetaDAO market, and
/// `activate`. Returns the context, KASS mint, market PDA, oracle, MetaDAO refs.
fn setup_active() -> (TestCtx, Pubkey, Pubkey, Pubkey, MetaDaoRefs) {
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
fn question_u32(ctx: &TestCtx, question: Pubkey, off: usize) -> u32 {
    let acc = ctx.svm.get_account(&question).expect("question exists");
    u32::from_le_bytes(acc.data[off..off + 4].try_into().unwrap())
}

/// Create a test user who holds ONLY cYES: split `amount` KASS into cYES+cNO, then
/// transfer the cNO leg out to a sink so the user's cNO balance is 0. Returns the
/// user keypair and its (kass_ata, cyes, cno) accounts.
fn holder_of_yes_only(
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
fn holder_of_no_only(
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

#[test]
fn resolve_yes_wins_and_redeems() {
    let (mut ctx, kass, market, oracle, refs) = setup_active();

    // One user holds only the WINNING cYES, another holds only the LOSING cNO.
    let (winner, win_kass, win_cyes, win_cno) =
        holder_of_yes_only(&mut ctx, kass, &refs, SPLIT_AMT);
    let (loser, lose_kass, lose_cyes, lose_cno) =
        holder_of_no_only(&mut ctx, kass, &refs, SPLIT_AMT);

    // Oracle resolves YES (option 0) → numerators [1,0].
    ctx.set_oracle_resolved(oracle, 0);
    let res = ctx.resolve_market(market, oracle, refs.question);
    assert!(res.is_ok(), "resolve_market: {res:?}");

    let m: Market = ctx.read_pod(market);
    assert_eq!(m.settled, 1, "settled flag set");
    assert_eq!(
        m.status,
        MarketStatus::Resolved.as_u8(),
        "status → Resolved"
    );
    // Default config carries fee_bps == 100 (> 0) with a live LP position, so the
    // collect_fee crank must still run: fee_collected stays 0 after resolve.
    assert_eq!(m.fee_collected, 0, "fee crank still pending (fee_bps > 0)");

    // Question resolved: denominator != 0, numerators [1,0].
    assert_ne!(
        question_u32(&ctx, refs.question, Q_DENOMINATOR_OFFSET),
        0,
        "resolved"
    );
    assert_eq!(
        question_u32(&ctx, refs.question, Q_NUM0_OFFSET),
        1,
        "num0 == 1"
    );
    assert_eq!(
        question_u32(&ctx, refs.question, Q_NUM1_OFFSET),
        0,
        "num1 == 0"
    );

    // The cYES holder redeems and receives the FULL split amount 1:1.
    let res = ctx.redeem(&winner, &refs, win_kass, win_cyes, win_cno);
    assert!(res.is_ok(), "winner redeem: {res:?}");
    assert_eq!(ctx.token_balance(win_kass), SPLIT_AMT, "cYES paid out 1:1");
    assert_eq!(ctx.token_balance(win_cyes), 0, "cYES burned");

    // The cNO (losing leg) holder redeems and receives NOTHING.
    let res = ctx.redeem(&loser, &refs, lose_kass, lose_cyes, lose_cno);
    assert!(res.is_ok(), "loser redeem: {res:?}");
    assert_eq!(ctx.token_balance(lose_kass), 0, "losing cNO pays 0");
    assert_eq!(ctx.token_balance(lose_cno), 0, "cNO burned");
}

#[test]
fn resolve_no_wins() {
    let (mut ctx, _kass, market, oracle, refs) = setup_active();

    // Oracle resolves NO (option 1) → numerators [0,1].
    ctx.set_oracle_resolved(oracle, 1);
    let res = ctx.resolve_market(market, oracle, refs.question);
    assert!(res.is_ok(), "resolve_market: {res:?}");

    let m: Market = ctx.read_pod(market);
    assert_eq!(m.settled, 1);
    assert_eq!(
        m.status,
        MarketStatus::Resolved.as_u8(),
        "status → Resolved"
    );
    assert_ne!(
        question_u32(&ctx, refs.question, Q_DENOMINATOR_OFFSET),
        0,
        "resolved"
    );
    assert_eq!(
        question_u32(&ctx, refs.question, Q_NUM0_OFFSET),
        0,
        "num0 == 0"
    );
    assert_eq!(
        question_u32(&ctx, refs.question, Q_NUM1_OFFSET),
        1,
        "num1 == 1"
    );
}

#[test]
fn resolve_void_pays_half() {
    let (mut ctx, kass, market, oracle, refs) = setup_active();

    // A user holds only cYES going into a VOID resolution.
    let (user, user_kass, user_cyes, user_cno) =
        holder_of_yes_only(&mut ctx, kass, &refs, SPLIT_AMT);

    // Oracle hits InvalidDeadend → void, numerators [1,1], denominator 2.
    ctx.set_oracle_phase(oracle, INVALID_DEADEND);
    let res = ctx.resolve_market(market, oracle, refs.question);
    assert!(res.is_ok(), "resolve_market: {res:?}");

    let m: Market = ctx.read_pod(market);
    assert_eq!(m.settled, 1);
    assert_eq!(m.status, MarketStatus::Void.as_u8(), "status → Void");
    assert_eq!(
        question_u32(&ctx, refs.question, Q_DENOMINATOR_OFFSET),
        2,
        "denominator == 2"
    );
    assert_eq!(
        question_u32(&ctx, refs.question, Q_NUM0_OFFSET),
        1,
        "num0 == 1"
    );
    assert_eq!(
        question_u32(&ctx, refs.question, Q_NUM1_OFFSET),
        1,
        "num1 == 1"
    );

    // A single-leg (cYES-only) holder redeems for HALF the split value.
    let res = ctx.redeem(&user, &refs, user_kass, user_cyes, user_cno);
    assert!(res.is_ok(), "redeem: {res:?}");
    assert_eq!(
        ctx.token_balance(user_kass),
        SPLIT_AMT / 2,
        "void pays half"
    );
}

#[test]
fn resolve_fee_free_market_stamps_fee_collected() {
    // A fee_bps == 0 market has nothing to collect, so resolve_market stamps
    // fee_collected == 1 directly (the collect_fee crank is not required, and
    // claim_lp opens as soon as the market resolves).
    let mut ctx = TestCtx::new();
    ctx.load_metadao();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    let fee_dest = ctx.create_token_account(kass, authority.pubkey(), 0);
    let (_cfg, res) = ctx.init_config_full(authority.pubkey(), kass, MIN_LIQ, 0, fee_dest);
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
    assert_eq!(
        ctx.read_pod::<Market>(market).fee_bps,
        0,
        "fee_bps snapshot == 0"
    );

    ctx.set_oracle_resolved(oracle, 0);
    let res = ctx.resolve_market(market, oracle, refs.question);
    assert!(res.is_ok(), "resolve: {res:?}");

    let m: Market = ctx.read_pod(market);
    assert_eq!(
        m.status,
        MarketStatus::Resolved.as_u8(),
        "status → Resolved"
    );
    assert_eq!(
        m.fee_collected, 1,
        "fee-free market stamps fee_collected at resolve"
    );
}

#[test]
fn resolve_rejects_unexpected_resolved_option() {
    let (mut ctx, _kass, market, oracle, refs) = setup_active();
    // A binary market only knows options 0/1; a Resolved oracle reporting option 2
    // is unexpected and must be rejected rather than silently mis-resolved.
    ctx.set_oracle_resolved(oracle, 2);
    let res = ctx.resolve_market(market, oracle, refs.question);
    assert_eq!(custom_code(&res), Some(MarketError::InvalidAccount as u32));
}

#[test]
fn resolve_rejects_non_terminal_oracle() {
    let (mut ctx, _kass, market, oracle, refs) = setup_active();
    // Oracle is still in Proposal (non-terminal) from setup.
    let res = ctx.resolve_market(market, oracle, refs.question);
    assert_eq!(
        custom_code(&res),
        Some(MarketError::OracleNotTerminal as u32)
    );
}

#[test]
fn resolve_is_idempotent() {
    let (mut ctx, _kass, market, oracle, refs) = setup_active();
    ctx.set_oracle_resolved(oracle, 0);
    let res = ctx.resolve_market(market, oracle, refs.question);
    assert!(res.is_ok(), "first resolve: {res:?}");
    // Second resolve: market.settled == 1 → AlreadySettled.
    let res = ctx.resolve_market(market, oracle, refs.question);
    assert_eq!(custom_code(&res), Some(MarketError::AlreadySettled as u32));
}

// ---------------------------------------------------------------------------
// Categorical (N > 2) sub-markets: each binds to (oracle, outcome_index) and
// resolves YES iff the oracle resolves to that outcome.
// ---------------------------------------------------------------------------

/// Base context for a 3-option categorical oracle: SVM + MetaDAO loaded, KASS
/// mint, config, and a 3-option oracle in Proposal. Returns (ctx, kass, oracle).
fn setup_categorical() -> (TestCtx, Pubkey, Pubkey) {
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
fn activate_sub_market(
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

#[test]
fn resolve_categorical_matching_outcome_pays_yes() {
    // A 3-option oracle resolves to option 1; the outcome_index=1 sub-market's YES
    // wins → numerators [1,0].
    let (mut ctx, kass, oracle) = setup_categorical();
    let (market, refs) = activate_sub_market(&mut ctx, kass, oracle, 1);

    ctx.set_oracle_resolved_full(oracle, 3, 1);
    let res = ctx.resolve_market(market, oracle, refs.question);
    assert!(res.is_ok(), "resolve: {res:?}");

    let m: Market = ctx.read_pod(market);
    assert_eq!(m.settled, 1);
    assert_eq!(
        m.status,
        MarketStatus::Resolved.as_u8(),
        "status → Resolved"
    );
    assert_ne!(
        question_u32(&ctx, refs.question, Q_DENOMINATOR_OFFSET),
        0,
        "resolved"
    );
    assert_eq!(
        question_u32(&ctx, refs.question, Q_NUM0_OFFSET),
        1,
        "YES pays: num0 == 1"
    );
    assert_eq!(
        question_u32(&ctx, refs.question, Q_NUM1_OFFSET),
        0,
        "num1 == 0"
    );
}

#[test]
fn resolve_categorical_nonmatching_outcomes_pay_no() {
    // The same 3-option oracle resolves to option 1; the outcome_index=0 and =2
    // sub-markets both LOSE → numerators [0,1].
    let (mut ctx, kass, oracle) = setup_categorical();
    let (market0, refs0) = activate_sub_market(&mut ctx, kass, oracle, 0);
    let (market2, refs2) = activate_sub_market(&mut ctx, kass, oracle, 2);

    ctx.set_oracle_resolved_full(oracle, 3, 1);

    for (market, refs) in [(market0, &refs0), (market2, &refs2)] {
        let res = ctx.resolve_market(market, oracle, refs.question);
        assert!(res.is_ok(), "resolve: {res:?}");
        let m: Market = ctx.read_pod(market);
        assert_eq!(
            m.status,
            MarketStatus::Resolved.as_u8(),
            "status → Resolved"
        );
        assert_eq!(
            question_u32(&ctx, refs.question, Q_NUM0_OFFSET),
            0,
            "NO wins: num0 == 0"
        );
        assert_eq!(
            question_u32(&ctx, refs.question, Q_NUM1_OFFSET),
            1,
            "num1 == 1"
        );
    }
}

#[test]
fn resolve_categorical_void_pays_half() {
    // A categorical sub-market voids on InvalidDeadend just like a binary one →
    // numerators [1,1], denominator 2.
    let (mut ctx, kass, oracle) = setup_categorical();
    let (market, refs) = activate_sub_market(&mut ctx, kass, oracle, 2);

    ctx.set_oracle_phase(oracle, INVALID_DEADEND);
    let res = ctx.resolve_market(market, oracle, refs.question);
    assert!(res.is_ok(), "resolve: {res:?}");

    let m: Market = ctx.read_pod(market);
    assert_eq!(m.status, MarketStatus::Void.as_u8(), "status → Void");
    assert_eq!(
        question_u32(&ctx, refs.question, Q_DENOMINATOR_OFFSET),
        2,
        "denominator == 2"
    );
    assert_eq!(
        question_u32(&ctx, refs.question, Q_NUM0_OFFSET),
        1,
        "num0 == 1"
    );
    assert_eq!(
        question_u32(&ctx, refs.question, Q_NUM1_OFFSET),
        1,
        "num1 == 1"
    );
}

#[test]
fn resolve_rejects_non_active_market() {
    // A never-activated (Funding) market has no Question to resolve.
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

    // question is irrelevant here — the NotActive guard fires before any binding.
    let res = ctx.resolve_market(market, oracle, Pubkey::new_unique());
    assert_eq!(custom_code(&res), Some(MarketError::NotActive as u32));
}

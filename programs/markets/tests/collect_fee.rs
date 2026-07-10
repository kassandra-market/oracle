//! Integration tests for `collect_fee` (Ix 9): the permissionless crank that cuts
//! the protocol `fee_bps` share of a resolved market's **accrued** LP earnings and
//! routes it (as KASS) to `Config.fee_destination`, via program-signed
//! `amm::remove_liquidity` → `conditional_vault::redeem_tokens` → SPL `transfer`.
//!
//! Drives the REAL deployed MetaDAO v0.4 `conditional_vault` + `amm` binaries in
//! LiteSVM (via `ctx.load_metadao()`) through the full lifecycle, and grows the
//! pool with a REAL `amm::swap` so the accrued value (and therefore the fee) is
//! genuine — computed from the on-chain reserves, not fabricated.

mod common;
use common::*;
use kassandra_markets_program::error::MarketError;
use kassandra_markets_program::state::{Market, MarketStatus};
use kassandra_markets_sdk::metadao::SwapType;
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

const PROPOSAL: u8 = 1; // kassandra Phase::Proposal (non-terminal)
const INVALID_DEADEND: u8 = 8; // kassandra Phase::InvalidDeadend (terminal void)
const MIN_LIQ: u64 = 1_000_000_000; // 1 KASS (9 dp) — the seeded pool depth
const SWAP_KASS: u64 = 3_000_000_000; // KASS the swapper splits for a trading position
const SWAP_IN: u64 = 1_500_000_000; // cYES sold into the pool to move price + accrue fees

/// Account byte offsets (absolute, incl. any 8-byte Anchor disc).
const Q_NUM0: usize = 76;
const Q_NUM1: usize = 80;
const Q_DENOM: usize = 84;
const AMM_BASE: usize = 115;
const AMM_QUOTE: usize = 123;
const MINT_SUPPLY: usize = 36;

fn read_u32_at(ctx: &TestCtx, key: Pubkey, off: usize) -> u128 {
    let acc = ctx.svm.get_account(&key).expect("account exists");
    u32::from_le_bytes(acc.data[off..off + 4].try_into().unwrap()) as u128
}
fn read_u64_at(ctx: &TestCtx, key: Pubkey, off: usize) -> u128 {
    let acc = ctx.svm.get_account(&key).expect("account exists");
    u64::from_le_bytes(acc.data[off..off + 8].try_into().unwrap()) as u128
}

/// Everything a collect_fee test needs after fund → activate.
struct Active {
    ctx: TestCtx,
    kass: Pubkey,
    market: Pubkey,
    oracle: Pubkey,
    refs: MetaDaoRefs,
    fee_dest: Pubkey,
    creator: Keypair,
}

/// Fund a single-contributor market to `MIN_LIQ`, compose the MetaDAO market, and
/// `activate`, with an explicit `fee_bps`. Returns the live `Active` handle.
fn setup_active(fee_bps: u16) -> Active {
    let mut ctx = TestCtx::new();
    ctx.load_metadao();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    let fee_dest = ctx.create_token_account(kass, authority.pubkey(), 0);
    let (_cfg, res) = ctx.init_config_full(authority.pubkey(), kass, MIN_LIQ, fee_bps, fee_dest);
    assert!(res.is_ok(), "init_config: {res:?}");

    let oracle = ctx.seed_kass_oracle(2, PROPOSAL);
    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 10_000_000_000);
    let (market, res) = ctx.create_market(&creator, oracle, kass, creator_ata, MIN_LIQ);
    assert!(res.is_ok(), "create_market: {res:?}");

    let refs = ctx.compose_metadao_market(market, oracle, kass);
    let res = ctx.activate(oracle, kass);
    assert!(res.is_ok(), "activate: {res:?}");
    assert_eq!(
        ctx.read_pod::<Market>(market).fee_bps,
        fee_bps,
        "fee_bps snapshot"
    );

    Active {
        ctx,
        kass,
        market,
        oracle,
        refs,
        fee_dest,
        creator,
    }
}

/// Grow the pool with a REAL swap: a fresh user splits `SWAP_KASS` KASS into
/// cYES+cNO, then SELLS `SWAP_IN` cYES into the pool (cYES in, cNO out). The swap
/// fee accrues to the reserves and the pool ends up cYES-heavy, so a subsequent
/// YES resolution realizes genuine LP earnings.
fn grow_pool_sell_cyes(a: &mut Active) {
    let user = Keypair::new();
    a.ctx.svm_airdrop(&user.pubkey());
    let u_kass = a.ctx.create_token_account(a.kass, user.pubkey(), SWAP_KASS);
    let u_cyes = a
        .ctx
        .create_token_account(a.refs.yes_mint, user.pubkey(), 0);
    let u_cno = a.ctx.create_token_account(a.refs.no_mint, user.pubkey(), 0);
    let res = a
        .ctx
        .user_split(&user, &a.refs, u_kass, u_cyes, u_cno, SWAP_KASS);
    assert!(res.is_ok(), "swapper split: {res:?}");

    let res = a
        .ctx
        .user_swap(&user, &a.refs, u_cyes, u_cno, SwapType::Sell, SWAP_IN, 0);
    assert!(res.is_ok(), "swap (sell cYES): {res:?}");
}

/// Analytically recompute the on-chain fee math from the resolved `Question` +
/// `Amm` reserves + LP-mint supply. Returns `(fee_lp, expected_fee_kass)` where
/// `expected_fee_kass` is the double-floored KASS the crank should deliver.
fn expected_fee(a: &Active, fee_bps: u128) -> (u64, u128) {
    let num0 = read_u32_at(&a.ctx, a.refs.question, Q_NUM0);
    let num1 = read_u32_at(&a.ctx, a.refs.question, Q_NUM1);
    let denom = read_u32_at(&a.ctx, a.refs.question, Q_DENOM);
    assert!(denom > 0, "question resolved");
    let base = read_u64_at(&a.ctx, a.refs.amm, AMM_BASE);
    let quote = read_u64_at(&a.ctx, a.refs.amm, AMM_QUOTE);
    let supply = read_u64_at(&a.ctx, a.refs.lp_mint, MINT_SUPPLY);
    assert!(supply > 0, "lp supply > 0");

    let m: Market = a.ctx.read_pod(a.market);
    let lp_total = m.lp_total as u128;
    let total_contributed = m.total_contributed as u128;

    let pool_value = (base * num0 + quote * num1) / denom;
    let realized_full = lp_total * pool_value / supply;
    let accrued = realized_full.saturating_sub(total_contributed);
    let fee_lp = if accrued == 0 {
        0
    } else {
        let accrued_lp = lp_total * accrued / realized_full;
        accrued_lp * fee_bps / 10_000
    };
    // Double-floor delivered KASS: remove_liquidity floors each reserve share,
    // redeem_tokens floors the payout.
    let base_out = base * fee_lp / supply;
    let quote_out = quote * fee_lp / supply;
    let expected_kass = (base_out * num0 + quote_out * num1) / denom;
    (u64::try_from(fee_lp).unwrap(), expected_kass)
}

#[test]
fn collect_fee_happy_cuts_accrued_and_then_claim_pays_reduced() {
    let mut a = setup_active(100); // 1% protocol fee
    grow_pool_sell_cyes(&mut a);

    // Resolve YES (option 0) → pool is cYES-heavy → the LP realized real earnings.
    a.ctx.set_oracle_resolved(a.oracle, 0);
    let res = a.ctx.resolve_market(a.market, a.oracle, a.refs.question);
    assert!(res.is_ok(), "resolve: {res:?}");
    let m: Market = a.ctx.read_pod(a.market);
    assert_eq!(m.status, MarketStatus::Resolved.as_u8());
    assert_eq!(m.fee_collected, 0, "crank still pending");

    // Ordering guard: claim_lp BEFORE collect_fee is rejected.
    let early_ata = a
        .ctx
        .create_token_account(a.refs.lp_mint, a.creator.pubkey(), 0);
    let res = a.ctx.claim_lp(a.market, a.creator.pubkey(), early_ata);
    assert_eq!(
        custom_code(&res),
        Some(MarketError::FeeNotCollected as u32),
        "claim before collect rejected"
    );

    // Recompute the expected fee from the on-chain state, then run the crank.
    let (fee_lp, expected_kass) = expected_fee(&a, 100);
    assert!(
        fee_lp > 0,
        "a real swap accrued a real fee: fee_lp {fee_lp}"
    );
    assert!(expected_kass > 0, "expected fee KASS > 0");
    let lp_total_before = a.ctx.read_pod::<Market>(a.market).lp_total;

    let res = a.ctx.collect_fee(a.oracle, a.kass, a.fee_dest);
    assert!(res.is_ok(), "collect_fee: {res:?}");

    // fee_destination received ≈ the analytically-expected KASS (delivered ≤ the
    // idealized value due to floor rounding; within a few base units of it).
    let got = a.ctx.token_balance(a.fee_dest) as i128;
    assert!(got > 0, "fee destination funded");
    assert!(
        (got - expected_kass as i128).abs() <= 4,
        "delivered {got} ≈ expected {expected_kass}"
    );

    // lp_total reduced by EXACTLY fee_lp; flag stamped.
    let m: Market = a.ctx.read_pod(a.market);
    assert_eq!(
        m.lp_total,
        lp_total_before - fee_lp,
        "lp_total cut by fee_lp"
    );
    assert_eq!(m.fee_collected, 1, "fee_collected stamped");
    let lp_total_after = m.lp_total;

    // Second collect_fee is idempotent-rejected.
    let res = a.ctx.collect_fee(a.oracle, a.kass, a.fee_dest);
    assert_eq!(custom_code(&res), Some(MarketError::AlreadySettled as u32));

    // claim_lp now pays pro-rata off the REDUCED lp_total. Single contributor
    // (creator staked all MIN_LIQ) → gets the whole post-fee lp_vault (minus dust).
    let (lp_vault, _) = kassandra_markets_sdk::pda::lp_vault(&a.market);
    let claim_ata = a
        .ctx
        .create_token_account(a.refs.lp_mint, a.creator.pubkey(), 0);
    let res = a.ctx.claim_lp(a.market, a.creator.pubkey(), claim_ata);
    assert!(res.is_ok(), "post-collect claim: {res:?}");
    // Sole contributor (staked all of MIN_LIQ) → pro-rata share is the whole
    // post-fee lp_total; the fee slice already left the vault via the crank.
    let claimed = a.ctx.token_balance(claim_ata);
    assert_eq!(
        claimed, lp_total_after,
        "claim pays pro-rata off reduced lp_total"
    );
    assert!(
        a.ctx.token_balance(lp_vault) < 2,
        "lp_vault drained to dust"
    );
}

#[test]
fn collect_fee_rejects_before_resolution() {
    // An Active (not yet resolved) market cannot be collected — status guard.
    let mut a = setup_active(100);
    let res = a.ctx.collect_fee(a.oracle, a.kass, a.fee_dest);
    assert_eq!(custom_code(&res), Some(MarketError::NotActive as u32));
}

#[test]
fn collect_fee_rejects_fee_free_market() {
    // fee_bps == 0 → resolve_market stamps fee_collected == 1 directly, so the
    // crank has nothing to do and rejects; claim_lp works without collecting.
    let mut a = setup_active(0);
    grow_pool_sell_cyes(&mut a); // even with accrual, a 0% fee collects nothing
    a.ctx.set_oracle_resolved(a.oracle, 0);
    let res = a.ctx.resolve_market(a.market, a.oracle, a.refs.question);
    assert!(res.is_ok(), "resolve: {res:?}");
    assert_eq!(
        a.ctx.read_pod::<Market>(a.market).fee_collected,
        1,
        "fee-free market stamped at resolve"
    );

    let res = a.ctx.collect_fee(a.oracle, a.kass, a.fee_dest);
    assert_eq!(custom_code(&res), Some(MarketError::AlreadySettled as u32));
    assert_eq!(a.ctx.token_balance(a.fee_dest), 0, "no fee taken");

    // claim_lp works directly (fee_collected already 1).
    let claim_ata = a
        .ctx
        .create_token_account(a.refs.lp_mint, a.creator.pubkey(), 0);
    let res = a.ctx.claim_lp(a.market, a.creator.pubkey(), claim_ata);
    assert!(res.is_ok(), "claim on fee-free market: {res:?}");
    assert!(a.ctx.token_balance(claim_ata) > 0, "creator claimed LP");
}

#[test]
fn collect_fee_no_accrual_takes_nothing_but_stamps_flag() {
    // No swap → the pool never earned → realized ≈ contributed → accrued == 0
    // (impermanent-loss / no-profit case). The crank takes no fee but stamps the
    // flag so claim_lp opens.
    let mut a = setup_active(100);
    a.ctx.set_oracle_resolved(a.oracle, 0);
    let res = a.ctx.resolve_market(a.market, a.oracle, a.refs.question);
    assert!(res.is_ok(), "resolve: {res:?}");

    let (fee_lp, _) = expected_fee(&a, 100);
    assert_eq!(fee_lp, 0, "no accrual → fee_lp == 0");
    let lp_total_before = a.ctx.read_pod::<Market>(a.market).lp_total;

    let res = a.ctx.collect_fee(a.oracle, a.kass, a.fee_dest);
    assert!(res.is_ok(), "collect_fee (no-op): {res:?}");
    let m: Market = a.ctx.read_pod(a.market);
    assert_eq!(m.fee_collected, 1, "flag stamped");
    assert_eq!(m.lp_total, lp_total_before, "lp_total unchanged (no fee)");
    assert_eq!(a.ctx.token_balance(a.fee_dest), 0, "no KASS moved");

    let claim_ata = a
        .ctx
        .create_token_account(a.refs.lp_mint, a.creator.pubkey(), 0);
    let res = a.ctx.claim_lp(a.market, a.creator.pubkey(), claim_ata);
    assert!(res.is_ok(), "claim after no-op collect: {res:?}");
    assert!(
        a.ctx.token_balance(claim_ata) > 0,
        "creator claimed full LP"
    );
}

#[test]
fn collect_fee_void_path() {
    // A voided market resolves [1,1] (denominator 2). After a swap the pool value
    // (base+quote)/2 exceeds the contributed liquidity (the swap fee grew both the
    // invariant and the reserve sum), so the crank still cuts a positive fee.
    let mut a = setup_active(100);
    grow_pool_sell_cyes(&mut a);

    a.ctx.set_oracle_phase(a.oracle, INVALID_DEADEND);
    let res = a.ctx.resolve_market(a.market, a.oracle, a.refs.question);
    assert!(res.is_ok(), "resolve void: {res:?}");
    let m: Market = a.ctx.read_pod(a.market);
    assert_eq!(m.status, MarketStatus::Void.as_u8(), "status → Void");
    assert_eq!(
        read_u32_at(&a.ctx, a.refs.question, Q_DENOM),
        2,
        "void denominator == 2"
    );

    let (fee_lp, expected_kass) = expected_fee(&a, 100);
    assert!(fee_lp > 0, "void fee_lp > 0 after swap: {fee_lp}");
    let lp_total_before = m.lp_total;

    let res = a.ctx.collect_fee(a.oracle, a.kass, a.fee_dest);
    assert!(res.is_ok(), "collect_fee void: {res:?}");

    let got = a.ctx.token_balance(a.fee_dest) as i128;
    assert!(got > 0, "void fee destination funded");
    assert!(
        (got - expected_kass as i128).abs() <= 4,
        "delivered {got} ≈ expected {expected_kass}"
    );
    let m: Market = a.ctx.read_pod(a.market);
    assert_eq!(
        m.lp_total,
        lp_total_before - fee_lp,
        "lp_total cut by fee_lp"
    );
    assert_eq!(m.fee_collected, 1, "fee_collected stamped");

    // claim_lp opens after collection.
    let claim_ata = a
        .ctx
        .create_token_account(a.refs.lp_mint, a.creator.pubkey(), 0);
    let res = a.ctx.claim_lp(a.market, a.creator.pubkey(), claim_ata);
    assert!(res.is_ok(), "post-void-collect claim: {res:?}");
    assert!(
        a.ctx.token_balance(claim_ata) > 0,
        "creator claimed reduced LP"
    );
}

#[test]
fn collect_fee_rejects_substituted_fee_destination() {
    // FUND-CUSTODY: the crank routes real pool value out, so a cranker must not be
    // able to redirect the fee to an account of their choosing. Passing any
    // `fee_destination != config.fee_destination` (even a valid KASS account) is
    // rejected by the `assert_key` bind → InvalidAccount, before any value moves.
    let mut a = setup_active(100);
    grow_pool_sell_cyes(&mut a);
    a.ctx.set_oracle_resolved(a.oracle, 0);
    let res = a.ctx.resolve_market(a.market, a.oracle, a.refs.question);
    assert!(res.is_ok(), "resolve: {res:?}");

    // A rogue KASS token account owned by an attacker (correct mint, wrong key).
    let attacker = Keypair::new();
    let rogue = a.ctx.create_token_account(a.kass, attacker.pubkey(), 0);
    assert_ne!(rogue, a.fee_dest, "rogue is a different account");
    let res = a.ctx.collect_fee(a.oracle, a.kass, rogue);
    assert_eq!(
        custom_code(&res),
        Some(MarketError::InvalidAccount as u32),
        "substituted fee_destination rejected"
    );
    // Nothing moved: still collectable, nothing in either account.
    assert_eq!(
        a.ctx.read_pod::<Market>(a.market).fee_collected,
        0,
        "not collected"
    );
    assert_eq!(a.ctx.token_balance(rogue), 0, "rogue got nothing");
    assert_eq!(a.ctx.token_balance(a.fee_dest), 0, "real dest got nothing");
}

#[test]
fn collect_fee_multi_contributor_claim_pro_rata_off_reduced_total() {
    // fee_bps > 0 with TWO contributors: after collect_fee cuts the fee slice, each
    // contributor's claim_lp must pay pro-rata off the REDUCED lp_total (not the
    // activation total). Creator seeds A, c2 seeds B, A + B == MIN_LIQ.
    const SEED_A: u64 = 600_000_000;
    const SEED_B: u64 = 400_000_000;

    let mut ctx = TestCtx::new();
    ctx.load_metadao();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    let fee_dest = ctx.create_token_account(kass, authority.pubkey(), 0);
    let (_cfg, res) = ctx.init_config_full(authority.pubkey(), kass, MIN_LIQ, 100, fee_dest);
    assert!(res.is_ok(), "init_config: {res:?}");

    let oracle = ctx.seed_kass_oracle(2, PROPOSAL);
    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 10_000_000_000);
    let (market, res) = ctx.create_market(&creator, oracle, kass, creator_ata, SEED_A);
    assert!(res.is_ok(), "create_market: {res:?}");
    let c2 = Keypair::new();
    ctx.svm_airdrop(&c2.pubkey());
    let c2_ata = ctx.create_token_account(kass, c2.pubkey(), 10_000_000_000);
    let res = ctx.contribute(&c2, market, c2_ata, SEED_B);
    assert!(res.is_ok(), "contribute: {res:?}");

    let refs = ctx.compose_metadao_market(market, oracle, kass);
    let res = ctx.activate(oracle, kass);
    assert!(res.is_ok(), "activate: {res:?}");

    // Grow the pool, resolve YES, collect the fee.
    let mut a = Active {
        ctx,
        kass,
        market,
        oracle,
        refs,
        fee_dest,
        creator,
    };
    grow_pool_sell_cyes(&mut a);
    a.ctx.set_oracle_resolved(a.oracle, 0);
    let res = a.ctx.resolve_market(a.market, a.oracle, a.refs.question);
    assert!(res.is_ok(), "resolve: {res:?}");
    let (fee_lp, _) = expected_fee(&a, 100);
    assert!(fee_lp > 0, "multi-contributor fee_lp > 0");
    let lp_before = a.ctx.read_pod::<Market>(a.market).lp_total;

    let res = a.ctx.collect_fee(a.oracle, a.kass, a.fee_dest);
    assert!(res.is_ok(), "collect_fee: {res:?}");
    let reduced = a.ctx.read_pod::<Market>(a.market).lp_total;
    assert_eq!(reduced, lp_before - fee_lp, "lp_total reduced by fee_lp");
    assert!(
        reduced < lp_before,
        "reduced total strictly below activation total"
    );

    // Each contributor claims pro-rata off the REDUCED lp_total.
    let a_share = u64::try_from((reduced as u128) * (SEED_A as u128) / (MIN_LIQ as u128)).unwrap();
    let b_share = u64::try_from((reduced as u128) * (SEED_B as u128) / (MIN_LIQ as u128)).unwrap();
    assert!(a_share > 0 && b_share > 0, "both shares positive");
    assert!(
        a_share + b_share <= reduced,
        "no over-distribution off reduced total"
    );

    let a_lp = a
        .ctx
        .create_token_account(a.refs.lp_mint, a.creator.pubkey(), 0);
    let res = a.ctx.claim_lp(a.market, a.creator.pubkey(), a_lp);
    assert!(res.is_ok(), "claim A: {res:?}");
    assert_eq!(
        a.ctx.token_balance(a_lp),
        a_share,
        "A pro-rata off reduced total"
    );

    let b_lp = a.ctx.create_token_account(a.refs.lp_mint, c2.pubkey(), 0);
    let res = a.ctx.claim_lp(a.market, c2.pubkey(), b_lp);
    assert!(res.is_ok(), "claim B: {res:?}");
    assert_eq!(
        a.ctx.token_balance(b_lp),
        b_share,
        "B pro-rata off reduced total"
    );
}

#[test]
fn collect_fee_positive_accrual_but_fee_floors_to_zero() {
    // Boundary: a TINY accrual with a tiny fee_bps → `accrued > 0` but
    // `fee_lp = accrued_lp · fee_bps / 10000` floors to 0. The crank must still
    // stamp fee_collected (no transfer, lp_total intact) so claim_lp opens.
    let mut a = setup_active(1); // 0.01% fee — the smallest non-zero cut

    // A minimal swap: selling `SELL` cYES adds exactly `SELL` to the cYES reserve,
    // so a YES resolution realizes ≈ `SELL` base units of accrued LP value (the
    // pool here mints supply == lp_total, no locked minimum). With `SELL` a few
    // thousand and fee_bps == 1, `accrued > 0` while `accrued_lp · 1 / 10000 == 0`.
    const SELL: u64 = 5_000;
    let user = Keypair::new();
    a.ctx.svm_airdrop(&user.pubkey());
    let u_kass = a.ctx.create_token_account(a.kass, user.pubkey(), 1_000_000);
    let u_cyes = a
        .ctx
        .create_token_account(a.refs.yes_mint, user.pubkey(), 0);
    let u_cno = a.ctx.create_token_account(a.refs.no_mint, user.pubkey(), 0);
    let res = a
        .ctx
        .user_split(&user, &a.refs, u_kass, u_cyes, u_cno, 1_000_000);
    assert!(res.is_ok(), "tiny split: {res:?}");
    let res = a
        .ctx
        .user_swap(&user, &a.refs, u_cyes, u_cno, SwapType::Sell, SELL, 0);
    assert!(res.is_ok(), "tiny swap: {res:?}");

    a.ctx.set_oracle_resolved(a.oracle, 0);
    let res = a.ctx.resolve_market(a.market, a.oracle, a.refs.question);
    assert!(res.is_ok(), "resolve: {res:?}");

    // Confirm we are exactly on the boundary: accrued > 0, fee_lp == 0.
    let num0 = read_u32_at(&a.ctx, a.refs.question, Q_NUM0);
    let num1 = read_u32_at(&a.ctx, a.refs.question, Q_NUM1);
    let denom = read_u32_at(&a.ctx, a.refs.question, Q_DENOM);
    let base = read_u64_at(&a.ctx, a.refs.amm, AMM_BASE);
    let quote = read_u64_at(&a.ctx, a.refs.amm, AMM_QUOTE);
    let supply = read_u64_at(&a.ctx, a.refs.lp_mint, MINT_SUPPLY);
    let m: Market = a.ctx.read_pod(a.market);
    let pool_value = (base * num0 + quote * num1) / denom;
    let realized = (m.lp_total as u128) * pool_value / supply;
    let accrued = realized.saturating_sub(m.total_contributed as u128);
    assert!(accrued > 0, "accrual is positive (accrued = {accrued})");
    let (fee_lp, _) = expected_fee(&a, 1);
    assert_eq!(
        fee_lp, 0,
        "fee floors to 0 at the boundary (accrued = {accrued})"
    );

    let lp_before = m.lp_total;
    let res = a.ctx.collect_fee(a.oracle, a.kass, a.fee_dest);
    assert!(res.is_ok(), "collect_fee (floored fee): {res:?}");
    let m: Market = a.ctx.read_pod(a.market);
    assert_eq!(m.fee_collected, 1, "flag stamped despite zero fee");
    assert_eq!(m.lp_total, lp_before, "lp_total intact (no LP removed)");
    assert_eq!(a.ctx.token_balance(a.fee_dest), 0, "no KASS transferred");

    // claim_lp opens.
    let claim_ata = a
        .ctx
        .create_token_account(a.refs.lp_mint, a.creator.pubkey(), 0);
    let res = a.ctx.claim_lp(a.market, a.creator.pubkey(), claim_ata);
    assert!(res.is_ok(), "claim after floored-fee collect: {res:?}");
    assert!(
        a.ctx.token_balance(claim_ata) > 0,
        "creator claimed full LP"
    );
}

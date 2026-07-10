//! Core collect_fee lifecycle: happy path, status guards, and the no-fee /
//! no-accrual / void branches.

use super::*;

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

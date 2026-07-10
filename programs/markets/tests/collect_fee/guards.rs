//! Fund-custody guard + multi-contributor accounting + the fee-floors-to-zero
//! boundary.

use super::*;

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

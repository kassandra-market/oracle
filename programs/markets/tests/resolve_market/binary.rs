//! Binary (2-outcome) resolution: YES/NO/void redemption + the guard rejections.

use super::*;

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

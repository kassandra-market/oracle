//! Categorical (N > 2) sub-market resolution: matching/non-matching outcomes,
//! void, and the non-active guard.

use super::*;

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

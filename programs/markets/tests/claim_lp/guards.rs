//! Rejection guards: not-active, wrong dest owner/mint, cross-market
//! contribution, and the dust-share-closes-contribution branch.

use super::*;

#[test]
fn claim_lp_rejects_not_active() {
    // A still-`Funding` market (never activated) has no LP to claim.
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    let (_cfg, res) = ctx.init_config(authority.pubkey(), kass, MIN_LIQ);
    assert!(res.is_ok(), "{res:?}");
    let oracle = ctx.seed_kass_oracle(2, PROPOSAL);
    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 5_000_000_000);
    let (market, res) = ctx.create_market(&creator, oracle, kass, creator_ata, SEED_A);
    assert!(res.is_ok(), "{res:?}");

    // No lp_mint yet; any token account will do — the status guard fires first.
    let dest = ctx.create_token_account(kass, creator.pubkey(), 0);
    let res = ctx.claim_lp(market, creator.pubkey(), dest);
    assert_eq!(custom_code(&res), Some(MarketError::NotActive as u32));
}

#[test]
fn claim_lp_rejects_wrong_dest_owner() {
    let mut s = setup_active_two_contributors();
    // Dest LP ata owned by a stranger, not the recorded contributor.
    let stranger = Keypair::new();
    let stranger_lp_ata = s.ctx.create_token_account(s.lp_mint, stranger.pubkey(), 0);
    let res = s
        .ctx
        .claim_lp_to(s.market, s.creator.pubkey(), stranger_lp_ata);
    assert_eq!(custom_code(&res), Some(MarketError::InvalidAccount as u32));

    // Stake untouched.
    let (a_contrib, _) = kassandra_markets_sdk::pda::contribution(&s.market, &s.creator.pubkey());
    assert_eq!(s.ctx.read_pod::<Contribution>(a_contrib).claimed, 0);
}

#[test]
fn claim_lp_rejects_wrong_mint() {
    let mut s = setup_active_two_contributors();
    // Dest owned by the contributor but on the WRONG mint (KASS, not lp_mint).
    let wrong_mint_ata = s.ctx.create_token_account(s.kass, s.creator.pubkey(), 0);
    let res = s.ctx.claim_lp(s.market, s.creator.pubkey(), wrong_mint_ata);
    assert_eq!(custom_code(&res), Some(MarketError::InvalidAccount as u32));
}

#[test]
fn claim_lp_rejects_cross_market_contribution() {
    use bytemuck::Zeroable;
    let mut s = setup_active_two_contributors();

    // A well-formed Contribution whose `market` field points at a DIFFERENT
    // market. Passed explicitly, it must be rejected by the cross-market guard
    // (before any token movement).
    let foreign_addr = Pubkey::new_unique();
    let mut c = Contribution::zeroed();
    c.account_type = AccountType::Contribution.as_u8();
    c.market = Pubkey::new_unique().to_bytes().into(); // NOT s.market
    c.contributor = s.creator.pubkey().to_bytes().into();

    c.amount = SEED_A;
    set_pod(&mut s.ctx, foreign_addr, &c);

    let dest = s.ctx.create_token_account(s.lp_mint, s.creator.pubkey(), 0);
    let res = s
        .ctx
        .claim_lp_with_contribution(s.market, foreign_addr, dest);
    assert_eq!(custom_code(&res), Some(MarketError::InvalidAccount as u32));
}

#[test]
fn claim_lp_dust_share_zero_still_closes_contribution() {
    use bytemuck::Zeroable;
    // The floor pro-rata share can round to 0 for a dust contribution (when
    // `lp_total * amount < total_contributed`). The processor must skip the
    // transfer but STILL close the contribution, so a dust contributor cannot
    // wedge in a retry loop. `open_contributions == 2` keeps this on the floor
    // pro-rata branch (NOT the last-claimer sweep). In the real flow
    // `lp_total == total_contributed` so this never triggers; we fabricate an
    // inconsistent state to cover the branch.
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let lp_mint = ctx.create_mint(9);
    let oracle = Pubkey::new_unique();
    let (market, mbump) = kassandra_markets_sdk::pda::market(&oracle, 0);
    let contributor = Keypair::new();
    let (contribution, cbump) =
        kassandra_markets_sdk::pda::contribution(&market, &contributor.pubkey());

    // A market-PDA-owned LP token account holding a tiny LP total, placed at the
    // canonical `lp_vault` PDA (the harness `claim_lp` derives that address).
    let (lp_vault, _) = kassandra_markets_sdk::pda::lp_vault(&market);
    set_token_at(&mut ctx, lp_vault, lp_mint, market, 1);

    let mut m = Market::zeroed();
    m.account_type = AccountType::Market.as_u8();
    m.oracle = oracle.to_bytes().into();

    m.kass_mint = kass.to_bytes().into();

    m.status = MarketStatus::Active.as_u8();
    m.bump = mbump;
    m.lp_mint = lp_mint.to_bytes().into();

    m.lp_vault = lp_vault.to_bytes().into();

    m.lp_total = 1;
    m.total_contributed = 1_000_000; // floor(1 * 1 / 1_000_000) == 0
    m.fee_collected = 1; // fee gate satisfied (this fabricated market skips collect_fee)
    m.open_contributions = 2; // > 1 → floor pro-rata branch (not the last-claimer sweep)
    set_pod(&mut ctx, market, &m);

    let mut c = Contribution::zeroed();
    c.account_type = AccountType::Contribution.as_u8();
    c.market = market.to_bytes().into();

    c.contributor = contributor.pubkey().to_bytes().into();

    c.amount = 1;
    c.bump = cbump;
    set_pod(&mut ctx, contribution, &c);

    let contrib_rent = ctx.lamports(contribution);
    assert!(contrib_rent > 0, "Contribution funded pre-claim");
    let dest = ctx.create_token_account(lp_mint, contributor.pubkey(), 0);
    let res = ctx.claim_lp(market, contributor.pubkey(), dest);
    assert!(res.is_ok(), "dust claim ok: {res:?}");
    assert_eq!(
        ctx.token_balance(dest),
        0,
        "no LP transferred (share floored to 0)"
    );
    assert_eq!(ctx.token_balance(lp_vault), 1, "lp_vault untouched");
    // The Contribution is CLOSED (reaped) even though no LP moved — no retry wedge.
    assert_eq!(ctx.lamports(contribution), 0, "Contribution closed");
    assert_eq!(
        ctx.lamports(contributor.pubkey()),
        contrib_rent,
        "Contribution rent returned to contributor"
    );
    assert_eq!(
        ctx.read_pod::<Market>(market).open_contributions,
        1,
        "counter decremented (2 → 1)"
    );
}

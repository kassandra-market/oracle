//! Happy-path claims: pro-rata two-contributor sweep, idempotency after close,
//! post-resolution survival, and mutual exclusion with refund.

use super::*;

#[test]
fn claim_lp_happy_two_contributors_pro_rata() {
    let mut s = setup_active_two_contributors();
    let (lp_vault, _) = kassandra_markets_sdk::pda::lp_vault(&s.market);
    assert_eq!(s.ctx.token_balance(lp_vault), s.lp_total, "lp_vault seeded");

    let a_share = expected_share(s.lp_total, SEED_A, s.total_contributed);
    assert!(a_share > 0, "A share nonzero");

    // Contributor A (creator) claims first — NOT the last claimer (counter 2 → 1),
    // so takes the floor pro-rata share. Their Contribution is then closed and its
    // rent returned to the creator.
    let a_contrib_rent = s
        .ctx
        .lamports(kassandra_markets_sdk::pda::contribution(&s.market, &s.creator.pubkey()).0);
    assert!(a_contrib_rent > 0, "A Contribution funded pre-claim");
    let a_wallet_before = s.ctx.lamports(s.creator.pubkey());
    let a_lp_ata = s.ctx.create_token_account(s.lp_mint, s.creator.pubkey(), 0);
    let res = s.ctx.claim_lp(s.market, s.creator.pubkey(), a_lp_ata);
    assert!(res.is_ok(), "{res:?}");
    assert_eq!(s.ctx.token_balance(a_lp_ata), a_share, "A pro-rata LP");

    // A's Contribution is CLOSED (reaped) and its rent went back to A's wallet.
    let (a_contrib, _) = kassandra_markets_sdk::pda::contribution(&s.market, &s.creator.pubkey());
    assert_eq!(s.ctx.lamports(a_contrib), 0, "A Contribution closed");
    assert_eq!(
        s.ctx.lamports(s.creator.pubkey()),
        a_wallet_before + a_contrib_rent,
        "A Contribution rent returned to contributor"
    );
    let m: Market = s.ctx.read_pod(s.market);
    assert_eq!(m.open_contributions, 1, "counter decremented after A");

    // vault still holds the un-swept remainder before B's (last) claim.
    let vault_before_b = s.ctx.token_balance(lp_vault);
    assert_eq!(
        vault_before_b,
        s.lp_total - a_share,
        "vault holds remainder"
    );

    // Contributor B claims LAST (counter == 1) → sweeps the ENTIRE remaining vault
    // (their pro-rata share PLUS A's floor dust), so lp_vault ends at exactly 0.
    let b_lp_ata = s.ctx.create_token_account(s.lp_mint, s.c2.pubkey(), 0);
    let res = s.ctx.claim_lp(s.market, s.c2.pubkey(), b_lp_ata);
    assert!(res.is_ok(), "{res:?}");
    assert_eq!(
        s.ctx.token_balance(b_lp_ata),
        vault_before_b,
        "B sweeps the remainder"
    );

    // lp_vault ends at EXACTLY 0 — no un-closeable dust left for close_market.
    assert_eq!(
        s.ctx.token_balance(lp_vault),
        0,
        "last claimer sweeps vault to 0"
    );

    // Both contributions closed; counter at 0.
    let (b_contrib, _) = kassandra_markets_sdk::pda::contribution(&s.market, &s.c2.pubkey());
    assert_eq!(s.ctx.lamports(a_contrib), 0, "A Contribution closed");
    assert_eq!(s.ctx.lamports(b_contrib), 0, "B Contribution closed");
    let m: Market = s.ctx.read_pod(s.market);
    assert_eq!(m.open_contributions, 0, "counter drained to 0");
}

#[test]
fn claim_lp_rejects_second_claim_contribution_closed() {
    let mut s = setup_active_two_contributors();
    let a_lp_ata = s.ctx.create_token_account(s.lp_mint, s.creator.pubkey(), 0);
    let res = s.ctx.claim_lp(s.market, s.creator.pubkey(), a_lp_ata);
    assert!(res.is_ok(), "{res:?}");

    // The claim CLOSED the Contribution, so a second claim can't even load it —
    // the absence of the account is the idempotency (fails the load guard).
    let (a_contrib, _) = kassandra_markets_sdk::pda::contribution(&s.market, &s.creator.pubkey());
    assert_eq!(
        s.ctx.lamports(a_contrib),
        0,
        "Contribution closed by first claim"
    );
    let res = s.ctx.claim_lp(s.market, s.creator.pubkey(), a_lp_ata);
    assert_eq!(custom_code(&res), Some(MarketError::InvalidAccount as u32));
}

#[test]
fn claim_lp_succeeds_after_resolution() {
    // FUND-LOSS REGRESSION: `resolve_market` is a permissionless crank that flips
    // status to Resolved the moment the oracle is terminal. claim_lp is the ONLY
    // path out of `lp_vault`, so it MUST stay open post-resolution (once the fee is
    // collected) or a contributor who hadn't claimed yet loses their LP forever.
    // The setup already resolved a fee-free market (fee_collected stamped).
    let mut s = setup_active_two_contributors();
    let m: Market = s.ctx.read_pod(s.market);
    assert_eq!(
        m.status,
        MarketStatus::Resolved.as_u8(),
        "status → Resolved"
    );
    assert_eq!(m.fee_collected, 1, "fee flag stamped");

    // A contributor who hadn't claimed before resolution still gets their LP.
    let a_share = expected_share(s.lp_total, SEED_A, s.total_contributed);
    assert!(a_share > 0);
    let a_lp_ata = s.ctx.create_token_account(s.lp_mint, s.creator.pubkey(), 0);
    let res = s.ctx.claim_lp(s.market, s.creator.pubkey(), a_lp_ata);
    assert!(res.is_ok(), "post-resolution claim: {res:?}");
    assert_eq!(
        s.ctx.token_balance(a_lp_ata),
        a_share,
        "still gets pro-rata LP"
    );
    // A claimed first (not the last claimer) → its Contribution is closed (reaped).
    let (a_contrib, _) = kassandra_markets_sdk::pda::contribution(&s.market, &s.creator.pubkey());
    assert_eq!(
        s.ctx.lamports(a_contrib),
        0,
        "Contribution closed after claim"
    );
}

#[test]
fn claim_lp_and_refund_mutually_exclusive() {
    // On a Resolved market, `refund` must fail (status != Cancelled) — proving a
    // contribution cannot be both LP-claimed and refunded.
    let mut s = setup_active_two_contributors();
    let a_lp_ata = s.ctx.create_token_account(s.lp_mint, s.creator.pubkey(), 0);
    let res = s.ctx.claim_lp(s.market, s.creator.pubkey(), a_lp_ata);
    assert!(res.is_ok(), "{res:?}");

    // Attempt to refund the same contribution on the Active market.
    let refund_dest = s.ctx.create_token_account(s.kass, s.creator.pubkey(), 0);
    let res = s.ctx.refund(s.market, s.creator.pubkey(), refund_dest);
    assert_eq!(custom_code(&res), Some(MarketError::NotCancelled as u32));

    // Silence unused-field warning on oracle in this test path.
    let _ = s.oracle;
}

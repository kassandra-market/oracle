use super::fixtures::*;
use super::support::*;
use super::*;

use kassandra_oracles_program::state::{Market, Phase};

#[test]
fn settle_fraud_disqualifies_and_resolves_fail_side() {
    // fail TWAP (price 3e9) >> pass TWAP (1e9) * 1.1 → fraud → disqualify.
    let (mut ctx, f) = fixture(QUOTE_LOW, QUOTE_HIGH);
    let bond_pool_before = ctx.oracle(f.oracle).bond_pool;
    let surviving_before = ctx.oracle(f.oracle).surviving_count;
    // open_challenge bumped the open-market counter 0 → 1.
    assert_eq!(ctx.oracle(f.oracle).open_challenge_count, 1);
    // Physical KASS conservation across the split path holds right after open.
    assert_kass_conserved(&ctx, f.oracle, f.m.kass_vault_underlying);

    ctx.warp(TWAP_WINDOW + 1); // cross market.twap_end

    let ix = settle_ix(
        &ctx,
        f.oracle,
        f.market,
        f.ai_claim,
        f.proposer,
        f.m.question,
        f.pass_amm,
        f.fail_amm,
        &f.extras(),
        f.nonce,
    );
    // Pre-settle balances for the USDC/KASS conservation checks.
    let escrow_before = ctx.token_balance(f.escrow_vault);
    let stake_before = ctx.token_balance(f.stake_vault);
    assert_eq!(escrow_before, required_escrow_usdc(BOND), "escrow funded");

    ctx.send_many(&cu(ix), &[]).expect("settle should succeed");

    // C2 KASS-fee carve-out: 1% of the bond → challenger; bond − fee → bond_pool.
    let kass_fee = BOND / 100;
    let net_slash = BOND - kass_fee;

    let p = ctx.proposer(f.proposer);
    assert_eq!(p.disqualified, 1, "fraud proposer disqualified");
    assert_eq!(p.slashed, 1);
    assert_eq!(
        p.slashed_amount, net_slash,
        "bond − kass_fee forfeit to bond_pool"
    );

    let o = ctx.oracle(f.oracle);
    assert_eq!(o.surviving_count, surviving_before - 1);
    assert_eq!(
        o.bond_pool,
        bond_pool_before + net_slash,
        "bond_pool gets bond − kass_fee (identity == slashed_amount)"
    );
    assert_eq!(o.phase, Phase::Challenge as u8, "phase stays Challenge");
    // settle decremented the open-market counter 1 → 0.
    assert_eq!(
        o.open_challenge_count, 0,
        "challenge settled, counter back to 0"
    );

    let market: Market = ctx.read_pod(f.market);
    assert_eq!(market.settled, 1);

    // Question resolved FAIL-side [0,1], denominator 1.
    let (n0, n1, denom) = question_resolution(&ctx, f.m.question);
    assert_eq!((n0, n1, denom), (0, 1, 1), "fail-side resolution");

    // The other proposer is untouched.
    assert_eq!(ctx.proposer(f.proposer_other).disqualified, 0);

    // --- physical redeem: the bond's conditional KASS came back as underlying --
    // The KASS conditional vault's underlying is fully drained (redeemed), and
    // both oracle-PDA conditional-KASS holders are burned to 0.
    assert_eq!(
        ctx.token_balance(f.m.kass_vault_underlying),
        0,
        "redeem drained the conditional KASS vault underlying"
    );
    assert_eq!(ctx.token_balance(f.oracle_pass_kass), 0, "pass-KASS burned");
    assert_eq!(ctx.token_balance(f.oracle_fail_kass), 0, "fail-KASS burned");

    // --- KASS routing: redeem +BOND to stake_vault, then kass_fee → challenger -
    assert_eq!(
        ctx.token_balance(f.challenger_kass),
        kass_fee,
        "challenger receives the KASS fee"
    );
    assert_eq!(
        ctx.token_balance(f.stake_vault),
        stake_before + BOND - kass_fee,
        "stake_vault: +bond (redeem) − kass_fee (to challenger)"
    );
    // KASS conservation with the fee carve-out: stake_vault + vault_underlying +
    // challenger_kass == total_oracle_stake (the fee left the system to the
    // challenger; everything else is accounted in stake_vault / the drained vault).
    let total = ctx.oracle(f.oracle).total_oracle_stake;
    assert_eq!(
        ctx.token_balance(f.stake_vault)
            + ctx.token_balance(f.m.kass_vault_underlying)
            + ctx.token_balance(f.challenger_kass),
        total,
        "KASS conservation incl. the kass_fee carve-out",
    );

    // --- USDC routing: full escrow returned to the challenger, none to proposer -
    assert_eq!(
        ctx.token_balance(f.challenger_usdc_dest),
        escrow_before,
        "full USDC escrow returned to challenger on a successful challenge"
    );
    assert_eq!(
        ctx.token_balance(f.proposer_usdc),
        0,
        "no proposer USDC fee on a successful challenge"
    );
    assert_eq!(ctx.token_balance(f.escrow_vault), 0, "escrow fully drained");
}

#[test]
fn settle_honest_survives_and_resolves_pass_side() {
    // pass TWAP == fail TWAP (both 1e9) → within threshold → survives.
    let (mut ctx, f) = fixture(QUOTE_LOW, QUOTE_LOW);
    let bond_pool_before = ctx.oracle(f.oracle).bond_pool;
    let surviving_before = ctx.oracle(f.oracle).surviving_count;
    // Physical KASS conservation across the split path holds right after open.
    assert_kass_conserved(&ctx, f.oracle, f.m.kass_vault_underlying);

    ctx.warp(TWAP_WINDOW + 1);

    let ix = settle_ix(
        &ctx,
        f.oracle,
        f.market,
        f.ai_claim,
        f.proposer,
        f.m.question,
        f.pass_amm,
        f.fail_amm,
        &f.extras(),
        f.nonce,
    );
    let escrow_before = ctx.token_balance(f.escrow_vault);
    let stake_before = ctx.token_balance(f.stake_vault);
    assert_eq!(escrow_before, required_escrow_usdc(BOND), "escrow funded");

    ctx.send_many(&cu(ix), &[]).expect("settle should succeed");

    let p = ctx.proposer(f.proposer);
    assert_eq!(p.disqualified, 0, "honest proposer survives");
    assert_eq!(p.slashed, 0);
    assert_eq!(p.slashed_amount, 0);

    let o = ctx.oracle(f.oracle);
    assert_eq!(o.surviving_count, surviving_before, "no slash");
    assert_eq!(o.bond_pool, bond_pool_before);

    assert_eq!(ctx.read_pod::<Market>(f.market).settled, 1);

    // Question resolved PASS-side [1,0].
    let (n0, n1, denom) = question_resolution(&ctx, f.m.question);
    assert_eq!((n0, n1, denom), (1, 0, 1), "pass-side resolution");

    // --- physical redeem: bond stays the proposer's, back in stake_vault -------
    assert_eq!(
        ctx.token_balance(f.m.kass_vault_underlying),
        0,
        "redeem drained the conditional KASS vault underlying"
    );
    assert_eq!(ctx.token_balance(f.oracle_pass_kass), 0, "pass-KASS burned");
    assert_eq!(ctx.token_balance(f.oracle_fail_kass), 0, "fail-KASS burned");
    assert_eq!(
        ctx.token_balance(f.stake_vault),
        stake_before + BOND,
        "stake_vault: +bond (redeem), no KASS fee on a failed challenge"
    );
    assert_eq!(
        ctx.token_balance(f.challenger_kass),
        0,
        "no challenger KASS fee when the challenge fails"
    );
    // No KASS left the system on the survive path: stake_vault + underlying ==
    // total_oracle_stake (the original idle-bond conservation, now physical).
    assert_kass_conserved(&ctx, f.oracle, f.m.kass_vault_underlying);

    // --- USDC routing: 1% fee → proposer, the remainder → challenger -----------
    let usdc_fee = escrow_before / 100;
    assert_eq!(
        ctx.token_balance(f.proposer_usdc),
        usdc_fee,
        "proposer receives the USDC fee on a failed challenge"
    );
    assert_eq!(
        ctx.token_balance(f.challenger_usdc_dest),
        escrow_before - usdc_fee,
        "challenger gets the escrow minus the fee"
    );
    // USDC conservation: fee + return == escrow, exactly.
    assert_eq!(
        ctx.token_balance(f.proposer_usdc) + ctx.token_balance(f.challenger_usdc_dest),
        escrow_before,
        "USDC escrow fully accounted (fee + return == escrow)"
    );
    assert_eq!(ctx.token_balance(f.escrow_vault), 0, "escrow fully drained");
}

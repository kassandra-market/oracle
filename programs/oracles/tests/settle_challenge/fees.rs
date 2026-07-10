use super::fixtures::*;
use super::support::*;
use super::*;

use kassandra_oracles_program::state::Market;

#[test]
fn settle_flip_slashed_then_disqualified_no_underflow() {
    // Cross-path liveness: a proposer flip-slashed earlier in finalize_ai_claims
    // (slashed_amount = bond/2, still surviving) is then challenged + disqualified.
    // The carve-out tops the prior slash up to bond − kass_fee WITHOUT underflow
    // (defaults: 50% flip + 1% fee = 51% ≤ 100%), and the kass_fee is capped to the
    // remaining un-slashed bond. Settle must succeed and the accounting stay
    // consistent — this is the exact path the C2 regression would have bricked.
    let (mut ctx, f) = fixture(QUOTE_LOW, QUOTE_HIGH); // fraud (disqualify) path
    let prior = BOND / 2; // a 50% flip-slash already in bond_pool
    ctx.set_proposer_prior_slash(f.oracle, f.proposer, prior);
    let bond_pool_before = ctx.oracle(f.oracle).bond_pool;
    let stake_before = ctx.token_balance(f.stake_vault);
    assert_eq!(ctx.proposer(f.proposer).slashed_amount, prior);

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
    ctx.send_many(&cu(ix), &[])
        .expect("flip-slashed→disqualified settle must NOT underflow/brick");

    let kass_fee = BOND / 100;
    let net_slash = BOND - kass_fee; // 0.99 bond ≥ the 0.5 prior slash
    let p = ctx.proposer(f.proposer);
    assert_eq!(p.disqualified, 1);
    assert_eq!(p.slashed_amount, net_slash, "topped up to bond − kass_fee");
    // bond_pool gained only the DELTA (net_slash − prior), never double-counting.
    assert_eq!(
        ctx.oracle(f.oracle).bond_pool,
        bond_pool_before + (net_slash - prior),
        "bond_pool delta == net_slash − prior_slash (identity holds)"
    );
    assert_eq!(ctx.read_pod::<Market>(f.market).settled, 1);
    // KASS routing: redeem +bond, kass_fee → challenger.
    assert_eq!(ctx.token_balance(f.challenger_kass), kass_fee);
    assert_eq!(
        ctx.token_balance(f.stake_vault),
        stake_before + BOND - kass_fee
    );
}

#[test]
fn settle_fee_rates_are_oracle_snapshotted() {
    // Fee sensitivity: settle reads the directional fee rates from the ORACLE's
    // snapshot (what create_oracle copies from Protocol and set_config retunes),
    // NOT a hard-coded const. Retune the snapshot to 5% KASS / 2% USDC and assert
    // the disqualify-path KASS fee tracks it. (set_config → new-oracle snapshot is
    // covered in set_config.rs; this pins the settle-side consumption.)
    let (mut ctx, f) = fixture(QUOTE_LOW, QUOTE_HIGH); // fraud (disqualify) path
                                                       // 5% KASS fee on a successful challenge, 2% USDC fee on a failed one.
    ctx.set_challenge_fees(f.oracle, 2, 100, 5, 100);
    let bond_pool_before = ctx.oracle(f.oracle).bond_pool;
    let escrow_before = ctx.token_balance(f.escrow_vault);

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
    ctx.send_many(&cu(ix), &[]).expect("settle should succeed");

    // 5% of the bond → challenger; bond − fee → bond_pool (the new rate, not 1%).
    let kass_fee = BOND * 5 / 100;
    assert_eq!(
        ctx.token_balance(f.challenger_kass),
        kass_fee,
        "settle used the retuned 5% KASS fee"
    );
    assert_eq!(ctx.proposer(f.proposer).slashed_amount, BOND - kass_fee);
    assert_eq!(
        ctx.oracle(f.oracle).bond_pool,
        bond_pool_before + BOND - kass_fee
    );
    // Full USDC escrow still returned to the challenger on disqualify (the USDC
    // fee rate only bites on the survive path).
    assert_eq!(ctx.token_balance(f.challenger_usdc_dest), escrow_before);
}

//! Tests for the dynamic EMA creation fee (Task H2): KASS burned on
//! `create_oracle`, proportional to an EMA of recent creation activity — 0 at
//! genesis, grows with rapid creations, shrinks when idle.

mod common;
use common::*;

use kassandra_program::config::{FEE_EMA_HALFLIFE_SECS, FEE_EMA_INCREMENT, FEE_EMA_SCALE};
use kassandra_program::fee::{bumped_fee_ema, decay_fee_ema, fee_for_ema};

/// Helper: create an oracle with a fresh future deadline and report
/// `(fee_burned, emission_minted)`. The fee is measured from the creator's KASS
/// balance (unaffected by emission); the emission is read from the new oracle.
/// Emission is ON by default, so `create_oracle` BURNS `fee` from the creator
/// AND MINTS `emission` into the oracle vault — the net mint-supply delta is
/// `emission − fee` (not just `−fee`). That money-flow identity is asserted here.
fn create_and_measure(ctx: &mut TestCtx, nonce: u64) -> (u64, u64) {
    let bal_before = ctx.token_balance(ctx.payer_kass);
    let sup_before = ctx.mint_supply(ctx.kass_mint);
    let deadline = ctx.now() + 1_000_000;
    let (oracle, res) = ctx.create_oracle(nonce, 2, deadline, 600);
    assert!(res.is_ok(), "create_oracle should succeed: {res:?}");
    let bal_after = ctx.token_balance(ctx.payer_kass);
    let sup_after = ctx.mint_supply(ctx.kass_mint);
    let fee = bal_before - bal_after;
    let emission = ctx.oracle(oracle).reward_emission;
    assert_eq!(
        sup_after,
        sup_before - fee + emission,
        "supply delta == emission minted − fee burned"
    );
    (fee, emission)
}

#[test]
fn genesis_create_is_free() {
    let mut ctx = TestCtx::new();
    let (protocol_pda, res) = ctx.init_protocol();
    assert!(res.is_ok(), "init_protocol should succeed: {res:?}");

    let bal_before = ctx.token_balance(ctx.payer_kass);
    let sup_before = ctx.mint_supply(ctx.kass_mint);
    let now = ctx.now();

    let (fee, emission) = create_and_measure(&mut ctx, 0);
    assert_eq!(fee, 0, "genesis creation must be free");
    assert_eq!(
        ctx.token_balance(ctx.payer_kass),
        bal_before,
        "creator KASS unchanged at genesis (fee 0; emission is minted into the vault, not from the creator)"
    );
    // Emission is ON by default: the genesis fee is 0 (no burn), so the mint
    // supply rose by EXACTLY the minted emission.
    assert_eq!(
        ctx.mint_supply(ctx.kass_mint),
        sup_before + emission,
        "mint supply rose by the minted emission (fee 0 at genesis)"
    );

    let p = ctx.protocol(protocol_pda);
    assert_eq!(
        p.fee_ema, FEE_EMA_INCREMENT,
        "fee_ema bumped to one creation unit"
    );
    assert_eq!(p.last_creation_unix, now, "last_creation_unix == now");
}

#[test]
fn rapid_creates_fee_grows_and_burns() {
    let mut ctx = TestCtx::new();
    let _ = ctx.init_protocol();

    let bal_start = ctx.token_balance(ctx.payer_kass);
    let sup_start = ctx.mint_supply(ctx.kass_mint);

    // Four rapid creations with no clock advance between them: decay is 0, so
    // each adds a full FEE_EMA_INCREMENT and the fee strictly increases.
    let mut fees = Vec::new();
    let mut emissions = Vec::new();
    for nonce in 0..4u64 {
        // create_and_measure asserts the per-create money-flow identity
        // (supply delta == emission − fee) internally.
        let (fee, emission) = create_and_measure(&mut ctx, nonce);
        fees.push(fee);
        emissions.push(emission);
    }

    assert_eq!(fees[0], 0, "first (genesis) creation is free");
    for w in fees.windows(2) {
        assert!(
            w[1] > w[0],
            "fee must strictly increase across rapid creations: {fees:?}"
        );
    }

    // The 2nd creation sees fee_ema == 1.0 unit → fee == FEE_PER_EMA_UNIT.
    assert_eq!(fees[1], fee_for_ema(FEE_EMA_INCREMENT));

    // Conservation: the creator balance dropped by Σ fees; the mint supply
    // dropped by Σ fees (the burns) but ROSE by Σ emissions (emission is ON by
    // default), so its net delta is `Σ emissions − Σ fees`.
    let total: u64 = fees.iter().sum();
    let total_emission: u64 = emissions.iter().sum();
    assert_eq!(ctx.token_balance(ctx.payer_kass), bal_start - total);
    assert_eq!(
        ctx.mint_supply(ctx.kass_mint),
        sup_start - total + total_emission
    );
}

#[test]
fn idle_gap_shrinks_the_fee() {
    let mut ctx = TestCtx::new();
    let (protocol_pda, _) = ctx.init_protocol();

    // Genesis (free) → fee_ema = 1.0 unit.
    let (f0, _) = create_and_measure(&mut ctx, 0);
    assert_eq!(f0, 0);

    // Immediate (no-gap) creation: fee proportional to the un-decayed EMA.
    let (fee_nogap, _) = create_and_measure(&mut ctx, 1);
    assert!(fee_nogap > 0, "second rapid creation must charge a fee");

    // Idle for two half-lives, then create again: the EMA has decayed, so this
    // fee is strictly LOWER than the no-gap counterpart.
    let ema_before_gap = ctx.protocol(protocol_pda).fee_ema;
    ctx.warp(2 * FEE_EMA_HALFLIFE_SECS);
    let now_at_gap = ctx.now();
    let (fee_gap, _) = create_and_measure(&mut ctx, 2);

    assert!(
        fee_gap < fee_nogap,
        "idle decay must shrink the fee: gap {fee_gap} >= nogap {fee_nogap}"
    );

    // The stored EMA reflects the decay: decayed(ema_before_gap) + one unit.
    let p = ctx.protocol(protocol_pda);
    let expected_decayed = decay_fee_ema(
        ema_before_gap,
        now_at_gap - 2 * FEE_EMA_HALFLIFE_SECS,
        now_at_gap,
    );
    assert_eq!(p.fee_ema, bumped_fee_ema(expected_decayed));
    assert!(
        p.fee_ema < ema_before_gap + FEE_EMA_INCREMENT,
        "decay must make the bumped EMA lower than the no-decay case"
    );
    // Sanity: two half-lives ≈ quarter of the pre-gap EMA.
    assert!((expected_decayed as u128) < (ema_before_gap as u128) / 2);
    let _ = FEE_EMA_SCALE;
}

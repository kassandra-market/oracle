//! The fraud, swap-driven real-AMM e2e lifecycle test (FAIL pool pumped past the
//! threshold via a genuine BUY swap + slot-weighted TWAP → disqualifies).

use super::lifecycle_common::*;
use super::ops::*;
use super::support::*;
use super::*;

use kassandra_oracles_program::config::{
    CHALLENGE_FAIL_USDC_FEE_DEN, CHALLENGE_FAIL_USDC_FEE_NUM, CHALLENGE_SUCCESS_KASS_FEE_DEN,
    CHALLENGE_SUCCESS_KASS_FEE_NUM,
};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

#[test]
fn e2e_fraud_full_lifecycle_swap_driven_disqualifies() {
    let mut ctx = TestCtx::new();
    ctx.svm.add_program(vault_id(), VAULT_SO).unwrap();
    ctx.svm.add_program(amm_id(), AMM_SO).unwrap();
    let kass_dao = ctx.bless_kass_price();

    let c = front_door_to_challenge(&mut ctx);
    let (m, oracle_pass_kass, oracle_fail_kass) = setup_market(&mut ctx, c.oracle);

    // PASS pool stays neutral (1e9). FAIL pool: a genuine BUY swap pushes its
    // price up, and TWO cranks 300 slots apart accumulate the post-swap price
    // into the slot-weighted TWAP — so the disqualify decision is driven by REAL
    // trading moving the TWAP past `pass + 10% threshold`, not a seeded price.
    let pass_amm = build_pool(
        &mut ctx,
        m.pass_mint,
        m.pass_usdc,
        BASE_RESERVE,
        QUOTE_NEUTRAL,
    );
    let fail_amm = build_pool(
        &mut ctx,
        m.fail_mint,
        m.fail_usdc,
        BASE_RESERVE,
        QUOTE_NEUTRAL,
    );
    crank_pool(&mut ctx, pass_amm);
    // 90 USDC BUY drains the fail pool's base hard → instantaneous price ≈ 3.5e9.
    swap_buy(&mut ctx, fail_amm, m.fail_mint, m.fail_usdc, 90_000_000);
    crank_pool(&mut ctx, fail_amm); // records the post-swap price
    crank_pool(&mut ctx, fail_amm); // accumulates it: TWAP ≈ (1e9 + 3.5e9)/2 ≫ 1.1e9

    let (market, _) =
        Pubkey::find_program_address(&[b"market", c.ai_claim.as_ref()], &ctx.program_id);
    let challenger = Keypair::new();
    ctx.svm
        .airdrop(&challenger.pubkey(), 1_000_000_000)
        .unwrap();
    let challenger_usdc_src = ctx.fund_usdc(&challenger, 5_000_000);

    let ix = open_challenge_ix(
        &ctx,
        c.oracle,
        c.ai_claim,
        c.proposer,
        market,
        challenger.pubkey(),
        &m,
        pass_amm,
        fail_amm,
        c.stake_vault,
        oracle_pass_kass,
        oracle_fail_kass,
        kass_dao,
        challenger_usdc_src,
        c.nonce,
    );
    ctx.send_many(&cu(ix), &[&challenger])
        .expect("open_challenge");

    let payouts = fabricate_payouts(&mut ctx, market, c.proposer_authority, challenger.pubkey());
    let escrow = required_escrow_usdc(BOND);

    // Emission is ON by default: the real-flow oracle's stake_vault also holds
    // the creation-time `reward_emission` (untouched until finalize_oracle). The
    // KASS-conservation baseline must therefore include it alongside Σ stakes.
    let total_before =
        ctx.oracle(c.oracle).total_oracle_stake + ctx.oracle(c.oracle).reward_emission;
    let bond_pool_before = ctx.oracle(c.oracle).bond_pool;
    let surviving_before = ctx.oracle(c.oracle).surviving_count;
    let stake_before = ctx.token_balance(c.stake_vault);

    ctx.warp(TWAP_WINDOW + 1);
    let extras = SettleExtras {
        stake_vault: c.stake_vault,
        kass_vault: m.kass_vault,
        kass_vault_underlying: m.kass_vault_underlying,
        pass_mint: m.pass_mint,
        fail_mint: m.fail_mint,
        oracle_pass_kass,
        oracle_fail_kass,
        escrow_vault: payouts.escrow_vault,
        proposer_usdc: payouts.proposer_usdc,
        challenger_usdc_dest: payouts.challenger_usdc_dest,
        challenger_kass: payouts.challenger_kass,
    };
    let ix = settle_ix(
        &ctx, c.oracle, market, c.ai_claim, c.proposer, m.question, pass_amm, fail_amm, &extras,
        c.nonce,
    );
    ctx.send_many(&cu(ix), &[]).expect("settle_challenge");

    let model = ConservationModel::compute(
        true,
        BOND,
        escrow,
        0,
        CHALLENGE_SUCCESS_KASS_FEE_NUM,
        CHALLENGE_SUCCESS_KASS_FEE_DEN,
        CHALLENGE_FAIL_USDC_FEE_NUM,
        CHALLENGE_FAIL_USDC_FEE_DEN,
    );

    assert_resolution_and_conservation(
        &ctx,
        c.oracle,
        market,
        c.proposer,
        m.question,
        &extras,
        &model,
        total_before,
        bond_pool_before,
        stake_before,
    );
    // Disqualify specifics: bond − kass_fee to bond_pool, surviving -= 1.
    let p = ctx.proposer(c.proposer);
    assert_eq!(p.disqualified, 1, "fraud → disqualified (swap-driven TWAP)");
    assert_eq!(p.slashed, 1);
    assert_eq!(p.slashed_amount, BOND - model.kass_fee);
    let o = ctx.oracle(c.oracle);
    assert_eq!(o.surviving_count, surviving_before - 1);
    assert_eq!(o.bond_pool, bond_pool_before + (BOND - model.kass_fee));
    let (n0, n1, denom) = question_resolution(&ctx, m.question);
    assert_eq!((n0, n1, denom), (0, 1, 1), "fail-side resolution");
}

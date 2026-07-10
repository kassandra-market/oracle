//! The honest, real-AMM e2e lifecycle test (both pools neutral → survives).

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
fn e2e_honest_full_lifecycle_survives() {
    let mut ctx = TestCtx::new();
    ctx.svm.add_program(vault_id(), VAULT_SO).unwrap();
    ctx.svm.add_program(amm_id(), AMM_SO).unwrap();
    let kass_dao = ctx.bless_kass_price();

    // REAL front door → Challenge with an un-slashed proposer + real AiClaim.
    let c = front_door_to_challenge(&mut ctx);
    let (m, oracle_pass_kass, oracle_fail_kass) = setup_market(&mut ctx, c.oracle);

    // Both pools at the neutral seeded price (1e9) → pass == fail → survives.
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
    crank_pool(&mut ctx, fail_amm);

    let (market, _) =
        Pubkey::find_program_address(&[b"market", c.ai_claim.as_ref()], &ctx.program_id);
    let challenger = Keypair::new();
    ctx.svm
        .airdrop(&challenger.pubkey(), 1_000_000_000)
        .unwrap();
    let challenger_usdc_src = ctx.fund_usdc(&challenger, 5_000_000);

    // REAL open_challenge: escrow + program-signed bond split.
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
    assert_eq!(
        ctx.token_balance(payouts.escrow_vault),
        escrow,
        "escrow funded"
    );

    // Emission is ON by default: the real-flow oracle's stake_vault also holds
    // the creation-time `reward_emission` (untouched until finalize_oracle). The
    // KASS-conservation baseline must therefore include it alongside Σ stakes.
    let total_before =
        ctx.oracle(c.oracle).total_oracle_stake + ctx.oracle(c.oracle).reward_emission;
    let bond_pool_before = ctx.oracle(c.oracle).bond_pool;
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

    // Independent reference (survive path; proposer un-slashed).
    let model = ConservationModel::compute(
        false,
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
    // Survive specifics.
    assert_eq!(ctx.proposer(c.proposer).disqualified, 0, "honest survives");
    assert_eq!(ctx.proposer(c.proposer).slashed_amount, 0);
    assert_eq!(ctx.oracle(c.oracle).bond_pool, bond_pool_before, "no slash");
    let (n0, n1, denom) = question_resolution(&ctx, m.question);
    assert_eq!((n0, n1, denom), (1, 0, 1), "pass-side resolution");
}

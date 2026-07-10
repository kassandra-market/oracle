// ---------------------------------------------------------------------------
// Arm E (Task S5): INVALIDDEADEND physical settlement conservation
// ---------------------------------------------------------------------------

use super::*;

use kassandra_oracles_program::state::Phase;
use solana_keypair::Keypair;
use solana_signer::Signer;

/// Run the InvalidDeadend settlement arm: seed a 2-proposer disputed oracle whose
/// surviving plurality ties, set a fuzzed emission, drive the REAL finalize_oracle
/// (which BURNS the emission back), then claim — every staker reclaims their full
/// stake and the vault drains to exactly 0 (`Σ payouts == Σ stakes`).
fn run_deadend_settlement(bond0: u64, bond1: u64, emission: u64) -> Result<(), TestCaseError> {
    let mut ctx = TestCtx::new();
    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: bond0,
        },
        ProposerSpec {
            option: 1,
            bond: bond1,
        },
    ]);
    let pdas: Vec<Pubkey> = ctx.proposers(oracle).iter().map(|p| p.pda).collect();
    let auths: Vec<Keypair> = ctx
        .proposers(oracle)
        .iter()
        .map(|p| p.authority.insecure_clone())
        .collect();
    // DISTINCT surviving claim options → plurality tie → InvalidDeadend.
    ctx.set_proposer_claim_option(pdas[0], 0);
    ctx.set_proposer_claim_option(pdas[1], 1);
    ctx.set_phase(oracle, Phase::Challenge);
    if emission > 0 {
        ctx.set_reward_emission(oracle, emission);
    }

    let vault = ctx.seeded(oracle).stake_vault;
    let nonce = ctx.seeded(oracle).nonce;
    let supply_before = ctx.mint_supply(ctx.kass_mint);
    let stakes = bond0 + bond1;
    prop_assert_eq!(ctx.token_balance(vault), stakes + emission);

    // REAL finalize_oracle → InvalidDeadend + emission burn-back.
    ctx.warp(WINDOW + 1);
    let res = ctx.send(ctx.finalize_oracle_ix(oracle, &pdas), &[]);
    prop_assert!(res.is_ok(), "finalize_oracle should succeed: {:?}", res);
    let o = ctx.oracle(oracle);
    prop_assert_eq!(o.phase, Phase::InvalidDeadend as u8);
    prop_assert_eq!(o.reward_pool, 0u64);
    prop_assert_eq!(
        ctx.token_balance(vault),
        stakes,
        "emission burned out of the vault"
    );
    prop_assert_eq!(
        ctx.mint_supply(ctx.kass_mint),
        supply_before - emission,
        "supply returns by the burned emission"
    );

    // Full returns: every proposer reclaims its whole bond, vault drains to 0.
    let mut total_payout = 0u64;
    for (auth, pda) in auths.iter().zip(&pdas) {
        let bond = ctx.proposer(*pda).bond;
        let dest = ctx.fund_kass(auth, 0);
        let ix = ctx.claim_proposer_ix(oracle, nonce, *pda, dest, vault, auth.pubkey());
        let res = ctx.send(ix, &[]);
        prop_assert!(res.is_ok(), "deadend claim should succeed: {:?}", res);
        prop_assert_eq!(ctx.token_balance(dest), bond, "full bond returned");
        total_payout += bond;
    }
    prop_assert_eq!(total_payout, stakes, "Σ payouts == Σ stakes");
    prop_assert_eq!(
        ctx.token_balance(vault),
        0u64,
        "vault fully drained on dead-end"
    );
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 48,
        max_shrink_iters: 128,
        .. ProptestConfig::default()
    })]

    /// Arm E (Task S5) — INVALIDDEADEND physical settlement with a fuzzed
    /// emission BURNED back by a REAL `finalize_oracle`, then full-stake returns
    /// claimed via the REAL S2 instruction: `Σ payouts == Σ stakes`, vault drained
    /// to 0, supply returns by the burned emission.
    #[test]
    fn deadend_settlement_conservation(
        bond0 in 1_000u64..3_001,
        bond1 in 1_000u64..3_001,
        emission in 0u64..2_001,
    ) {
        run_deadend_settlement(bond0, bond1, emission)?;
    }
}

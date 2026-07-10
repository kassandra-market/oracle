// ===========================================================================
// Arm F (DS1): SLASHED-then-deadend physical settlement conservation.
// ===========================================================================
//
// A tie dead-end reached via the REAL `finalize_oracle` AFTER fuzzed proposer
// slashes (challenge-disqualify with a fuzzed kass_fee, and flip-slashed
// SURVIVORS), with a fuzzed emission. finalize_oracle BURNS the slashed
// `bond_pool` + the emission; the survivors' claims then drain the vault to dust.
// Runs BOTH a plain InvalidDeadend AND a governance-resolved (`resolve_deadend` →
// Resolved) sweep and asserts they pay IDENTICALLY (the no-marker insight), with
// the full conservation equation:
//   Σ returned principal + dust + Σ kass_fee_out + bond_pool_burned
//     + emission_burned == Σ bonds + emission.

use super::*;

use kassandra_oracles_program::state::Phase;
use solana_keypair::Keypair;
use solana_signer::Signer;

/// One fuzzed proposer for Arm F: a flip-slashed SURVIVOR or a challenge-
/// disqualified proposer (kass_fee left the vault).
#[derive(Clone, Copy, Debug)]
struct SlashDeadendProposerGen {
    bond: u64,
    /// % of the bond slashed (0..=100): a flip slash if surviving, else the
    /// bond_pool contribution `bond − kass_fee` if disqualified.
    slash_pct: u8,
    disqualified: bool,
}

fn slash_deadend_proposer_strategy() -> impl Strategy<Value = SlashDeadendProposerGen> {
    (1_000u64..3_001, 0u8..=100, any::<bool>()).prop_map(|(bond, slash_pct, disqualified)| {
        SlashDeadendProposerGen {
            bond,
            slash_pct,
            disqualified,
        }
    })
}

fn run_slashed_deadend_settlement(
    extra: &[SlashDeadendProposerGen],
    emission: u64,
    governance_resolve: bool,
) -> Result<(), TestCaseError> {
    let mut ctx = TestCtx::new();

    // Two SURVIVORS (bonds 2000) claiming DISTINCT options → plurality tie →
    // dead-end, plus the fuzzed `extra` proposers (each either a flip-slashed
    // survivor or a challenge-disqualified one). Keeping exactly two "anchor"
    // survivors guarantees the tie regardless of how `extra` is partitioned.
    let mut specs = vec![
        ProposerSpec {
            option: 0,
            bond: 2_000,
        },
        ProposerSpec {
            option: 1,
            bond: 2_000,
        },
    ];
    // Each extra proposer gets a DISTINCT option (2, 3, ...) so that EVERY
    // surviving claim is unique → the plurality always ties → dead-end, no matter
    // how `extra` is partitioned into survivors/disqualified.
    for (i, e) in extra.iter().enumerate() {
        specs.push(ProposerSpec {
            option: (2 + i) as u8,
            bond: e.bond,
        });
    }
    let oracle = ctx.seed_disputed_oracle(&specs);
    let pdas: Vec<Pubkey> = ctx.proposers(oracle).iter().map(|p| p.pda).collect();
    let auths: Vec<Keypair> = ctx
        .proposers(oracle)
        .iter()
        .map(|p| p.authority.insecure_clone())
        .collect();

    // Anchor survivors claim distinct options → tie.
    ctx.set_proposer_claim_option(pdas[0], 0);
    ctx.set_proposer_claim_option(pdas[1], 1);

    let mut kass_fee_out = 0u64;
    for (i, e) in extra.iter().enumerate() {
        let pda = pdas[2 + i];
        let slash = (e.bond as u128 * e.slash_pct as u128 / 100) as u64;
        if e.disqualified {
            // Challenge-disqualify: kass_fee = bond − slash left the vault; bond_pool
            // gains `slash`. (slash == bond_pool contribution.)
            let kass_fee = e.bond - slash;
            ctx.seed_challenge_disqualify(oracle, pda, kass_fee);
            kass_fee_out += kass_fee;
        } else {
            // Flip-slashed SURVIVING: slash into bond_pool; claim its DISTINCT
            // option (2 + i) so the all-distinct surviving plurality stays a tie.
            ctx.set_proposer_prior_slash(oracle, pda, slash);
            ctx.set_proposer_claim_option(pda, (2 + i) as u8);
        }
    }
    ctx.set_phase(oracle, Phase::Challenge);
    if emission > 0 {
        ctx.set_reward_emission(oracle, emission);
    }
    if governance_resolve {
        ctx.ensure_protocol();
        let dao = Keypair::new();
        ctx.airdrop(&dao, 1_000_000_000);
        let (_da, kass_dao) = TestCtx::stand_in_governance(0x44);
        ctx.force_governance(dao.pubkey(), kass_dao);

        let vault = ctx.seeded(oracle).stake_vault;
        let nonce = ctx.seeded(oracle).nonce;
        let sum_bonds: u64 = specs.iter().map(|s| s.bond).sum();
        let supply_before = ctx.mint_supply(ctx.kass_mint);
        let bond_pool = ctx.oracle(oracle).bond_pool;

        ctx.warp(WINDOW + 1);
        let res = ctx.send(ctx.finalize_oracle_ix(oracle, &pdas), &[]);
        prop_assert!(res.is_ok(), "finalize_oracle: {:?}", res);
        let (_p, rres) = ctx.resolve_deadend(oracle, &dao, 0);
        prop_assert!(rres.is_ok(), "resolve_deadend: {:?}", rres);
        let o = ctx.oracle(oracle);
        prop_assert_eq!(o.phase, Phase::Resolved as u8);
        prop_assert_eq!(o.reward_pool, 0u64);
        assert_deadend_drains(
            &mut ctx,
            oracle,
            nonce,
            vault,
            &pdas,
            &auths,
            sum_bonds,
            emission,
            bond_pool,
            kass_fee_out,
            supply_before,
        )?;
        return Ok(());
    }

    let vault = ctx.seeded(oracle).stake_vault;
    let nonce = ctx.seeded(oracle).nonce;
    let sum_bonds: u64 = specs.iter().map(|s| s.bond).sum();
    let supply_before = ctx.mint_supply(ctx.kass_mint);
    let bond_pool = ctx.oracle(oracle).bond_pool;

    ctx.warp(WINDOW + 1);
    let res = ctx.send(ctx.finalize_oracle_ix(oracle, &pdas), &[]);
    prop_assert!(res.is_ok(), "finalize_oracle: {:?}", res);
    let o = ctx.oracle(oracle);
    prop_assert_eq!(o.phase, Phase::InvalidDeadend as u8);
    prop_assert_eq!(o.reward_pool, 0u64);
    assert_deadend_drains(
        &mut ctx,
        oracle,
        nonce,
        vault,
        &pdas,
        &auths,
        sum_bonds,
        emission,
        bond_pool,
        kass_fee_out,
        supply_before,
    )
}

/// Shared post-finalize assertion for Arm F: the slashed bond_pool + emission
/// were burned, every survivor reclaims `bond − slashed_amount` / every
/// disqualified proposer 0, the vault drains to dust, and the full conservation
/// equation balances.
#[allow(clippy::too_many_arguments)]
fn assert_deadend_drains(
    ctx: &mut TestCtx,
    oracle: Pubkey,
    nonce: u64,
    vault: Pubkey,
    pdas: &[Pubkey],
    auths: &[Keypair],
    sum_bonds: u64,
    emission: u64,
    bond_pool: u64,
    kass_fee_out: u64,
    supply_before: u64,
) -> Result<(), TestCaseError> {
    // The slashed bond_pool + emission were burned back to the reservoir.
    prop_assert_eq!(
        ctx.mint_supply(ctx.kass_mint),
        supply_before - bond_pool - emission,
        "bond_pool + emission burned"
    );
    // Post-burn vault == Σ bonds − kass_fee_out − bond_pool (the returnable
    // non-slashed principal).
    let vault_after = ctx.token_balance(vault);
    prop_assert_eq!(vault_after, sum_bonds - kass_fee_out - bond_pool);

    let mut returned = 0u64;
    for (auth, pda) in auths.iter().zip(pdas) {
        let p = ctx.proposer(*pda);
        let expected = if p.disqualified != 0 {
            0
        } else {
            p.bond - p.slashed_amount
        };
        let dest = ctx.fund_kass(auth, 0);
        let res = ctx.send(
            ctx.claim_proposer_ix(oracle, nonce, *pda, dest, vault, auth.pubkey()),
            &[],
        );
        prop_assert!(res.is_ok(), "claim must not run vault short: {:?}", res);
        prop_assert_eq!(
            ctx.token_balance(dest),
            expected,
            "non-slashed principal only"
        );
        returned += expected;
    }
    let dust = ctx.token_balance(vault);
    prop_assert_eq!(returned, vault_after, "Σ returned == post-burn vault");
    prop_assert_eq!(dust, 0u64, "vault fully drained to dust");
    // Full conservation: Σ returned + dust + kass_fee_out + bond_pool_burned +
    // emission_burned == Σ bonds + emission.
    prop_assert_eq!(
        returned + dust + kass_fee_out + bond_pool + emission,
        sum_bonds + emission,
        "Σ returned + dust + kass_fee_out + bond_pool_burned + emission_burned == Σ bonds + emission"
    );
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 48,
        max_shrink_iters: 128,
        .. ProptestConfig::default()
    })]

    /// Arm F (DS1) — SLASHED-then-deadend physical settlement: fuzzed proposer
    /// slashes (challenge-disqualify w/ fuzzed kass_fee + flip-slashed survivors)
    /// + a fuzzed emission, terminated via the REAL `finalize_oracle` (burns the
    /// slashed bond_pool + emission), then the survivor claims drain the vault.
    /// `governance_resolve` toggles the `resolve_deadend` → Resolved path, which
    /// MUST pay identically (the no-marker insight). Asserts the full conservation
    /// equation incl. the kass_fee that left to the challenger.
    #[test]
    fn slashed_deadend_settlement_conservation(
        extra in prop::collection::vec(slash_deadend_proposer_strategy(), 0..=3),
        emission in 0u64..2_001,
        governance_resolve in any::<bool>(),
    ) {
        run_slashed_deadend_settlement(&extra, emission, governance_resolve)?;
    }
}

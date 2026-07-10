// ---------------------------------------------------------------------------
// Arm C: proposal-phase termination + conservation (Task H6)
// ---------------------------------------------------------------------------
//
// Drives the REAL happy-path entry point — `create_oracle` → `propose`×N →
// `finalize_proposals` — and asserts the proposal-phase invariants against an
// INDEPENDENT reference computed from the generated options alone (it does NOT
// trust the program's classification). This is the proposal-phase mirror of
// Arm A's dispute-phase fuzz.
//
//   * **Termination / decision.** The independent reference is simply "are all
//     generated options equal?": if so the oracle MUST end `Resolved` with
//     `resolved_option == that common option`; otherwise it MUST end
//     `FactProposal` with `dispute_bond_total == Σ bonds` (the fixed fact-quorum
//     denominator the dispute core consumes).
//   * **Conservation at the proposal boundary.** BEFORE any `submit_fact` (there
//     are none in this arm) the triple equality `stake_vault balance ==
//     oracle.total_oracle_stake == Σ bonds` holds exactly, and
//     `proposer_count == surviving_count == n`.
//   * **Cap never bricks.** `n <= 8 <= MAX_PROPOSERS (60)`, so every `propose`
//     and the `finalize_proposals` succeed. The cap-REJECTION itself (the 61st
//     proposer → `TooManyProposers`) is covered deterministically in
//     `tests/propose.rs`; it is intentionally NOT re-tested here (generating 60
//     proposers per case would be needlessly slow).

use super::*;

use kassandra_oracles_program::state::Phase;

#[derive(Clone, Debug)]
struct ProposalScenario {
    /// `options_count` passed to `create_oracle` (>= 2).
    options_count: u8,
    /// One `(option, bond)` per proposer; `option < options_count`, `bond > 0`.
    proposers: Vec<(u8, u64)>,
}

fn proposal_scenario_strategy() -> impl Strategy<Value = ProposalScenario> {
    // Pick options_count first, then draw options strictly within range. Keep
    // `n` modest (1..=8) — the cap is exercised deterministically elsewhere.
    (2u8..=4).prop_flat_map(|options_count| {
        prop::collection::vec((0u8..options_count, 1_000u64..3_001u64), 1..=8).prop_map(
            move |proposers| ProposalScenario {
                options_count,
                proposers,
            },
        )
    })
}

fn run_proposal_phase(s: &ProposalScenario) -> Result<(), TestCaseError> {
    let n = s.proposers.len();
    let sum_bonds: u64 = s.proposers.iter().map(|(_, b)| *b).sum();

    // Independent reference decision: all options equal => Resolved(common),
    // else FactProposal. Computed from the generated scenario, NOT the program.
    let first_option = s.proposers[0].0;
    let all_equal = s.proposers.iter().all(|(o, _)| *o == first_option);

    let mut ctx = TestCtx::new();
    let oracle = ctx.create_real_oracle(s.options_count, TWAP_WINDOW);
    // Emission is ON by default: create_oracle mints `reward_emission` into the
    // vault, so the vault holds Σ bonds PLUS the emission (never counted as stake).
    let emission = ctx.oracle(oracle).reward_emission;
    for (option, bond) in &s.proposers {
        ctx.propose_real(oracle, *option, *bond);
    }
    let vault = ctx.seeded(oracle).stake_vault;

    // ---- conservation at the proposal boundary (no facts in this arm) ------
    let pre = ctx.oracle(oracle);
    prop_assert_eq!(pre.proposer_count as usize, n, "proposer_count == n");
    prop_assert_eq!(pre.surviving_count as usize, n, "surviving_count == n");
    prop_assert_eq!(
        pre.total_oracle_stake,
        sum_bonds,
        "total_oracle_stake == Σ bonds"
    );
    prop_assert_eq!(
        ctx.token_balance(vault),
        sum_bonds + emission,
        "stake_vault balance == Σ bonds + emission"
    );
    prop_assert_eq!(
        ctx.token_balance(vault),
        pre.total_oracle_stake + emission,
        "stake_vault balance == total_oracle_stake + emission"
    );

    // ---- finalize_proposals (cap never bricks: n <= 8 <= MAX_PROPOSERS) ----
    let res = ctx.finalize_proposals_real(oracle);
    prop_assert!(res.is_ok(), "finalize_proposals should succeed: {:?}", res);

    // ---- termination / decision against the independent reference ----------
    let o = ctx.oracle(oracle);
    if all_equal {
        prop_assert_eq!(o.phase, Phase::Resolved as u8, "all-equal => Resolved");
        prop_assert_eq!(
            o.resolved_option,
            first_option,
            "resolved_option == common option"
        );
    } else {
        prop_assert_eq!(
            o.phase,
            Phase::FactProposal as u8,
            "distinct options => FactProposal"
        );
        prop_assert_eq!(
            o.dispute_bond_total,
            sum_bonds,
            "dispute_bond_total == Σ bonds"
        );
    }

    // Conservation still holds after finalize (no token CPI in either branch):
    // the vault is Σ bonds + the (untouched) emission.
    prop_assert_eq!(ctx.token_balance(vault), o.total_oracle_stake + emission);
    prop_assert_eq!(o.total_oracle_stake, sum_bonds);
    Ok(())
}

proptest! {
    // Arm C drives the full real happy-path entry point per case (fresh LiteSVM
    // + program deploy + create_oracle + up to 8 proposes + finalize), so the
    // case count is kept modest (48) to stay fast and non-flaky.
    #![proptest_config(ProptestConfig {
        cases: 48,
        max_shrink_iters: 128,
        .. ProptestConfig::default()
    })]

    /// Arm C (Task H6) — proposal-phase termination + conservation. Real
    /// create_oracle → propose×N → finalize_proposals; asserts the
    /// Resolved-iff-all-agree decision, `dispute_bond_total == Σ bonds` on
    /// conflict, and the `stake_vault == total_oracle_stake == Σ bonds` triple
    /// (proposer_count == surviving_count == n) at the proposal boundary.
    #[test]
    fn proposal_phase_invariants(s in proposal_scenario_strategy()) {
        run_proposal_phase(&s)?;
    }
}

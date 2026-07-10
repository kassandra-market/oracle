//! Property-based invariant fuzz harness for the Kassandra dispute core.
//!
//! This is Task 13: drive randomized-but-phase-LEGAL action sequences against a
//! seeded disputed oracle and assert the design §9 invariants. A pure-Rust
//! [`ReferenceModel`] predicts, from the generated scenario alone, the expected
//! terminal state + ledger; the harness then runs the REAL deployed instructions
//! in LiteSVM and asserts the on-chain result matches. The reference model is
//! deliberately independent of the program's own classification/plurality code
//! (it does NOT call `kassandra_oracles_program::plurality` or the processors).
//!
//! # §9 invariants: covered vs deferred
//!
//! * **#1 Phase ordering / disjoint windows — ASSERTED.** Every action is driven
//!   in its legal phase, and at three points the harness injects an illegal
//!   action (an instruction attempted in the wrong phase) and asserts it errors
//!   (`WrongPhase`). Fact-proposal and fact-voting are distinct phases gated by
//!   their own instructions, so submitting a fact during voting (and finalizing
//!   before/after the right phase) failing demonstrates the disjoint windows.
//! * **#2 Termination — ASSERTED.** Every generated dispute reaches a terminal
//!   phase (`Resolved` or `InvalidDeadend`) within the bounded single round. The
//!   fuzzer uses 2..=5 proposers (`<= MAX_PROPOSERS == 60`, the Task-12
//!   registration-cap contract) and <=3 facts so the one-shot `finalize_oracle`
//!   and the finalize calls all fit a single transaction.
//! * **#3 KASS conservation — ASSERTED.** No challenge is opened in this harness,
//!   so no KASS is moved into a MetaDAO conditional vault; the dispute
//!   instructions move nothing EXCEPT the terminal InvalidDeadend BURN. The
//!   precise statements are: (a) `stake_vault` balance `== oracle.total_oracle_
//!   stake` at every step UNTIL a terminal InvalidDeadend, where `finalize_oracle`
//!   / `finalize_no_facts` burn the slashed `bond_pool` (+ emission, disabled in
//!   this arm) back to the reservoir — so the post-burn vault is `total_oracle_
//!   stake − bond_pool` (the returnable principal; 0 on the no-facts path); a
//!   Resolved terminal moves nothing; (b) `total_oracle_stake` equals the sum of
//!   the seeded bonds, the fact submit stakes, and the vote stakes, reconciled
//!   against an independent reference ledger; (c) `bond_pool` equals the sum of
//!   the rejected-fact stakes and the proposer slashes, reconciled against the
//!   independently-computed reference bond pool.
//!   On the AiClaim path we ALSO cross-check the documented internal identity
//!   `bond_pool == Σ proposer.slashed_amount + Σ rejected-fact stakes`.
//! * **#7 Plurality correctness — ASSERTED.** At `Resolved`, `resolved_option`
//!   equals an independent reference plurality over the surviving proposers'
//!   `claim_option`s; ties / no-survivors → `InvalidDeadend`.
//! * **#9 Terminal exclusivity — ASSERTED.** The oracle ends in exactly one
//!   terminal phase; no token moved (bonds remain escrowed as counters);
//!   `Resolved` records a valid in-range option, `InvalidDeadend` (from
//!   `finalize_oracle`) carries the `CLAIM_OPTION_NONE` sentinel.
//!
//! ## Partially covered / deferred
//! * **#4 stake-locking** — structurally guaranteed (no instruction in this
//!   milestone transfers KASS OUT of the vault; conservation #3.1 above, asserted
//!   at every step, is exactly "locked bonds never leave"). Not separately fuzzed.
//! * **#5 fee-EMA** — DEFERRED: the creation-fee EMA lives in the un-built
//!   `create_oracle` tokenomics layer; nothing to fuzz here.
//! * **#6 quorum correctness** — covered as a side effect of #3: the reference
//!   model classifies each fact (agreed / duplicate-dominant / rejected) by the
//!   independent quorum rule and the on-chain `bond_pool` must match; the
//!   per-fact `agreed`/`duplicate` flags are also asserted.
//! * **#8 slash-trigger correctness** — DEFERRED from this proptest harness: it
//!   requires the heavy MetaDAO decision-market path (create_amm, add_liquidity,
//!   crank, open_challenge, settle_challenge), which is slow and flaky to run
//!   inside proptest in LiteSVM. It is covered DETERMINISTICALLY by the eight
//!   hand-enumerated TWAP-outcome cases in `settle_challenge.rs` (real deployed
//!   AMM). This harness therefore drives only the no-challenge path
//!   (`open_challenge_count == 0`), which still reaches a terminal state.
//! * **#10 closure** — DEFERRED: AiClaim-account closure / rent reclamation is an
//!   un-built separate instruction (see `finalize_oracle.rs` docs).
//!
//! ## Uniform per-proposer slash identity (asserted on ALL paths)
//! Every slash path — `finalize_ai_claims` (no-show / flip), `settle_challenge`
//! (challenge-fail), and the `finalize_facts` no-facts dead-end — records the
//! amount it adds to `bond_pool` in `proposer.slashed_amount`. The harness
//! therefore asserts `proposer.slashed_amount == that proposer's bond_pool
//! contribution` for EVERY slashed proposer (including the no-facts dead-end in
//! Arm A), and reconciles `bond_pool == Σ proposer.slashed_amount + Σ
//! rejected-fact stakes` on both terminal paths. (An earlier revision left
//! `slashed_amount == 0` on the no-facts path; that gap is now fixed in
//! `finalize_facts`, so the identity is uniform and no longer path-scoped.)
//!
//! # Arm split: PRE-settlement (A/B/C) vs POST-settlement (D/E)
//! Arms **A/B/C** are the original COUNTER-ONLY invariant fuzz: emission DISABLED,
//! NO claims run, asserting the dispute-core ledger at the terminal counter state
//! (`stake_vault == total_oracle_stake`, the bond_pool identity, plurality). They
//! are unchanged by S5 and still green. Arms **D/E** (Task S5) are the PHYSICAL
//! settlement fuzz: they take a terminal oracle with emission ENABLED, run every
//! real S2 claim (+ S4 close), and assert each payout against an INDEPENDENT
//! reference (a reimplementation of the bucket/pro-rata/ceil-slash math — it never
//! calls `kassandra_oracles_program::reward`) plus the conservation equation. See the
//! Arms D/E banner further down for the full real-vs-seeded split.
//!
//! The invariant arms are split across the `invariants/` submodules (one file per
//! arm); this root retains the shared scenario generation, the pure-Rust
//! [`ReferenceModel`], the independent `ref_plurality`, and the shared
//! `finalize_oracle_ix` builder that Arms A and B both drive.

mod common;
use common::*;

use std::collections::BTreeMap;

use kassandra_oracles_program::config::{
    FLIP_SLASH_DEN, FLIP_SLASH_NUM, THRESHOLD_DEN, THRESHOLD_NUM,
};
use proptest::prelude::*;
use solana_instruction::Instruction;
use solana_pubkey::Pubkey;

#[path = "invariants/full_dispute.rs"]
mod full_dispute;
#[path = "invariants/challenge_terminal.rs"]
mod challenge_terminal;
#[path = "invariants/proposal_phase.rs"]
mod proposal_phase;
#[path = "invariants/resolved_settlement.rs"]
mod resolved_settlement;
#[path = "invariants/deadend_settlement.rs"]
mod deadend_settlement;
#[path = "invariants/slashed_deadend_settlement.rs"]
mod slashed_deadend_settlement;

// ---------------------------------------------------------------------------
// Scenario generation
// ---------------------------------------------------------------------------

/// One generated proposer. `original_option` and `bond` are seeded; the proposer
/// then either no-shows (never submits an AI claim) or submits `claim_raw`
/// (clamped into the valid option range) at AI-claim time.
#[derive(Clone, Copy, Debug)]
struct ProposerGen {
    original_option: u8,
    bond: u64,
    /// True => never submits an AI claim (no-show => fully slashed).
    no_show: bool,
    /// Raw claim option (clamped to `0..options_count`) when submitting.
    claim_raw: u8,
}

/// One generated fact: its own submit stake plus the approve / duplicate vote
/// weights cast on it (each by a single fresh voter; 0 => no vote cast).
#[derive(Clone, Copy, Debug)]
struct FactGen {
    stake: u64,
    approve: u64,
    duplicate: u64,
}

#[derive(Clone, Debug)]
struct Scenario {
    proposers: Vec<ProposerGen>,
    facts: Vec<FactGen>,
}

fn proposer_strategy() -> impl Strategy<Value = ProposerGen> {
    (0u8..3, 1_000u64..3_001, any::<bool>(), 0u8..3).prop_map(
        |(original_option, bond, no_show, claim_raw)| ProposerGen {
            original_option,
            bond,
            no_show,
            claim_raw,
        },
    )
}

fn fact_strategy() -> impl Strategy<Value = FactGen> {
    (1u64..1_001, 0u64..6_000, 0u64..6_000).prop_map(|(stake, approve, duplicate)| FactGen {
        stake,
        approve,
        duplicate,
    })
}

fn scenario_strategy() -> impl Strategy<Value = Scenario> {
    (
        prop::collection::vec(proposer_strategy(), 2..=5),
        prop::collection::vec(fact_strategy(), 0..=3),
    )
        .prop_map(|(proposers, facts)| Scenario { proposers, facts })
}

// ---------------------------------------------------------------------------
// Reference model (pure Rust, independent of the program)
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum Terminal {
    /// `finalize_oracle` resolved with this winning option.
    Resolved(u8),
    /// Tie or no survivors via `finalize_oracle` (resolved_option == 0xFF).
    DeadendFromOracle,
    /// No facts ever submitted: `finalize_facts` dead-ends immediately
    /// (resolved_option is NOT written — stays its zeroed default).
    DeadendNoFacts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FactClass {
    Agreed,
    DuplicateDominant,
    Rejected,
}

struct ReferenceModel {
    /// Total KASS that flows into the vault across the whole run.
    total_in: u64,
    /// Independently-computed final `bond_pool`.
    bond_pool: u64,
    /// Independently-computed final `surviving_count`.
    surviving_count: u16,
    /// Per-fact classification (in scenario order).
    fact_class: Vec<FactClass>,
    /// Expected terminal state.
    terminal: Terminal,
    /// Σ proposer.slashed_amount — populated on EVERY path (the no-facts dead-end
    /// now records each proposer's bond as its slashed_amount, like every other
    /// slash path), so the per-proposer identity is asserted uniformly.
    slash_total: u64,
    /// Per-proposer expected `(slashed, disqualified, slashed_amount)`, in
    /// scenario order; populated on every path.
    proposer_expect: Vec<(bool, bool, u64)>,
    options_count: u8,
}

/// Independent strict-plurality over surviving votes: unique argmax => Winner,
/// else Tie / NoSurvivors. Deliberately a fresh implementation, not the
/// program's `plurality`.
fn ref_plurality(votes: &[u8]) -> Option<u8> {
    if votes.is_empty() {
        return None; // NoSurvivors
    }
    let mut counts: BTreeMap<u8, u32> = BTreeMap::new();
    for &v in votes {
        *counts.entry(v).or_insert(0) += 1;
    }
    let max = counts.values().copied().max().unwrap();
    let winners: Vec<u8> = counts
        .iter()
        .filter(|(_, &c)| c == max)
        .map(|(&o, _)| o)
        .collect();
    if winners.len() == 1 {
        Some(winners[0]) // Winner
    } else {
        None // Tie
    }
}

impl ReferenceModel {
    fn compute(s: &Scenario) -> Self {
        let dispute_bond_total: u64 = s.proposers.iter().map(|p| p.bond).sum();
        let max_opt = s.proposers.iter().map(|p| p.original_option).max().unwrap();
        // Mirror seed_disputed_oracle: options_count = max(max_opt + 1, 2).
        let options_count = ((max_opt as u16 + 1).max(2)) as u8;

        // KASS that flows in: seeded bonds + each fact submit stake + each vote.
        let mut total_in = dispute_bond_total;
        for f in &s.facts {
            total_in += f.stake + f.approve + f.duplicate;
        }

        // ---- no-facts dead-end ------------------------------------------------
        if s.facts.is_empty() {
            // Every proposer is fully slashed; slashed_amount == bond on this path
            // too (uniform identity).
            let proposer_expect = s.proposers.iter().map(|p| (true, true, p.bond)).collect();
            return ReferenceModel {
                total_in,
                bond_pool: dispute_bond_total, // every bond slashed into the pool
                surviving_count: 0,
                fact_class: Vec::new(),
                terminal: Terminal::DeadendNoFacts,
                slash_total: dispute_bond_total,
                proposer_expect,
                options_count,
            };
        }

        // ---- fact classification (quorum rule, §9 #6) -------------------------
        let mut bond_pool = 0u64;
        let mut fact_class = Vec::with_capacity(s.facts.len());
        for f in &s.facts {
            let class = if f.duplicate > f.approve {
                FactClass::DuplicateDominant
            } else if f.approve > f.duplicate
                && (f.approve as u128) * (THRESHOLD_DEN as u128)
                    >= (dispute_bond_total as u128) * (THRESHOLD_NUM as u128)
            {
                FactClass::Agreed
            } else {
                FactClass::Rejected
            };
            if class == FactClass::Rejected {
                bond_pool += f.stake;
            }
            fact_class.push(class);
        }

        // ---- AI-claim slashing (§9 #3 / #4 / #9) ------------------------------
        let mut surviving_count: u16 = 0;
        let mut slash_total = 0u64;
        let mut surviving_claims = Vec::new();
        let mut proposer_expect = Vec::with_capacity(s.proposers.len());
        for p in &s.proposers {
            if p.no_show {
                // No-show: full slash + disqualify.
                bond_pool += p.bond;
                slash_total += p.bond;
                proposer_expect.push((true, true, p.bond));
            } else {
                let claim = p.claim_raw % options_count;
                let flipped = claim != p.original_option;
                if flipped {
                    let slash = p.bond * FLIP_SLASH_NUM / FLIP_SLASH_DEN;
                    bond_pool += slash;
                    slash_total += slash;
                    proposer_expect.push((slash > 0, false, slash));
                } else {
                    proposer_expect.push((false, false, 0));
                }
                surviving_count += 1;
                surviving_claims.push(claim);
            }
        }

        // ---- terminal plurality (§9 #7 / #9) ----------------------------------
        let terminal = match ref_plurality(&surviving_claims) {
            Some(opt) => Terminal::Resolved(opt),
            None => Terminal::DeadendFromOracle,
        };

        ReferenceModel {
            total_in,
            bond_pool,
            surviving_count,
            fact_class,
            terminal,
            slash_total,
            proposer_expect,
            options_count,
        }
    }
}

// ---------------------------------------------------------------------------
// Shared instruction builder (Arms A and B both drive finalize_oracle)
// ---------------------------------------------------------------------------

fn finalize_oracle_ix(ctx: &TestCtx, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
    // S3 account order (oracle, kass_mint, stake_vault, token program, tail) +
    // the oracle-nonce payload, via the shared harness builder.
    ctx.finalize_oracle_ix(oracle, tail)
}

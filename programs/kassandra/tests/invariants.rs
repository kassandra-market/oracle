//! Property-based invariant fuzz harness for the Kassandra dispute core.
//!
//! This is Task 13: drive randomized-but-phase-LEGAL action sequences against a
//! seeded disputed oracle and assert the design §9 invariants. A pure-Rust
//! [`ReferenceModel`] predicts, from the generated scenario alone, the expected
//! terminal state + ledger; the harness then runs the REAL deployed instructions
//! in LiteSVM and asserts the on-chain result matches. The reference model is
//! deliberately independent of the program's own classification/plurality code
//! (it does NOT call `kassandra_program::plurality` or the processors).
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
//! calls `kassandra_program::reward`) plus the conservation equation. See the
//! Arms D/E banner further down for the full real-vs-seeded split.

mod common;
use common::*;

use std::collections::BTreeMap;

use kassandra_program::{
    config::{FLIP_SLASH_DEN, FLIP_SLASH_NUM, THRESHOLD_DEN, THRESHOLD_NUM},
    error::KassandraError,
    state::{Phase, CLAIM_OPTION_NONE, VOTE_APPROVE, VOTE_DUPLICATE},
};
use proptest::prelude::*;
use solana_instruction::Instruction;
use solana_instruction_error::InstructionError;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction_error::TransactionError;

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
// Instruction builders (mirror the per-instruction integration tests)
// ---------------------------------------------------------------------------

fn finalize_facts_ix(ctx: &TestCtx, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
    ctx.finalize_facts_ix(oracle, tail)
}

fn claim_pda(program_id: &Pubkey, oracle: &Pubkey, proposer: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"claim", oracle.as_ref(), proposer.as_ref()], program_id)
}

fn finalize_oracle_ix(ctx: &TestCtx, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
    // S3 account order (oracle, kass_mint, stake_vault, token program, tail) +
    // the oracle-nonce payload, via the shared harness builder.
    ctx.finalize_oracle_ix(oracle, tail)
}

// ---------------------------------------------------------------------------
// Conservation check (§9 #3, asserted at every step)
// ---------------------------------------------------------------------------

/// At every step the vault balance must equal the on-chain `total_oracle_stake`
/// (no KASS created/destroyed; nothing leaves the vault in this milestone).
fn assert_conservation_step(
    ctx: &TestCtx,
    oracle: Pubkey,
    vault: Pubkey,
) -> Result<(), TestCaseError> {
    let o = ctx.oracle(oracle);
    prop_assert_eq!(
        ctx.token_balance(vault),
        o.total_oracle_stake,
        "vault balance must equal total_oracle_stake (KASS conservation, §9 #3)"
    );
    Ok(())
}

fn custom_err(e: KassandraError) -> TransactionError {
    TransactionError::InstructionError(0, InstructionError::Custom(e as u32))
}

// ---------------------------------------------------------------------------
// Arm A: full randomized dispute flow (no MetaDAO challenge)
// ---------------------------------------------------------------------------

fn run_full_dispute(s: &Scenario) -> Result<(), TestCaseError> {
    let model = ReferenceModel::compute(s);

    let mut ctx = TestCtx::new();
    let specs: Vec<ProposerSpec> = s
        .proposers
        .iter()
        .map(|p| ProposerSpec {
            option: p.original_option,
            bond: p.bond,
        })
        .collect();
    let oracle = ctx.seed_disputed_oracle(&specs);
    let vault = ctx.seeded(oracle).stake_vault;
    let proposer_pdas: Vec<Pubkey> = ctx.proposers(oracle).iter().map(|p| p.pda).collect();

    // Sanity: the harness and program agree on options_count / bond total.
    prop_assert_eq!(ctx.oracle(oracle).options_count, model.options_count);
    prop_assert_eq!(
        ctx.oracle(oracle).dispute_bond_total,
        s.proposers.iter().map(|p| p.bond).sum::<u64>()
    );
    assert_conservation_step(&ctx, oracle, vault)?;

    // §9 #1: an instruction attempted in the wrong phase fails. In FactProposal,
    // finalize_facts (which requires FactVoting) must be rejected.
    {
        let err = ctx
            .send(finalize_facts_ix(&ctx, oracle, &proposer_pdas), &[])
            .unwrap_err()
            .err;
        prop_assert_eq!(err, custom_err(KassandraError::WrongPhase));
    }

    // ---- submit facts (FactProposal window) -------------------------------
    let mut fact_pdas = Vec::with_capacity(s.facts.len());
    for (i, f) in s.facts.iter().enumerate() {
        let submitter = Keypair::new();
        ctx.svm.airdrop(&submitter.pubkey(), 1_000_000_000).unwrap();
        let submitter_kass = ctx.fund_kass(&submitter, f.stake);
        let content_hash = [(i as u8) + 1; 32];
        let (fact, _) = TestCtx::fact_pda(&ctx.program_id, &oracle, &content_hash);
        ctx.send(
            submit_fact_ix(
                &ctx,
                oracle,
                fact,
                submitter.pubkey(),
                submitter_kass,
                vault,
                submit_fact_payload(&content_hash, f.stake, b"ipfs://fact"),
            ),
            &[&submitter],
        )
        .map_err(|e| TestCaseError::fail(format!("submit_fact: {e:?}")))?;
        fact_pdas.push(fact);
        assert_conservation_step(&ctx, oracle, vault)?;
    }

    // ---- advance to FactVoting --------------------------------------------
    ctx.warp(WINDOW + 1);
    ctx.send(advance_phase_ix(&ctx, oracle), &[])
        .map_err(|e| TestCaseError::fail(format!("advance_phase: {e:?}")))?;
    prop_assert_eq!(ctx.oracle(oracle).phase, Phase::FactVoting as u8);

    // §9 #1 / disjoint windows: submitting a fact during FactVoting must fail.
    {
        let submitter = Keypair::new();
        ctx.svm.airdrop(&submitter.pubkey(), 1_000_000_000).unwrap();
        let submitter_kass = ctx.fund_kass(&submitter, 1_000);
        let content_hash = [0xEEu8; 32];
        let (fact, _) = TestCtx::fact_pda(&ctx.program_id, &oracle, &content_hash);
        let err = ctx
            .send(
                submit_fact_ix(
                    &ctx,
                    oracle,
                    fact,
                    submitter.pubkey(),
                    submitter_kass,
                    vault,
                    submit_fact_payload(&content_hash, 1_000, b"x"),
                ),
                &[&submitter],
            )
            .unwrap_err()
            .err;
        prop_assert_eq!(err, custom_err(KassandraError::WrongPhase));
    }

    // ---- cast votes (FactVoting window) -----------------------------------
    for (i, f) in s.facts.iter().enumerate() {
        let fact = fact_pdas[i];
        if f.approve > 0 {
            cast_vote(&mut ctx, oracle, vault, fact, VOTE_APPROVE, f.approve)?;
            assert_conservation_step(&ctx, oracle, vault)?;
        }
        if f.duplicate > 0 {
            cast_vote(&mut ctx, oracle, vault, fact, VOTE_DUPLICATE, f.duplicate)?;
            assert_conservation_step(&ctx, oracle, vault)?;
        }
    }

    // ---- finalize facts ----------------------------------------------------
    ctx.warp(WINDOW + 1);
    if s.facts.is_empty() {
        // No-facts dead-end: pass the full proposer set; terminates immediately.
        ctx.send(finalize_facts_ix(&ctx, oracle, &proposer_pdas), &[])
            .map_err(|e| TestCaseError::fail(format!("finalize_facts (no-facts): {e:?}")))?;

        let o = ctx.oracle(oracle);
        prop_assert_eq!(o.phase, Phase::InvalidDeadend as u8);
        prop_assert_eq!(o.surviving_count, 0);
        prop_assert_eq!(o.bond_pool, model.bond_pool);
        // §9 #3 / uniform identity: every proposer is fully slashed and its
        // slashed_amount == bond (the same value added to bond_pool) on the
        // no-facts dead-end path too — no longer scoped to the AiClaim path.
        let mut slash_total = 0u64;
        for (i, pda) in proposer_pdas.iter().enumerate() {
            let p = ctx.proposer(*pda);
            let (exp_slashed, exp_disq, exp_amount) = model.proposer_expect[i];
            prop_assert_eq!(p.slashed != 0, exp_slashed);
            prop_assert_eq!(p.disqualified != 0, exp_disq);
            prop_assert_eq!(p.slashed_amount, exp_amount);
            slash_total += p.slashed_amount;
        }
        // No facts => no rejected-fact stakes; bond_pool == Σ slashed_amount.
        prop_assert_eq!(o.bond_pool, slash_total);
        prop_assert_eq!(model.slash_total, slash_total);
        // §9 #3 conservation under the dead-end BURN: the no-facts terminal burns
        // the slashed bond_pool (= Σ bonds) out of the vault (a non-outcome
        // distributes nothing; the bonds have no recipient), so the vault drains
        // to 0 (no fact stakes exist on this path). `total_oracle_stake` is the
        // pre-burn accumulator and is unchanged. §9 #9 single terminal reached.
        prop_assert_eq!(
            ctx.token_balance(vault),
            0u64,
            "no-facts dead-end drains the vault (bond_pool burned)"
        );
        prop_assert_eq!(
            o.bond_pool,
            o.total_oracle_stake,
            "burned bond_pool == total_oracle_stake (every bond slashed)"
        );
        prop_assert_eq!(model.terminal, Terminal::DeadendNoFacts);
        prop_assert_eq!(model.total_in, o.total_oracle_stake);
        return Ok(());
    }

    ctx.send(finalize_facts_ix(&ctx, oracle, &fact_pdas), &[])
        .map_err(|e| TestCaseError::fail(format!("finalize_facts: {e:?}")))?;
    prop_assert_eq!(ctx.oracle(oracle).phase, Phase::AiClaim as u8);
    // §9 #6: per-fact classification matches the independent quorum rule.
    for (i, fact) in fact_pdas.iter().enumerate() {
        let f = ctx.fact(*fact);
        match model.fact_class[i] {
            FactClass::Agreed => {
                prop_assert_eq!(f.agreed, 1);
                prop_assert_eq!(f.duplicate, 0);
            }
            FactClass::DuplicateDominant => {
                prop_assert_eq!(f.duplicate, 1);
                prop_assert_eq!(f.agreed, 0);
            }
            FactClass::Rejected => {
                prop_assert_eq!(f.agreed, 0);
                prop_assert_eq!(f.duplicate, 0);
            }
        }
    }
    assert_conservation_step(&ctx, oracle, vault)?;

    // ---- submit AI claims (AiClaim window) --------------------------------
    for (i, p) in s.proposers.iter().enumerate() {
        if p.no_show {
            continue;
        }
        let authority = ctx.proposers(oracle)[i].authority.insecure_clone();
        ctx.svm.airdrop(&authority.pubkey(), 1_000_000_000).unwrap();
        let proposer_pda = proposer_pdas[i];
        let (claim, _) = claim_pda(&ctx.program_id, &oracle, &proposer_pda);
        let option = p.claim_raw % model.options_count;
        ctx.send(
            submit_ai_claim_ix(
                &ctx,
                oracle,
                proposer_pda,
                claim,
                authority.pubkey(),
                submit_ai_payload(option),
            ),
            &[&authority],
        )
        .map_err(|e| TestCaseError::fail(format!("submit_ai_claim: {e:?}")))?;
    }
    assert_conservation_step(&ctx, oracle, vault)?;

    // ---- finalize AI claims -----------------------------------------------
    ctx.warp(WINDOW + 1);
    ctx.send(finalize_ai_claims_ix(&ctx, oracle, &proposer_pdas), &[])
        .map_err(|e| TestCaseError::fail(format!("finalize_ai_claims: {e:?}")))?;
    let o = ctx.oracle(oracle);
    prop_assert_eq!(o.phase, Phase::Challenge as u8);
    prop_assert_eq!(o.surviving_count, model.surviving_count);

    // §9 #3 / #4 / #9: per-proposer slash outcome matches the reference; the
    // internal identity bond_pool == Σ slashed_amount + Σ rejected-fact stakes
    // holds (the slashed_amount identity is uniform across all slash paths).
    let mut on_chain_slash_total = 0u64;
    for (i, pda) in proposer_pdas.iter().enumerate() {
        let p = ctx.proposer(*pda);
        let (exp_slashed, exp_disq, exp_amount) = model.proposer_expect[i];
        prop_assert_eq!(p.slashed != 0, exp_slashed);
        prop_assert_eq!(p.disqualified != 0, exp_disq);
        prop_assert_eq!(p.slashed_amount, exp_amount);
        on_chain_slash_total += p.slashed_amount;
    }
    let rejected_stakes: u64 = s
        .facts
        .iter()
        .zip(&model.fact_class)
        .filter(|(_, &c)| c == FactClass::Rejected)
        .map(|(f, _)| f.stake)
        .sum();
    prop_assert_eq!(o.bond_pool, model.bond_pool);
    prop_assert_eq!(
        o.bond_pool,
        on_chain_slash_total + rejected_stakes,
        "bond_pool == Σ slashed_amount + Σ rejected-fact stakes"
    );
    prop_assert_eq!(model.slash_total, on_chain_slash_total);
    assert_conservation_step(&ctx, oracle, vault)?;

    // §9 #1: finalize_oracle attempted before Challenge window elapsed must fail
    // with WindowNotElapsed (we have NOT warped past the challenge window yet).
    {
        let err = ctx
            .send(finalize_oracle_ix(&ctx, oracle, &proposer_pdas), &[])
            .unwrap_err()
            .err;
        prop_assert_eq!(err, custom_err(KassandraError::WindowNotElapsed));
    }

    // ---- finalize oracle (terminal) ---------------------------------------
    ctx.warp(WINDOW + 1);
    ctx.send(finalize_oracle_ix(&ctx, oracle, &proposer_pdas), &[])
        .map_err(|e| TestCaseError::fail(format!("finalize_oracle: {e:?}")))?;

    let o = ctx.oracle(oracle);
    // §9 #7 + #9: terminal state and resolved_option match the reference.
    match &model.terminal {
        Terminal::Resolved(opt) => {
            prop_assert_eq!(o.phase, Phase::Resolved as u8);
            prop_assert_eq!(o.resolved_option, *opt);
            prop_assert!(
                (o.resolved_option as u16) < model.options_count as u16,
                "resolved option must be in range"
            );
        }
        Terminal::DeadendFromOracle => {
            prop_assert_eq!(o.phase, Phase::InvalidDeadend as u8);
            prop_assert_eq!(o.resolved_option, CLAIM_OPTION_NONE);
        }
        Terminal::DeadendNoFacts => unreachable!("facts present on this path"),
    }

    // §9 #9 terminal exclusivity: phase is exactly one terminal state. On
    // Resolved no KASS moved (bonds remain escrowed counters); on InvalidDeadend
    // finalize_oracle BURNED the slashed bond_pool out of the vault (emission is
    // disabled in this arm), leaving exactly the returnable principal.
    prop_assert!(o.phase == Phase::Resolved as u8 || o.phase == Phase::InvalidDeadend as u8);
    if o.phase == Phase::Resolved as u8 {
        assert_conservation_step(&ctx, oracle, vault)?;
    } else {
        prop_assert_eq!(
            ctx.token_balance(vault) + o.bond_pool,
            o.total_oracle_stake,
            "dead-end burned bond_pool out of the vault (vault + bond_pool == total)"
        );
    }
    prop_assert_eq!(model.total_in, o.total_oracle_stake);

    // §9 #1 idempotency / wrong-phase: a second finalize_oracle now fails.
    {
        let err = ctx
            .send(finalize_oracle_ix(&ctx, oracle, &proposer_pdas), &[])
            .unwrap_err()
            .err;
        prop_assert_eq!(err, custom_err(KassandraError::WrongPhase));
    }

    Ok(())
}

/// Cast a vote of `kind`/`stake` on `fact` from a fresh funded voter.
fn cast_vote(
    ctx: &mut TestCtx,
    oracle: Pubkey,
    vault: Pubkey,
    fact: Pubkey,
    kind: u8,
    stake: u64,
) -> Result<(), TestCaseError> {
    let voter = Keypair::new();
    ctx.svm.airdrop(&voter.pubkey(), 1_000_000_000).unwrap();
    let voter_kass = ctx.fund_kass(&voter, stake);
    let (fact_vote, _) = TestCtx::vote_pda(&ctx.program_id, &fact, &voter.pubkey());
    ctx.send(
        vote_fact_ix(
            ctx,
            oracle,
            fact,
            fact_vote,
            voter.pubkey(),
            voter_kass,
            vault,
            vote_payload(kind, stake),
        ),
        &[&voter],
    )
    .map_err(|e| TestCaseError::fail(format!("vote_fact: {e:?}")))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Arm B: plurality / terminal fuzz over an arbitrary post-settlement set
// ---------------------------------------------------------------------------
//
// Arm A can only disqualify via no-show. To fuzz `finalize_oracle`'s plurality
// over an ARBITRARY surviving set (including proposers that submitted a claim but
// were later disqualified by a challenge market), this arm seeds the oracle
// directly into Challenge with a chosen disqualified/surviving partition and
// chosen claim options, then asserts the terminal decision against the same
// independent `ref_plurality`. This broadens §9 #7 / #9 coverage cheaply.

#[derive(Clone, Copy, Debug)]
struct ChallengeProposerGen {
    claim: u8,
    disqualified: bool,
}

fn challenge_proposer_strategy() -> impl Strategy<Value = ChallengeProposerGen> {
    (0u8..4, any::<bool>()).prop_map(|(claim, disqualified)| ChallengeProposerGen {
        claim,
        disqualified,
    })
}

fn run_challenge_terminal(proposers: &[ChallengeProposerGen]) -> Result<(), TestCaseError> {
    let mut ctx = TestCtx::new();
    let specs: Vec<ProposerSpec> = proposers
        .iter()
        .map(|p| ProposerSpec {
            option: p.claim.min(2),
            bond: 1_000,
        })
        .collect();
    let oracle = ctx.seed_disputed_oracle(&specs);
    let vault = ctx.seeded(oracle).stake_vault;
    let pdas: Vec<Pubkey> = ctx.proposers(oracle).iter().map(|p| p.pda).collect();

    let mut surviving_claims = Vec::new();
    let mut surviving_count: u16 = 0;
    for (pda, p) in pdas.iter().zip(proposers) {
        if p.disqualified {
            // Disqualified proposers are skipped by finalize_oracle; mark them and
            // give the no-show sentinel (their claim is irrelevant).
            ctx.set_proposer_disqualified(*pda);
            ctx.set_proposer_claim_option(*pda, CLAIM_OPTION_NONE);
        } else {
            ctx.set_proposer_claim_option(*pda, p.claim);
            surviving_claims.push(p.claim);
            surviving_count += 1;
        }
    }
    ctx.set_surviving_count(oracle, surviving_count);
    ctx.set_phase(oracle, Phase::Challenge);

    ctx.warp(WINDOW + 1);
    ctx.send(finalize_oracle_ix(&ctx, oracle, &pdas), &[])
        .map_err(|e| TestCaseError::fail(format!("finalize_oracle: {e:?}")))?;

    let o = ctx.oracle(oracle);
    match ref_plurality(&surviving_claims) {
        Some(opt) => {
            prop_assert_eq!(o.phase, Phase::Resolved as u8);
            prop_assert_eq!(o.resolved_option, opt);
        }
        None => {
            prop_assert_eq!(o.phase, Phase::InvalidDeadend as u8);
            prop_assert_eq!(o.resolved_option, CLAIM_OPTION_NONE);
        }
    }
    // §9 #9: exactly one terminal phase, vault untouched (counter-only).
    prop_assert!(o.phase == Phase::Resolved as u8 || o.phase == Phase::InvalidDeadend as u8);
    prop_assert_eq!(ctx.token_balance(vault), o.total_oracle_stake);
    Ok(())
}

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

// ===========================================================================
// Arms D + E: PHYSICAL staker-settlement conservation fuzz (Task S5)
// ===========================================================================
//
// Arms A/B/C above are the PRE-settlement (counter-only) invariant fuzz: they
// assert the dispute-core ledger (`stake_vault == total_oracle_stake`, the
// bond_pool identity, plurality) at the terminal counter state, with emission
// DISABLED and NO claims run — exactly as built for Task 13/H6. They are left
// untouched (still green, emission-disabled).
//
// Arms D/E are the POST-settlement physical fuzz added by S5: for a TERMINAL
// oracle with fuzzed bonds / fact stakes / vote kinds / per-fact agreed-vs-
// rejected-vs-duplicate / proposer correct-vs-wrong-vs-flipped-vs-disqualified /
// outcome AND emission enabled with a fuzzed amount, they run EVERY real S2 claim
// (+ S4 close) and assert each payout against an INDEPENDENT reference that
// REIMPLEMENTS the bucket / pro-rata / ceil-slash math (it does NOT call the
// program's `kassandra_program::reward` — that would be circular), plus the
// conservation equation: `Σ payouts + dust == Σ stakes + reward_emission`
// (Resolved) / `Σ payouts == Σ stakes` (InvalidDeadend, emission burned). Every
// payout is sourced from the stake vault; `total_oracle_stake` is never read.
//
// What is REAL vs SEEDED here: the CLAIM + CLOSE instructions (the token movers)
// are REAL, and on the InvalidDeadend arm the emission BURN-BACK is a REAL
// `finalize_oracle`. The terminal RESOLUTION (stamping reward_pool / cohort
// totals) is seeded — the organic stamping path is covered by Arm A + the
// `finalize_*` unit tests + `emissions.rs`. The deadend-after-SETTLED-CHALLENGE-
// with-emission combination flagged in S3 is covered deterministically by
// `settlement_e2e::e2e_deadend_after_settled_challenge_with_emission`.

/// Independent reimplementation of `reward::reward_buckets` (NOT a call to it).
fn ref_buckets(pool: u64, pw: u64, fw: u64, total_correct: u64, total_approved: u64) -> (u64, u64) {
    if total_approved == 0 {
        return (pool, 0);
    }
    if total_correct == 0 {
        return (0, pool);
    }
    let denom = pw as u128 + fw as u128;
    if denom == 0 {
        return (pool, 0);
    }
    let p = pool as u128;
    (
        (p * pw as u128 / denom) as u64,
        (p * fw as u128 / denom) as u64,
    )
}

/// Independent pro-rata reward (floor); 0 when the cohort total is 0.
fn ref_share(stake: u64, bucket: u64, total: u64) -> u64 {
    if total == 0 {
        return 0;
    }
    (stake as u128 * bucket as u128 / total as u128) as u64
}

/// Independent CEIL voter slash `ceil(stake·num/den)` (the on-chain rejected-fact
/// approve-voter slash; ceil keeps the vault solvent against the floor-aggregate
/// bond_pool credit).
fn ref_ceil_slash(stake: u64, num: u64, den: u64) -> u64 {
    if den == 0 {
        return 0;
    }
    ((stake as u128 * num as u128).div_ceil(den as u128)) as u64
}

const PW: u64 = kassandra_program::config::REWARD_PROPOSER_WEIGHT;
const FW: u64 = kassandra_program::config::REWARD_FACT_WEIGHT;
const SLASH_NUM: u64 = 1;
const SLASH_DEN: u64 = 2;
const RESOLVED_OPT: u8 = 1;

/// A fuzzed proposer for the Resolved settlement arm.
#[derive(Clone, Copy, Debug)]
struct SettleProposerGen {
    bond: u64,
    /// Claims the winning option (vs the losing one).
    correct: bool,
    disqualified: bool,
    /// Slashed fraction of the bond in percent (0..=100): the flip slash for a
    /// survivor, or the bond_pool contribution for a disqualified proposer.
    slash_pct: u8,
}

/// A fuzzed fact (+ its votes) for the Resolved settlement arm.
#[derive(Clone, Debug)]
struct SettleFactGen {
    stake: u64,
    /// 0 = agreed, 1 = rejected, 2 = duplicate-dominant.
    class: u8,
    /// (stake, is_approve) per vote.
    votes: Vec<(u64, bool)>,
}

fn settle_proposer_strategy() -> impl Strategy<Value = SettleProposerGen> {
    (1_000u64..3_001, any::<bool>(), any::<bool>(), 0u8..=100).prop_map(
        |(bond, correct, disqualified, slash_pct)| SettleProposerGen {
            bond,
            correct,
            disqualified,
            slash_pct,
        },
    )
}

fn settle_fact_strategy() -> impl Strategy<Value = SettleFactGen> {
    (
        100u64..1_001,
        0u8..3,
        prop::collection::vec((100u64..1_001, any::<bool>()), 0..=2),
    )
        .prop_map(|(stake, class, votes)| SettleFactGen {
            stake,
            class,
            votes,
        })
}

/// Run the full Resolved settlement sweep for one fuzzed scenario and assert the
/// matrix + conservation against the independent reference.
fn run_resolved_settlement(
    proposers: &[SettleProposerGen],
    facts: &[SettleFactGen],
    emission: u64,
) -> Result<(), TestCaseError> {
    // Map the generated scenario to the harness terminal-oracle specs.
    let p_specs: Vec<ClaimProposerSpec> = proposers
        .iter()
        .map(|p| {
            let slashed_amount = (p.bond as u128 * p.slash_pct as u128 / 100) as u64;
            ClaimProposerSpec {
                bond: p.bond,
                claim_option: if p.correct { RESOLVED_OPT } else { 0 },
                disqualified: p.disqualified,
                slashed_amount,
            }
        })
        .collect();
    let f_specs: Vec<ClaimFactSpec> = facts
        .iter()
        .map(|f| ClaimFactSpec {
            stake: f.stake,
            agreed: f.class == 0,
            duplicate: f.class == 2,
            votes: f
                .votes
                .iter()
                .map(|&(stake, approve)| ClaimVoteSpec {
                    stake,
                    kind: if approve {
                        VOTE_APPROVE
                    } else {
                        VOTE_DUPLICATE
                    },
                })
                .collect(),
        })
        .collect();

    // ----- independent reference stamps (recomputed from the scenario) --------
    let mut total_correct = 0u64;
    for p in &p_specs {
        if !p.disqualified && p.claim_option == RESOLVED_OPT {
            total_correct += p.bond;
        }
    }
    let mut total_approved = 0u64;
    for f in &f_specs {
        if f.agreed {
            let approve: u64 = f
                .votes
                .iter()
                .filter(|v| v.kind == VOTE_APPROVE)
                .map(|v| v.stake)
                .sum();
            total_approved += f.stake + approve;
        }
    }
    let mut bond_pool = 0u64;
    for p in &p_specs {
        bond_pool += p.slashed_amount;
    }
    for f in &f_specs {
        if !f.agreed && !f.duplicate {
            let approve: u64 = f
                .votes
                .iter()
                .filter(|v| v.kind == VOTE_APPROVE)
                .map(|v| v.stake)
                .sum();
            bond_pool += f.stake + (approve as u128 * SLASH_NUM as u128 / SLASH_DEN as u128) as u64;
        }
    }
    let reward_pool = bond_pool + emission;
    let (pbucket, fbucket) = ref_buckets(reward_pool, PW, FW, total_correct, total_approved);

    // ----- seed the terminal oracle + fold the emission -----------------------
    let mut ctx = TestCtx::new();
    let seed = ctx.seed_terminal_oracle(
        Phase::Resolved,
        RESOLVED_OPT,
        &p_specs,
        &f_specs,
        SLASH_NUM,
        SLASH_DEN,
    );
    // The seeded resolution stamps must agree with the independent reference.
    let o0 = ctx.oracle(seed.oracle);
    prop_assert_eq!(o0.total_correct_proposer_stake, total_correct);
    prop_assert_eq!(o0.total_approved_fact_stake, total_approved);
    prop_assert_eq!(o0.reward_pool, bond_pool, "pre-emission reward_pool");
    if emission > 0 {
        ctx.fold_reward_emission(seed.oracle, emission);
    }
    let vault_initial = ctx.token_balance(seed.stake_vault);
    prop_assert_eq!(vault_initial, seed.vault_initial + emission);

    let mut total_payout = 0u64;

    // ----- claim every fact vote, then the submitter (votes-first ordering) ----
    for f in &seed.facts {
        let fact = ctx.fact(f.submitter.account);
        let resolved_fact_agreed = fact.agreed == 1;
        let resolved_fact_dup = fact.duplicate == 1;
        for v in &f.votes {
            let vote = ctx.fact_vote(v.account);
            let predicted = if vote.kind == VOTE_DUPLICATE {
                vote.stake
            } else if resolved_fact_agreed {
                vote.stake + ref_share(vote.stake, fbucket, total_approved)
            } else if resolved_fact_dup {
                vote.stake
            } else {
                vote.stake - ref_ceil_slash(vote.stake, SLASH_NUM, SLASH_DEN)
            };
            let ix = ctx.claim_fact_vote_ix(
                seed.oracle,
                seed.nonce,
                v.account,
                f.submitter.account,
                v.dest_kass,
                seed.stake_vault,
                v.authority.pubkey(),
            );
            let res = ctx.send(ix, &[]);
            prop_assert!(
                res.is_ok(),
                "vote claim must not run the vault short: {:?}",
                res
            );
            prop_assert_eq!(
                ctx.token_balance(v.dest_kass),
                predicted,
                "vote payout matches reference"
            );
            total_payout += predicted;
        }
        let predicted_sub = if resolved_fact_agreed {
            fact.stake + ref_share(fact.stake, fbucket, total_approved)
        } else if resolved_fact_dup {
            fact.stake
        } else {
            0
        };
        let ix = ctx.claim_fact_ix(
            seed.oracle,
            seed.nonce,
            f.submitter.account,
            f.submitter.dest_kass,
            seed.stake_vault,
            f.submitter.authority.pubkey(),
        );
        let res = ctx.send(ix, &[]);
        prop_assert!(
            res.is_ok(),
            "submitter claim must not run the vault short: {:?}",
            res
        );
        prop_assert_eq!(
            ctx.token_balance(f.submitter.dest_kass),
            predicted_sub,
            "submitter payout matches reference"
        );
        total_payout += predicted_sub;
    }

    // ----- claim every proposer (+ close its AiClaim) — reward receivers last --
    for p in &seed.proposers {
        let pr = ctx.proposer(p.account);
        let predicted = if pr.disqualified != 0 {
            0
        } else {
            let base = pr.bond - pr.slashed_amount;
            let reward = if pr.claim_option == RESOLVED_OPT {
                ref_share(pr.bond, pbucket, total_correct)
            } else {
                0
            };
            base + reward
        };
        // Seed + close an AiClaim for this proposer (rent-only; honors claims+closes).
        let ai_claim = ctx.seed_ai_claim(seed.oracle, p.account, p.authority.pubkey());
        let ix = ctx.claim_proposer_ix(
            seed.oracle,
            seed.nonce,
            p.account,
            p.dest_kass,
            seed.stake_vault,
            p.authority.pubkey(),
        );
        let res = ctx.send(ix, &[]);
        prop_assert!(
            res.is_ok(),
            "proposer claim must not run the vault short: {:?}",
            res
        );
        prop_assert_eq!(
            ctx.token_balance(p.dest_kass),
            predicted,
            "proposer payout matches reference"
        );
        total_payout += predicted;

        let ix = ctx.close_ai_claim_ix(seed.oracle, ai_claim, p.authority.pubkey());
        prop_assert!(ctx.send(ix, &[]).is_ok(), "close_ai_claim should succeed");
        prop_assert!(ctx.is_closed(ai_claim));
    }

    // ----- CONSERVATION: Σ payouts + dust == Σ stakes + reward_emission --------
    let dust = ctx.token_balance(seed.stake_vault);
    prop_assert_eq!(
        total_payout + dust,
        vault_initial,
        "Σ payouts + dust == Σ stakes + reward_emission"
    );
    // Dust = floor/ceil reward remainders PLUS, for each disqualified proposer, the
    // forfeited-but-uncredited `bond − slashed_amount`. In the real flow that
    // remainder is the `kass_fee` already sent OUT of the vault to the challenger
    // (so it is never in the vault); the seeded model over-funds the vault by it,
    // leaving it as conservation-SAFE dust (an under-pay, never an over-pay). The
    // bound below catches any gross over-retention while allowing that surplus.
    let disq_forfeit_surplus: u64 = p_specs
        .iter()
        .filter(|p| p.disqualified)
        .map(|p| p.bond - p.slashed_amount)
        .sum();
    // Per-voter CEIL excess: a rejected fact credits bond_pool with the FLOOR
    // aggregate `floor(Σapprove·num/den)` but each approve-voter is slashed
    // `ceil(stakeᵢ·num/den)`, so the vault safely retains up to (ceil − floor) per
    // voter (the S2 ceil-slash solvency margin). Allow it in the dust bound.
    let mut ceil_excess: u64 = 0;
    for f in &f_specs {
        if !f.agreed && !f.duplicate {
            for v in &f.votes {
                if v.kind == VOTE_APPROVE {
                    let floor = (v.stake as u128 * SLASH_NUM as u128 / SLASH_DEN as u128) as u64;
                    ceil_excess += ref_ceil_slash(v.stake, SLASH_NUM, SLASH_DEN) - floor;
                }
            }
        }
    }
    prop_assert!(
        dust <= reward_pool + disq_forfeit_surplus + ceil_excess,
        "dust is floor/ceil reward remainder + disqualified forfeit surplus + ceil-slash margin"
    );
    Ok(())
}

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

// ---------------------------------------------------------------------------
// proptest entry points
// ---------------------------------------------------------------------------
//
// Case-count choices: every case rebuilds a fresh LiteSVM and loads the program
// `.so`, which dominates per-case cost, so counts are kept modest to stay fast
// and non-flaky. Arm A drives the whole multi-transaction flow (~10-25 txs/case);
// Arm B is a single finalize transaction. Neither touches the heavy MetaDAO
// market path (see the module header: #8 is covered deterministically elsewhere).

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 96,
        max_shrink_iters: 128,
        .. ProptestConfig::default()
    })]

    /// Arm A — full randomized dispute flow asserts §9 #1/#2/#3/#6/#7/#9.
    #[test]
    fn full_dispute_invariants(s in scenario_strategy()) {
        run_full_dispute(&s)?;
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 160,
        max_shrink_iters: 256,
        .. ProptestConfig::default()
    })]

    /// Arm B — plurality / terminal exclusivity over an arbitrary surviving set
    /// (§9 #7/#9), modelling an arbitrary post-challenge disqualification set.
    #[test]
    fn challenge_terminal_invariants(
        proposers in prop::collection::vec(challenge_proposer_strategy(), 2..=6)
    ) {
        run_challenge_terminal(&proposers)?;
    }
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

proptest! {
    // Arm D drives a full physical claim+close sweep per case (fresh LiteSVM +
    // program deploy + seed + up to ~12 claim/close txs), so the case count is
    // kept modest (48) to stay fast and non-flaky.
    #![proptest_config(ProptestConfig {
        cases: 48,
        max_shrink_iters: 128,
        .. ProptestConfig::default()
    })]

    /// Arm D (Task S5) — RESOLVED physical-settlement conservation with emission.
    /// Every matrix combination (proposer correct/wrong/flipped/disqualified ×
    /// fact agreed/rejected/duplicate × vote approve/duplicate) × a fuzzed
    /// emission, claimed via the REAL S2/S4 instructions and asserted against the
    /// INDEPENDENT reference + `Σ payouts + dust == Σ stakes + reward_emission`.
    #[test]
    fn resolved_settlement_conservation(
        proposers in prop::collection::vec(settle_proposer_strategy(), 1..=3),
        facts in prop::collection::vec(settle_fact_strategy(), 0..=2),
        emission in 0u64..2_001,
    ) {
        run_resolved_settlement(&proposers, &facts, emission)?;
    }
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

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
//! * **#3 KASS conservation (counter-only settlement) — ASSERTED.** Because this
//!   milestone is counter-only (no physical token return/reward/redemption; no
//!   challenge is opened in this harness so no KASS is moved into a MetaDAO
//!   conditional vault), the precise conservation statements are: (a) `stake_vault`
//!   balance `== oracle.total_oracle_stake` at every step (no KASS
//!   created/destroyed; the dispute instructions move nothing); (b)
//!   `total_oracle_stake == Σ(seeded bonds) + Σ(fact submit stakes) + Σ(vote
//!   stakes)`, reconciled against an independent reference ledger; (c) `bond_pool
//!   == Σ(rejected-fact stakes) + Σ(proposer slashes)`, reconciled against the
//!   independently-computed reference bond pool. On the AiClaim path we ALSO
//!   cross-check the documented internal identity `bond_pool == Σ
//!   proposer.slashed_amount + Σ rejected-fact stakes` (see the no-facts-deadend
//!   note below for why it is path-scoped).
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

mod common;
use common::*;

use std::collections::BTreeMap;

use kassandra_program::{
    config::{FLIP_SLASH_DEN, FLIP_SLASH_NUM, THRESHOLD_DEN, THRESHOLD_NUM},
    error::KassandraError,
    instruction::Ix,
    state::{Phase, CLAIM_OPTION_NONE, VOTE_APPROVE, VOTE_DUPLICATE},
};
use proptest::prelude::*;
use solana_sdk::{
    instruction::{AccountMeta, Instruction, InstructionError},
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    system_program,
    transaction::TransactionError,
};
use spl_token::ID as TOKEN_PROGRAM_ID;

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

fn submit_fact_payload(content_hash: &[u8; 32], stake: u64, uri: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(1 + 32 + 8 + 2 + uri.len());
    data.push(Ix::SubmitFact as u8);
    data.extend_from_slice(content_hash);
    data.extend_from_slice(&stake.to_le_bytes());
    data.extend_from_slice(&(uri.len() as u16).to_le_bytes());
    data.extend_from_slice(uri);
    data
}

fn submit_fact_ix(
    ctx: &TestCtx,
    oracle: Pubkey,
    fact: Pubkey,
    submitter: Pubkey,
    submitter_kass: Pubkey,
    vault: Pubkey,
    data: Vec<u8>,
) -> Instruction {
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(fact, false),
            AccountMeta::new(submitter, true),
            AccountMeta::new(submitter_kass, false),
            AccountMeta::new(vault, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data,
    }
}

fn advance_phase_ix(ctx: &TestCtx, oracle: Pubkey) -> Instruction {
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![AccountMeta::new(oracle, false)],
        data: vec![Ix::AdvancePhase as u8],
    }
}

fn vote_payload(kind: u8, stake: u64) -> Vec<u8> {
    let mut data = Vec::with_capacity(1 + 1 + 8);
    data.push(Ix::VoteFact as u8);
    data.push(kind);
    data.extend_from_slice(&stake.to_le_bytes());
    data
}

#[allow(clippy::too_many_arguments)]
fn vote_fact_ix(
    ctx: &TestCtx,
    oracle: Pubkey,
    fact: Pubkey,
    fact_vote: Pubkey,
    voter: Pubkey,
    voter_kass: Pubkey,
    vault: Pubkey,
    data: Vec<u8>,
) -> Instruction {
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(fact, false),
            AccountMeta::new(fact_vote, false),
            AccountMeta::new(voter, true),
            AccountMeta::new(voter_kass, false),
            AccountMeta::new(vault, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data,
    }
}

fn finalize_facts_ix(ctx: &TestCtx, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
    let mut accounts = Vec::with_capacity(1 + tail.len());
    accounts.push(AccountMeta::new(oracle, false));
    for k in tail {
        accounts.push(AccountMeta::new(*k, false));
    }
    Instruction {
        program_id: ctx.program_id,
        accounts,
        data: vec![Ix::FinalizeFacts as u8],
    }
}

fn claim_pda(program_id: &Pubkey, oracle: &Pubkey, proposer: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"claim", oracle.as_ref(), proposer.as_ref()], program_id)
}

fn submit_ai_payload(option: u8) -> Vec<u8> {
    let mut data = Vec::with_capacity(1 + 32 + 32 + 32 + 1);
    data.push(Ix::SubmitAiClaim as u8);
    data.extend_from_slice(&[0xAA; 32]);
    data.extend_from_slice(&[0xBB; 32]);
    data.extend_from_slice(&[0xCC; 32]);
    data.push(option);
    data
}

fn submit_ai_claim_ix(
    ctx: &TestCtx,
    oracle: Pubkey,
    proposer: Pubkey,
    claim: Pubkey,
    authority: Pubkey,
    data: Vec<u8>,
) -> Instruction {
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(proposer, false),
            AccountMeta::new(claim, false),
            AccountMeta::new(authority, true),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data,
    }
}

fn finalize_ai_claims_ix(ctx: &TestCtx, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
    let mut accounts = Vec::with_capacity(1 + tail.len());
    accounts.push(AccountMeta::new(oracle, false));
    for k in tail {
        accounts.push(AccountMeta::new(*k, false));
    }
    Instruction {
        program_id: ctx.program_id,
        accounts,
        data: vec![Ix::FinalizeAiClaims as u8],
    }
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
        // §9 #3 conservation holds; §9 #9 single terminal state reached.
        assert_conservation_step(&ctx, oracle, vault)?;
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

    // §9 #9 terminal exclusivity: phase is exactly one terminal state, and no
    // KASS moved (conservation still holds; bonds remain escrowed counters).
    prop_assert!(o.phase == Phase::Resolved as u8 || o.phase == Phase::InvalidDeadend as u8);
    assert_conservation_step(&ctx, oracle, vault)?;
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
        sum_bonds,
        "stake_vault balance == Σ bonds"
    );
    prop_assert_eq!(
        ctx.token_balance(vault),
        pre.total_oracle_stake,
        "stake_vault balance == total_oracle_stake"
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

    // Conservation still holds after finalize (no token CPI in either branch).
    prop_assert_eq!(ctx.token_balance(vault), o.total_oracle_stake);
    prop_assert_eq!(o.total_oracle_stake, sum_bonds);
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

//! `finalize_facts`: settle the fact-voting round once its window has elapsed.
//!
//! This instruction performs NO token CPI. It only mutates account data and
//! advances a running `Oracle.bond_pool` counter of slashed KASS owed to the
//! pool. The actual KASS stays escrowed in the stake vault; per-staker
//! reward / return / withdrawal (paying out approved-fact stakers, returning
//! duplicate/voter stakes, draining the bond pool) is a DEFERRED later task.
//! `bond_pool` here is purely an accounting counter.
//!
//! # Incremental finalization
//! A dispute may have an unbounded number of facts / proposers, but a single
//! transaction can only carry so many accounts. So finalization is INCREMENTAL:
//! each call settles ANY non-empty subset of the not-yet-settled set, bumping
//! `Oracle.settled_count` (facts) or decrementing `Oracle.surviving_count`
//! (proposers). The phase only advances once the WHOLE set is processed
//! (`settled_count == fact_count`, or `surviving_count == 0`), so a large set
//! can be finalized in chunks across many txs without ever getting stuck.
//!
//! # Behavior
//! Gated to [`Phase::FactVoting`] after the voting window has elapsed.
//!
//! * **No-facts dead-end** (`fact_count == 0`): the tail is a subset of the
//!   oracle's proposers. Each is disqualified + slashed, its bond added to
//!   `bond_pool`, and `surviving_count` decremented. Once `surviving_count`
//!   reaches 0 the oracle terminates in [`Phase::InvalidDeadend`].
//! * **Otherwise**: the tail is a subset of the oracle's facts. Each is:
//!   - duplicate-dominant (`duplicate_stake > approve_stake`) → `duplicate=1`,
//!     not slashed (its stake is returned later).
//!   - agreed (`approve_stake > duplicate_stake` AND
//!     `approve_stake * THRESHOLD_DEN >= dispute_bond_total * THRESHOLD_NUM`)
//!     → `agreed=1`, no bond_pool change (reward is a later claim).
//!   - rejected (neither of the above) → `settled` only, and the FULL
//!     `fact.stake` is added to `bond_pool`. The rejected-fact submitter
//!     forfeits 100% of their fact-submission stake to the pool — this is the
//!     intended penalty. Approve-voter stake settlement on rejected facts is a
//!     separate DEFERRED task.
//!
//! Once `settled_count == fact_count` the oracle advances to
//! [`Phase::AiClaim`] with a fresh window.
//!
//! Each fact/proposer is settled exactly once: an already-`settled` fact (or
//! already-slashed proposer) aborts with [`KassandraError::AlreadySettled`].
//!
//! # Accounts
//! 0. oracle — writable, owned by this program
//! 1. onward — the tail: a non-empty subset of the oracle's proposers
//!    (no-facts case) or facts. Each writable, owned by this program,
//!    belonging to this oracle, distinct within the call.
//!
//! # Instruction payload
//! Empty (after the 1-byte discriminant).

use pinocchio::{
    account_info::AccountInfo, program_error::ProgramError, pubkey::Pubkey, ProgramResult,
};

use crate::{
    clock::{now, require_after_end, require_phase},
    error::KassandraError,
    processor::guards::{load_fact, load_oracle, load_proposer},
    state::{Fact, Oracle, Phase, Proposer},
};

/// A fact is agreed iff approve strictly beats duplicate AND clears the
/// supermajority threshold (snapshotted on the oracle at create_oracle) of the
/// fixed `dispute_bond_total`. u128 intermediates avoid overflow on the
/// cross-multiplication.
fn is_agreed(
    approve_stake: u64,
    duplicate_stake: u64,
    dispute_bond_total: u64,
    threshold_num: u64,
    threshold_den: u64,
) -> bool {
    approve_stake > duplicate_stake
        && (approve_stake as u128) * (threshold_den as u128)
            >= (dispute_bond_total as u128) * (threshold_num as u128)
}

/// Reject if `key` appears in `prior` (distinctness within the call).
fn require_distinct(prior: &[AccountInfo], key: &Pubkey) -> ProgramResult {
    for a in prior {
        if a.key() == key {
            return Err(KassandraError::InvalidAccount.into());
        }
    }
    Ok(())
}

pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], _payload: &[u8]) -> ProgramResult {
    let [oracle_ai, tail @ ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // Owner + size + account_type check, then an owned copy for mutation.
    let mut oracle: Oracle = load_oracle(oracle_ai, program_id)?;

    require_phase(&oracle, Phase::FactVoting)?;
    let now = now()?;
    require_after_end(&oracle, now)?;

    // The fact-approval threshold is undefined without a positive denominator.
    if oracle.dispute_bond_total == 0 {
        return Err(KassandraError::NoDisputeBond.into());
    }

    // At least one account must be supplied to do any work.
    if tail.is_empty() {
        return Err(KassandraError::IncompleteFactSet.into());
    }

    if oracle.fact_count == 0 {
        finalize_no_facts(program_id, oracle_ai, &mut oracle, tail)?;
    } else {
        finalize_with_facts(program_id, oracle_ai, &mut oracle, tail, now)?;
    }

    Ok(())
}

/// No facts ever cleared: slash a subset of proposers into the pool. Once every
/// proposer is slashed (`surviving_count == 0`), terminate in
/// [`Phase::InvalidDeadend`].
fn finalize_no_facts(
    program_id: &Pubkey,
    oracle_ai: &AccountInfo,
    oracle: &mut Oracle,
    proposers: &[AccountInfo],
) -> ProgramResult {
    for (i, p_ai) in proposers.iter().enumerate() {
        require_distinct(&proposers[..i], p_ai.key())?;

        let mut proposer = load_proposer(p_ai, program_id)?;
        if proposer.oracle != *oracle_ai.key() {
            return Err(KassandraError::InvalidAccount.into());
        }
        // Idempotency: each proposer is slashed exactly once.
        if proposer.is_slashed() {
            return Err(KassandraError::AlreadySettled.into());
        }
        // Defensive (unreachable in the real flow, where `disqualified` is always
        // set together with `slashed`): a proposer already disqualified on some
        // other path is already excluded from `surviving_count`. Mark it slashed
        // for idempotency but do NOT re-account into `bond_pool` or re-decrement
        // `surviving_count` — symmetric with `finalize_ai_claims`'s
        // already-disqualified branch.
        if proposer.is_disqualified() {
            proposer.slashed = 1;
            let mut data = p_ai.try_borrow_mut_data()?;
            data[..Proposer::LEN].copy_from_slice(bytemuck::bytes_of(&proposer));
            continue;
        }

        proposer.disqualified = 1;
        proposer.slashed = 1;
        // Record the per-proposer slash so the identity "a proposer's bond_pool
        // contribution == its slashed_amount" holds on EVERY slash path (no-show,
        // flip, challenge-fail, and this no-facts dead-end) — the deferred
        // settlement layer can then reconcile each proposer's loss uniformly,
        // without a path-specific special case.
        proposer.slashed_amount = proposer.bond;
        oracle.bond_pool = oracle
            .bond_pool
            .checked_add(proposer.bond)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        oracle.surviving_count = oracle
            .surviving_count
            .checked_sub(1)
            .ok_or(ProgramError::ArithmeticOverflow)?;

        let mut data = p_ai.try_borrow_mut_data()?;
        data[..Proposer::LEN].copy_from_slice(bytemuck::bytes_of(&proposer));
    }

    // Terminal only once the whole proposer set has been slashed.
    if oracle.surviving_count == 0 {
        oracle.set_phase(Phase::InvalidDeadend);
    }
    write_oracle(oracle_ai, oracle)
}

/// Classify and settle a subset of facts. Once every fact is settled
/// (`settled_count == fact_count`), advance to [`Phase::AiClaim`].
fn finalize_with_facts(
    program_id: &Pubkey,
    oracle_ai: &AccountInfo,
    oracle: &mut Oracle,
    facts: &[AccountInfo],
    now: i64,
) -> ProgramResult {
    for (i, f_ai) in facts.iter().enumerate() {
        require_distinct(&facts[..i], f_ai.key())?;

        // Owner + size + account_type check, then an owned copy for mutation.
        let mut fact = load_fact(f_ai, program_id)?;
        if fact.oracle != *oracle_ai.key() {
            return Err(KassandraError::InvalidAccount.into());
        }
        if fact.is_settled() {
            return Err(KassandraError::AlreadySettled.into());
        }

        if fact.duplicate_stake > fact.approve_stake {
            // Duplicate-dominant: ignored, stake returned later, NOT slashed.
            fact.duplicate = 1;
        } else if is_agreed(
            fact.approve_stake,
            fact.duplicate_stake,
            oracle.dispute_bond_total,
            oracle.threshold_num,
            oracle.threshold_den,
        ) {
            // Agreed: reward is a later claim, no bond_pool change here.
            fact.agreed = 1;
        } else {
            // Rejected: the submitter forfeits 100% of their fact-submission
            // stake to the pool counter.
            oracle.bond_pool = oracle
                .bond_pool
                .checked_add(fact.stake)
                .ok_or(ProgramError::ArithmeticOverflow)?;
        }
        fact.settled = 1;
        oracle.settled_count = oracle
            .settled_count
            .checked_add(1)
            .ok_or(ProgramError::ArithmeticOverflow)?;

        let mut data = f_ai.try_borrow_mut_data()?;
        data[..Fact::LEN].copy_from_slice(bytemuck::bytes_of(&fact));
    }

    // Advance only once the whole fact set has been settled.
    if oracle.settled_count == oracle.fact_count {
        oracle.set_phase(Phase::AiClaim);
        oracle.phase_ends_at = now
            .checked_add(oracle.phase_window)
            .ok_or(ProgramError::ArithmeticOverflow)?;
    }
    write_oracle(oracle_ai, oracle)
}

/// Write the mutated oracle back into its account data.
fn write_oracle(oracle_ai: &AccountInfo, oracle: &Oracle) -> ProgramResult {
    let mut data = oracle_ai.try_borrow_mut_data()?;
    data[..Oracle::LEN].copy_from_slice(bytemuck::bytes_of(oracle));
    Ok(())
}

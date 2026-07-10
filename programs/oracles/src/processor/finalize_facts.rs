//! `finalize_facts`: settle the fact-voting round once its window has elapsed.
//!
//! On the fact-settling path this instruction performs NO token CPI: it only
//! mutates account data and advances a running `Oracle.bond_pool` counter of
//! slashed KASS owed to the pool. Per-staker reward / return / withdrawal of the
//! escrowed KASS is the DEFERRED S2 pull-claim layer; `bond_pool` here is an
//! accounting counter.
//!
//! The ONE exception is the **no-facts dead-end** (`fact_count == 0`): when the
//! last proposer is slashed and the oracle terminates in [`Phase::InvalidDeadend`],
//! it BURNS the accumulated `bond_pool` (= Σ proposer bonds) AND the
//! `reward_emission` back from `stake_vault` to the supply reservoir — symmetric
//! with `finalize_oracle`'s InvalidDeadend burn. A dead-end is a non-outcome: the
//! slashed bonds have no recipient (no winner) and the emission funds no reward,
//! so both are burned (the user-decided deterrent against propose-conflict-then-
//! abandon), leaving the vault drained to dust. Because the burn is signed by the
//! oracle PDA seeds and targets the canonical mint/vault, the instruction takes
//! the same fixed `kass_mint`/`stake_vault`/token-program accounts + `oracle_nonce`
//! payload as `finalize_oracle` (required on BOTH paths; only the no-facts
//! terminal one actually burns).
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
//!     → `agreed=1`, no bond_pool change (reward is a later claim). Accumulates
//!     `oracle.total_approved_fact_stake += fact.stake + fact.approve_stake`
//!     (Task S1): the submitter + approve-voter stake that earns the fact reward
//!     rate at claim time. Stamp only — NO token movement.
//!   - rejected (neither of the above) → `settled` only, and `bond_pool` gains
//!     BOTH the submitter's full slash (`fact.stake`, 100% forfeit) AND the
//!     approve-voters' aggregate slash on this fact
//!     (`fact.approve_stake · fact_vote_slash_num / fact_vote_slash_den`,
//!     u128 floor — Task S1). The approve-voters later reclaim only
//!     `stake·(1 − fact_vote_slash_frac)`; the slashed fraction is added here in
//!     aggregate from the fact's `approve_stake` total (no per-vote iteration).
//!
//! Once `settled_count == fact_count` the oracle advances to
//! [`Phase::AiClaim`] with a fresh window.
//!
//! Each fact/proposer is settled exactly once: an already-`settled` fact (or
//! already-slashed proposer) aborts with [`KassandraError::AlreadySettled`].
//!
//! # Accounts
//! 0. oracle        — writable, owned by this program (mutated; signs the burn).
//! 1. kass_mint     — writable; `== oracle.kass_mint` (the no-facts dead-end burn target).
//! 2. stake_vault   — writable; `== oracle.stake_vault` (bonds/emission burned from here).
//! 3. token program — `pinocchio_token::ID`.
//! 4. onward        — the tail: a non-empty subset of the oracle's proposers
//!    (no-facts case) or facts. Each writable, owned by this program,
//!    belonging to this oracle, distinct within the call.
//!
//! The fixed burn accounts (1-3) are required on BOTH paths (no-facts dead-end
//! and the fact-settling path), like `finalize_oracle`; only the no-facts
//! terminal transition actually burns. Validating the canonical mint/vault is
//! cheap.
//!
//! # Instruction payload (after the 1-byte discriminant), exactly 8 bytes
//! `oracle_nonce: u64 LE` — re-derives + verifies the oracle PDA, whose seeds
//! `[b"oracle", nonce_le, bump]` program-sign the no-facts dead-end burn.

use pinocchio::{
    account::AccountView as AccountInfo, address::Address as Pubkey, cpi::Signer,
    error::ProgramError, ProgramResult,
};
use pinocchio_token::instructions::Burn;

use crate::{
    clock::{now, require_after_end, require_phase},
    error::KassandraError,
    processor::guards::{
        assert_key, load_fact, load_oracle, load_proposer, require_distinct, verify_oracle_pda,
    },
    state::{Fact, Oracle, Phase, Proposer},
};

/// Exact payload length: `oracle_nonce[8]` (re-derives the oracle PDA signer for
/// the no-facts dead-end burn).
const PAYLOAD_LEN: usize = 8;

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

pub fn process(program_id: &Pubkey, accounts: &mut [AccountInfo], payload: &[u8]) -> ProgramResult {
    let [oracle_ai, kass_mint_ai, stake_vault_ai, token_prog_ai, tail @ ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // Owner + size + account_type check, then an owned copy for mutation. Done
    // BEFORE the payload/fixed-account parse so a bad-owner oracle still fails
    // with `InvalidAccount` (dispatch-routing contract).
    let mut oracle: Oracle = load_oracle(oracle_ai, program_id)?;

    require_phase(&oracle, Phase::FactVoting)?;
    let now = now()?;
    require_after_end(&oracle, now)?;

    // The fact-approval threshold is undefined without a positive denominator.
    if oracle.dispute_bond_total == 0 {
        return Err(KassandraError::NoDisputeBond.into());
    }

    // Payload nonce → re-derive + verify the oracle PDA (its seeds sign the
    // no-facts dead-end burn), exactly like finalize_oracle / the S2 claims.
    if payload.len() != PAYLOAD_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let nonce = u64::from_le_bytes(payload[0..8].try_into().unwrap());
    verify_oracle_pda(program_id, oracle_ai, &oracle, nonce)?;

    // Fixed burn accounts (canonical mint + vault + token program). Required on
    // both paths; only the no-facts terminal transition actually burns.
    assert_key(token_prog_ai, &pinocchio_token::ID)?;
    assert_key(kass_mint_ai, &oracle.kass_mint)?;
    assert_key(stake_vault_ai, &oracle.stake_vault)?;

    // At least one account must be supplied to do any work.
    if tail.is_empty() {
        return Err(KassandraError::IncompleteFactSet.into());
    }

    if oracle.fact_count == 0 {
        finalize_no_facts(
            program_id,
            oracle_ai,
            kass_mint_ai,
            stake_vault_ai,
            &mut oracle,
            tail,
            nonce,
        )?;
    } else {
        finalize_with_facts(program_id, oracle_ai, &mut oracle, tail, now)?;
    }

    Ok(())
}

/// No facts ever cleared: slash a subset of proposers into the pool. Once every
/// proposer is slashed (`surviving_count == 0`), terminate in
/// [`Phase::InvalidDeadend`].
#[allow(clippy::too_many_arguments)]
fn finalize_no_facts(
    program_id: &Pubkey,
    oracle_ai: &mut AccountInfo,
    kass_mint_ai: &AccountInfo,
    stake_vault_ai: &AccountInfo,
    oracle: &mut Oracle,
    proposers: &mut [AccountInfo],
    nonce: u64,
) -> ProgramResult {
    for i in 0..proposers.len() {
        let (prior, rest) = proposers.split_at_mut(i);
        let p_ai = &mut rest[0];
        require_distinct(prior, p_ai.address())?;

        let mut proposer = load_proposer(p_ai, program_id)?;
        if proposer.oracle != *oracle_ai.address() {
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
            let mut data = p_ai.try_borrow_mut()?;
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

        let mut data = p_ai.try_borrow_mut()?;
        data[..Proposer::LEN].copy_from_slice(bytemuck::bytes_of(&proposer));
    }

    // Terminal only once the whole proposer set has been slashed: burn the
    // slashed `bond_pool` (= Σ proposer bonds) AND the `reward_emission` back to
    // the reservoir so the dead-end strands nothing (a non-outcome distributes
    // nothing; the slashed bonds have no recipient and the emission funds no
    // reward — both burned, mirroring finalize_oracle). The vault is left empty
    // (no fact stakes exist on this path), so it drains to dust. Both sit in
    // `stake_vault` (token authority == the oracle PDA), so the burn is signed by
    // the oracle seeds. `bond_pool`/`reward_emission` are left as the durable
    // record of what was slashed/minted then burned.
    if oracle.surviving_count == 0 {
        oracle.set_phase(Phase::InvalidDeadend);
        let burn_amount = oracle
            .bond_pool
            .checked_add(oracle.reward_emission)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if burn_amount > 0 {
            let nonce_le = nonce.to_le_bytes();
            let bump_seed = [oracle.bump];
            let seeds = Oracle::signer_seeds(&nonce_le, &bump_seed);
            Burn::new(stake_vault_ai, kass_mint_ai, oracle_ai, burn_amount)
                .invoke_signed(&[Signer::from(&seeds)])?;
        }
    }
    write_oracle(oracle_ai, oracle)
}

/// Classify and settle a subset of facts. Once every fact is settled
/// (`settled_count == fact_count`), advance to [`Phase::AiClaim`].
fn finalize_with_facts(
    program_id: &Pubkey,
    oracle_ai: &mut AccountInfo,
    oracle: &mut Oracle,
    facts: &mut [AccountInfo],
    now: i64,
) -> ProgramResult {
    for i in 0..facts.len() {
        let (prior, rest) = facts.split_at_mut(i);
        let f_ai = &mut rest[0];
        require_distinct(prior, f_ai.address())?;

        // Owner + size + account_type check, then an owned copy for mutation.
        let mut fact = load_fact(f_ai, program_id)?;
        if fact.oracle != *oracle_ai.address() {
            return Err(KassandraError::InvalidAccount.into());
        }
        if fact.is_settled() {
            return Err(KassandraError::AlreadySettled.into());
        }

        if fact.duplicate_stake > fact.approve_stake {
            // Duplicate-dominant: ignored, stake returned later, NOT slashed and
            // NOT counted into the approved-fact reward cohort.
            fact.duplicate = 1;
        } else if is_agreed(
            fact.approve_stake,
            fact.duplicate_stake,
            oracle.dispute_bond_total,
            oracle.threshold_num,
            oracle.threshold_den,
        ) {
            // Agreed: reward is a later (S2) claim, no bond_pool change here.
            // Accumulate the approved-fact reward cohort's total stake — the
            // submitter stake PLUS the aggregate approve-voter stake on this fact,
            // both of which earn the fact_rate at claim time. Stamped on the
            // oracle (S1 totals) for the pull-claims; NO token movement.
            fact.agreed = 1;
            oracle.total_approved_fact_stake = oracle
                .total_approved_fact_stake
                .checked_add(fact.stake)
                .ok_or(ProgramError::ArithmeticOverflow)?
                .checked_add(fact.approve_stake)
                .ok_or(ProgramError::ArithmeticOverflow)?;
        } else {
            // Rejected: the submitter forfeits 100% of their fact-submission
            // stake to the pool counter...
            oracle.bond_pool = oracle
                .bond_pool
                .checked_add(fact.stake)
                .ok_or(ProgramError::ArithmeticOverflow)?;
            // ...AND the approve-voters on this rejected fact forfeit the slash
            // fraction of their stake to the pool. We add the AGGREGATE slash in
            // one shot from the fact's running `approve_stake` total — no per-vote
            // iteration. Each approve-voter later (S2) reclaims only
            // `stake·(1 − fact_vote_slash_frac)`; the slashed fraction is already
            // in `bond_pool` here. u128 floor; the `fact_vote_slash_den > 0`
            // bound is enforced by set_config (and the per-oracle snapshot
            // defaults to a positive denominator), so this never divides by zero.
            let voter_slash = ((fact.approve_stake as u128) * (oracle.fact_vote_slash_num as u128)
                / (oracle.fact_vote_slash_den as u128)) as u64;
            oracle.bond_pool = oracle
                .bond_pool
                .checked_add(voter_slash)
                .ok_or(ProgramError::ArithmeticOverflow)?;
        }
        fact.settled = 1;
        oracle.settled_count = oracle
            .settled_count
            .checked_add(1)
            .ok_or(ProgramError::ArithmeticOverflow)?;

        let mut data = f_ai.try_borrow_mut()?;
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
fn write_oracle(oracle_ai: &mut AccountInfo, oracle: &Oracle) -> ProgramResult {
    let mut data = oracle_ai.try_borrow_mut()?;
    data[..Oracle::LEN].copy_from_slice(bytemuck::bytes_of(oracle));
    Ok(())
}

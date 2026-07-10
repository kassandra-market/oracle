//! `finalize_ai_claims`: settle the AI-claim round once its window has elapsed.
//!
//! Performs NO token CPI: like `finalize_facts`, it only mutates account data
//! and bumps the `Oracle.bond_pool` accounting counter. The escrowed KASS does
//! not move here.
//!
//! # Incremental finalization
//! Mirrors `finalize_facts`: each call processes ANY non-empty subset of the
//! not-yet-ai-finalized proposers, bumping `Oracle.ai_finalized_count`. The
//! phase only advances to [`Phase::Challenge`] once the WHOLE proposer set has
//! been processed (`ai_finalized_count == proposer_count`), so an arbitrarily
//! large set can be finalized in chunks across many txs.
//!
//! # Slash rules (design §5, §7)
//! For each proposer in the tail (gated to [`Phase::AiClaim`], after the window):
//! * **Already disqualified** (defensive — does not occur in the normal flow
//!   before AiClaim): not slashed again, just marked ai-finalized and counted.
//!   It is already excluded from `surviving_count`.
//! * **No-show** (`claim_option == CLAIM_OPTION_NONE`): FULL slash — abandoning
//!   mid-dispute. `slashed=1`, `disqualified=1`, `bond_pool += bond`,
//!   `surviving_count -= 1`.
//! * **Flipped** (`is_flipped()`): PARTIAL slash of `bond * FLIP_SLASH_NUM /
//!   FLIP_SLASH_DEN` into `bond_pool`. The proposer keeps a valid (flipped)
//!   claim that still counts in the plurality, so they REMAIN surviving (not
//!   disqualified, `surviving_count` untouched).
//! * **Submitted, not flipped**: no slash; remains surviving.
//!
//! Each proposer is ai-finalized exactly once: an already-ai-finalized proposer
//! aborts with [`KassandraError::AlreadySettled`].
//!
//! # Accounts
//! 0. oracle — writable, owned by this program
//! 1. onward — the tail: a non-empty subset of the oracle's proposers, each
//!    writable, owned by this program, belonging to this oracle, distinct.
//!
//! # Instruction payload
//! Empty (after the 1-byte discriminant).

use pinocchio::{
    account::AccountView as AccountInfo, address::Address as Pubkey, error::ProgramError,
    ProgramResult,
};

use crate::{
    clock::{now, require_after_end, require_phase},
    error::KassandraError,
    processor::guards::{load_oracle, load_proposer, require_distinct},
    state::{Oracle, Phase, Proposer, CLAIM_OPTION_NONE},
};

pub fn process(
    program_id: &Pubkey,
    accounts: &mut [AccountInfo],
    _payload: &[u8],
) -> ProgramResult {
    let [oracle_ai, tail @ ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // Owner + size + account_type check, then an owned copy for mutation.
    let mut oracle: Oracle = load_oracle(oracle_ai, program_id)?;

    require_phase(&oracle, Phase::AiClaim)?;
    let now = now()?;
    require_after_end(&oracle, now)?;

    // At least one proposer must be supplied to do any work.
    if tail.is_empty() {
        return Err(KassandraError::IncompleteFactSet.into());
    }

    for i in 0..tail.len() {
        let (prior, rest) = tail.split_at_mut(i);
        let p_ai = &mut rest[0];
        require_distinct(prior, p_ai.address())?;

        let mut proposer = load_proposer(p_ai, program_id)?;
        if proposer.oracle != *oracle_ai.address() {
            return Err(KassandraError::InvalidAccount.into());
        }
        // Idempotency: each proposer is ai-finalized exactly once.
        if proposer.is_ai_finalized() {
            return Err(KassandraError::AlreadySettled.into());
        }

        if proposer.is_disqualified() {
            // Already out (e.g. slashed in a prior phase). Don't slash again;
            // just mark + count so the set can complete. Not surviving.
        } else if proposer.claim_option == CLAIM_OPTION_NONE {
            // No-show: full slash. `slashed_amount` == the bond_pool delta.
            proposer.slashed = 1;
            proposer.disqualified = 1;
            proposer.slashed_amount = proposer.bond;
            oracle.bond_pool = oracle
                .bond_pool
                .checked_add(proposer.bond)
                .ok_or(ProgramError::ArithmeticOverflow)?;
            oracle.surviving_count = oracle
                .surviving_count
                .checked_sub(1)
                .ok_or(ProgramError::ArithmeticOverflow)?;
        } else if proposer.is_flipped() {
            // Flip: partial slash, but the (flipped) claim still counts —
            // proposer remains surviving and is NOT disqualified. The SAME
            // value is recorded on the proposer and added to bond_pool.
            let slash = ((proposer.bond as u128) * (oracle.flip_slash_num as u128)
                / (oracle.flip_slash_den as u128)) as u64;
            proposer.slashed = 1;
            proposer.slashed_amount = slash;
            oracle.bond_pool = oracle
                .bond_pool
                .checked_add(slash)
                .ok_or(ProgramError::ArithmeticOverflow)?;
        }
        // else: submitted, not flipped — no slash, remains surviving.

        proposer.ai_finalized = 1;
        oracle.ai_finalized_count = oracle
            .ai_finalized_count
            .checked_add(1)
            .ok_or(ProgramError::ArithmeticOverflow)?;

        let mut data = p_ai.try_borrow_mut()?;
        data[..Proposer::LEN].copy_from_slice(bytemuck::bytes_of(&proposer));
    }

    // Advance only once the whole proposer set has been ai-finalized.
    if oracle.ai_finalized_count == oracle.proposer_count {
        oracle.set_phase(Phase::Challenge);
        oracle.phase_ends_at = now
            .checked_add(oracle.phase_window)
            .ok_or(ProgramError::ArithmeticOverflow)?;
    }

    let mut data = oracle_ai.try_borrow_mut()?;
    data[..Oracle::LEN].copy_from_slice(bytemuck::bytes_of(&oracle));
    Ok(())
}

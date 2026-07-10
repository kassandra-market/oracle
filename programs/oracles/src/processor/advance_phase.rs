//! `advance_phase`: permissionless `FactProposal -> FactVoting` freeze.
//!
//! Once the fact-proposal window has elapsed (`now >= phase_ends_at`), anyone
//! may tick the oracle into the [`Phase::FactVoting`] window. No signer is
//! required: the transition is purely time-gated and deterministic, so leaving
//! it open avoids a liveness dependency on any single keeper.
//!
//! This handles ONLY the `FactProposal -> FactVoting` edge. Every other phase
//! transition is owned by its dedicated finalize instruction (later tasks), so
//! any other starting phase is rejected with [`KassandraError::WrongPhase`].
//!
//! # Accounts
//! 0. oracle — writable, owned by this program
//!
//! # Instruction payload
//! Empty (after the 1-byte discriminant).

use pinocchio::{
    account::AccountView as AccountInfo, address::Address as Pubkey, error::ProgramError,
    ProgramResult,
};

use crate::{
    clock::{now, require_after_end, require_phase},
    state::{Oracle, Phase},
};

pub fn process(
    program_id: &Pubkey,
    accounts: &mut [AccountInfo],
    _payload: &[u8],
) -> ProgramResult {
    let [oracle_ai, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // Owner + size + account_type check, then an owned copy for mutation.
    let mut oracle: Oracle = crate::processor::guards::load_oracle(oracle_ai, program_id)?;

    // Only the FactProposal -> FactVoting edge lives here.
    require_phase(&oracle, Phase::FactProposal)?;

    // The proposal window must have elapsed before voting can be frozen open.
    let now = now()?;
    require_after_end(&oracle, now)?;

    // Freeze the fact set and open a fresh voting window.
    oracle.set_phase(Phase::FactVoting);
    oracle.phase_ends_at = now
        .checked_add(oracle.phase_window)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    {
        let mut data = oracle_ai.try_borrow_mut()?;
        data[..Oracle::LEN].copy_from_slice(bytemuck::bytes_of(&oracle));
    }

    Ok(())
}

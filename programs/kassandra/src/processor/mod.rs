//! Top-level instruction dispatch.
//!
//! [`process`] decodes the leading discriminant byte (see
//! [`crate::instruction::Ix`]) and routes to the matching processor. Each arm
//! is currently a [`KassandraError::NotImplemented`] placeholder; later tasks
//! replace them one at a time with real processors living in
//! `processor/<name>.rs`.

use pinocchio::{
    account_info::AccountInfo, program_error::ProgramError, pubkey::Pubkey, ProgramResult,
};

use crate::{error::KassandraError, instruction::Ix};

pub fn process(
    _program_id: &Pubkey,
    _accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    // First byte = discriminant; the rest is the (not-yet-parsed) payload.
    let (&disc, _payload) = data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;
    let ix = Ix::from_u8(disc).ok_or(ProgramError::InvalidInstructionData)?;

    match ix {
        Ix::SubmitFact
        | Ix::VoteFact
        | Ix::FinalizeFacts
        | Ix::SubmitAiClaim
        | Ix::OpenChallenge
        | Ix::SettleChallenge
        | Ix::FinalizeOracle => Err(KassandraError::NotImplemented.into()),
    }
}

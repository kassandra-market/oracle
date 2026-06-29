//! Small, shared account-validation helpers used by instruction processors.
//!
//! Kept deliberately minimal — only the checks the current processors need.

use pinocchio::{account_info::AccountInfo, pubkey::Pubkey, ProgramResult};

use crate::error::KassandraError;

/// Require that `account` is owned by `program_id`, else
/// [`KassandraError::InvalidAccount`].
pub fn assert_owned_by_program(account: &AccountInfo, program_id: &Pubkey) -> ProgramResult {
    if !account.is_owned_by(program_id) {
        return Err(KassandraError::InvalidAccount.into());
    }
    Ok(())
}

/// Require that `account` signed the transaction, else
/// [`KassandraError::Unauthorized`].
pub fn assert_signer(account: &AccountInfo) -> ProgramResult {
    if !account.is_signer() {
        return Err(KassandraError::Unauthorized.into());
    }
    Ok(())
}

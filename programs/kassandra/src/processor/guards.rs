//! Small, shared account-validation helpers used by instruction processors.
//!
//! Kept deliberately minimal — only the checks the current processors need.

use pinocchio::{
    account_info::AccountInfo,
    instruction::{Seed, Signer},
    program_error::ProgramError,
    pubkey::Pubkey,
    ProgramResult,
};
use pinocchio_system::instructions::CreateAccount;

use crate::{
    error::KassandraError,
    state::{AccountType, Fact, Oracle, Proposer},
};

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

/// Require that `account`'s key equals `expected`, else
/// [`KassandraError::InvalidAccount`]. Used to pin sysvar/program ids and
/// stored references (e.g. `oracle.stake_vault`).
pub fn assert_key(account: &AccountInfo, expected: &Pubkey) -> ProgramResult {
    if account.key() != expected {
        return Err(KassandraError::InvalidAccount.into());
    }
    Ok(())
}

/// Load and validate an [`Oracle`] account: it must be owned by `program_id`,
/// large enough, and carry the [`AccountType::Oracle`] tag. Rejecting on the
/// tag is the type-confusion fix — a `Fact`/`Proposer`/etc. fed into the oracle
/// slot fails here. Returns an owned, alignment-safe copy.
pub fn load_oracle(account: &AccountInfo, program_id: &Pubkey) -> Result<Oracle, ProgramError> {
    assert_owned_by_program(account, program_id)?;
    if account.data_len() < Oracle::LEN {
        return Err(KassandraError::InvalidAccount.into());
    }
    let oracle: Oracle = {
        let data = account.try_borrow_data()?;
        bytemuck::pod_read_unaligned::<Oracle>(&data[..Oracle::LEN])
    };
    if oracle.account_type != AccountType::Oracle.as_u8() {
        return Err(KassandraError::InvalidAccount.into());
    }
    Ok(oracle)
}

/// Load and validate a [`Fact`] account: it must be owned by `program_id`,
/// large enough, and carry the [`AccountType::Fact`] tag (the type-confusion
/// guard, mirroring [`load_oracle`]). Returns an owned, alignment-safe copy.
pub fn load_fact(account: &AccountInfo, program_id: &Pubkey) -> Result<Fact, ProgramError> {
    assert_owned_by_program(account, program_id)?;
    if account.data_len() < Fact::LEN {
        return Err(KassandraError::InvalidAccount.into());
    }
    let fact: Fact = {
        let data = account.try_borrow_data()?;
        bytemuck::pod_read_unaligned::<Fact>(&data[..Fact::LEN])
    };
    if fact.account_type != AccountType::Fact.as_u8() {
        return Err(KassandraError::InvalidAccount.into());
    }
    Ok(fact)
}

/// Load and validate a [`Proposer`] account: it must be owned by `program_id`,
/// large enough, and carry the [`AccountType::Proposer`] tag (the type-confusion
/// guard, mirroring [`load_oracle`]/[`load_fact`]). Returns an owned,
/// alignment-safe copy.
pub fn load_proposer(account: &AccountInfo, program_id: &Pubkey) -> Result<Proposer, ProgramError> {
    assert_owned_by_program(account, program_id)?;
    if account.data_len() < Proposer::LEN {
        return Err(KassandraError::InvalidAccount.into());
    }
    let proposer: Proposer = {
        let data = account.try_borrow_data()?;
        bytemuck::pod_read_unaligned::<Proposer>(&data[..Proposer::LEN])
    };
    if proposer.account_type != AccountType::Proposer.as_u8() {
        return Err(KassandraError::InvalidAccount.into());
    }
    Ok(proposer)
}

/// Create a fresh, rent-exempt, program-owned PDA account, signing with the
/// PDA's `seeds` + `bump`. `payer` funds the rent. De-dups the `CreateAccount`
/// CPI boilerplate shared by every account-creating processor.
pub fn create_pda(
    payer: &AccountInfo,
    pda: &AccountInfo,
    seeds: &[Seed],
    lamports: u64,
    space: usize,
    owner: &Pubkey,
) -> ProgramResult {
    let signer = Signer::from(seeds);
    CreateAccount {
        from: payer,
        to: pda,
        lamports,
        space: space as u64,
        owner,
    }
    .invoke_signed(&[signer])
}

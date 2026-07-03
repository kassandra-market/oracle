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
    state::{AccountType, AiClaim, Fact, Oracle, Proposer, Protocol},
};

/// The canonical `[b"protocol"]` singleton PDA (bump 255) for this program id.
/// Hardcoded so [`load_protocol`] validates the singleton with a cheap key
/// comparison instead of a ~1476-CU `find_program_address` on the hot path
/// (`create_oracle` et al.). Guarded against drift by the `protocol_pda_const`
/// integration test, which re-derives it.
pub const PROTOCOL_PDA: Pubkey =
    pinocchio_pubkey::pubkey!("DUpkpXThaPjDS7TtwwdMJHam7Ki6a8Fg9bmvNf5ggMn6");

/// Canonical bump for [`PROTOCOL_PDA`] (`init_protocol` signs the create with it
/// instead of deriving via `find_program_address`). Guarded by the same test.
pub const PROTOCOL_BUMP: u8 = 255;

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

/// Require that `signer_ai` BOTH signed the transaction AND is the protocol's
/// recorded `dao_authority` (the Squads v4 multisig vault PDA), else
/// [`KassandraError::Unauthorized`]. This is the shared gate for the privileged
/// governance instructions (`set_config` (F3), `resolve_deadend` (F4)): a passed
/// v0.6 futarchy proposal executes through the Squads vault, which signs as this
/// `dao_authority`. Before `set_governance` records it, `dao_authority` is zero,
/// so no real signer matches and the gate denies (no separate "unset" check
/// needed).
pub fn assert_dao_authority(protocol: &Protocol, signer_ai: &AccountInfo) -> ProgramResult {
    assert_signer(signer_ai)?;
    if signer_ai.key() != &protocol.dao_authority {
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

/// Load and validate an [`AiClaim`] account: owned by `program_id`, large
/// enough, and tagged [`AccountType::AiClaim`] (the type-confusion guard,
/// mirroring [`load_oracle`]). Returns an owned, alignment-safe copy.
pub fn load_ai_claim(account: &AccountInfo, program_id: &Pubkey) -> Result<AiClaim, ProgramError> {
    assert_owned_by_program(account, program_id)?;
    if account.data_len() < AiClaim::LEN {
        return Err(KassandraError::InvalidAccount.into());
    }
    let claim: AiClaim = {
        let data = account.try_borrow_data()?;
        bytemuck::pod_read_unaligned::<AiClaim>(&data[..AiClaim::LEN])
    };
    if claim.account_type != AccountType::AiClaim.as_u8() {
        return Err(KassandraError::InvalidAccount.into());
    }
    Ok(claim)
}

/// Load and validate the [`Protocol`] singleton: its address must be the
/// canonical `[b"protocol"]` PDA, owned by `program_id`, large enough, and
/// tagged [`AccountType::Protocol`] (the type-confusion guard, mirroring
/// [`load_oracle`]). Re-deriving + pinning the singleton address here means a
/// future second Protocol-typed account can never be substituted for the real
/// one — every caller (H1 `create_oracle`, H2) gets that defense for free.
/// Returns an owned, alignment-safe copy.
pub fn load_protocol(account: &AccountInfo, program_id: &Pubkey) -> Result<Protocol, ProgramError> {
    // Compare against the precomputed singleton address — the `[b"protocol"]` PDA
    // is fixed for this program id, so we skip the `find_program_address` syscall.
    assert_key(account, &PROTOCOL_PDA)?;
    assert_owned_by_program(account, program_id)?;
    if account.data_len() < Protocol::LEN {
        return Err(KassandraError::InvalidAccount.into());
    }
    let protocol: Protocol = {
        let data = account.try_borrow_data()?;
        bytemuck::pod_read_unaligned::<Protocol>(&data[..Protocol::LEN])
    };
    if protocol.account_type != AccountType::Protocol.as_u8() {
        return Err(KassandraError::InvalidAccount.into());
    }
    Ok(protocol)
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

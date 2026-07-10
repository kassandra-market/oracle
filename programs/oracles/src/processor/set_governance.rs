//! `set_governance`: one-time admin→DAO handoff recording the DAO linkage.
//!
//! Records `dao_authority` (the Squads v4 multisig **vault** PDA — F0 finding
//! #1, NOT a futarchy PDA) and `kass_dao` (the futarchy `Dao` account whose
//! embedded spot AMM is F5's KASS price source) into the [`Protocol`] singleton.
//! Both pubkeys are passed in the PAYLOAD and validated non-zero.
//!
//! # Linkage validation (Task G1)
//! The handoff no longer trusts the payload verbatim — it VALIDATES the real
//! Squads-vault / futarchy-DAO linkage against threaded accounts:
//! - The `kass_dao` ACCOUNT is threaded (read-only). Its key must equal the
//!   payload `kass_dao`, it must be owned by the futarchy program
//!   ([`md6::FUTARCHY_ID`]), and its first 8 bytes must be the `Dao` Anchor
//!   discriminator ([`md6::DAO_ACCOUNT_DISCRIMINATOR`]) — else
//!   [`KassandraError::InvalidFutarchyDao`].
//! - The payload `dao_authority` must equal the Squads v4 multisig **vault** PDA
//!   derived for that DAO (multisig `create_key == kass_dao` → multisig → vault,
//!   index 0; see [`md6::squads_multisig_pda`] / [`md6::squads_vault_pda`]) —
//!   else [`KassandraError::DaoAuthorityMismatch`]. So a bogus or mismatched
//!   handoff is rejected on-chain, and the recorded `dao_authority` is exactly
//!   the DAO's real execution authority.
//!
//! # Trust model (v1, one-shot handoff)
//! - While `governance_set == 0`: callable ONLY by `Protocol.admin` (the
//!   `init_protocol` admin). This is the one-time bootstrap handoff of control
//!   to the DAO.
//! - Once `governance_set == 1`: callable ONLY by the current
//!   `Protocol.dao_authority`, so governance can rotate its own linkage. The
//!   old admin is rejected ([`KassandraError::GovernanceAlreadySet`]).
//!
//! So the trust assumption is: the admin sets the DAO linkage exactly once, and
//! the DAO controls it thereafter (it can rotate itself, never back to the
//! admin).
//!
//! # Mint authority
//! The KASS mint authority is the program PDA `[b"mint_authority"]` (see
//! [`crate::config::MINT_AUTHORITY_SEED`]). F1 only DEFINES that seed; the
//! binding `kass_mint.mint_authority == mint_authority_pda` is asserted at first
//! emission (settlement milestone), since verifying it here would require
//! threading the mint account.
//!
//! # Accounts
//! 0. protocol PDA — writable; the `[b"protocol"]` singleton
//! 1. authority    — signer; `Protocol.admin` pre-handoff, `dao_authority` post
//! 2. kass_dao     — read-only; the futarchy `Dao` account (must equal the
//!    payload `kass_dao`; validated owner + discriminator)
//!
//! # Instruction payload
//! `dao_authority: [u8; 32]` ++ `kass_dao: [u8; 32]` (64 bytes).

use pinocchio::{
    account::AccountView as AccountInfo, address::Address as Pubkey, error::ProgramError,
    ProgramResult,
};

use crate::{
    cpi::metadao_v06 as md6,
    error::KassandraError,
    processor::guards::{assert_key, assert_signer, load_protocol},
    state::Protocol,
};

pub fn process(program_id: &Pubkey, accounts: &mut [AccountInfo], payload: &[u8]) -> ProgramResult {
    let [protocol_ai, authority_ai, kass_dao_ai, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // --- payload ------------------------------------------------------------
    if payload.len() < 64 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let dao_authority: Pubkey = <[u8; 32]>::try_from(&payload[..32]).unwrap().into();
    let kass_dao: Pubkey = <[u8; 32]>::try_from(&payload[32..64]).unwrap().into();
    // Both linkage keys must be non-zero (a zeroed key is the "unset" sentinel).
    if dao_authority == Pubkey::default() || kass_dao == Pubkey::default() {
        return Err(KassandraError::InvalidAccount.into());
    }

    // --- account validation -------------------------------------------------
    assert_signer(authority_ai)?;
    // load_protocol pins the singleton address, owner, length, and type tag.
    let mut protocol = load_protocol(protocol_ai, program_id)?;

    // --- trust model: admin sets once, then dao_authority rotates -----------
    if protocol.is_governance_set() {
        // Post-handoff: only the current DAO authority may rotate the linkage.
        if authority_ai.address() != &protocol.dao_authority {
            return Err(KassandraError::GovernanceAlreadySet.into());
        }
    } else {
        // Pre-handoff: only the init admin may perform the one-time handoff.
        if authority_ai.address() != &protocol.admin {
            return Err(KassandraError::Unauthorized.into());
        }
    }

    // --- validate the real Squads-vault / futarchy-DAO linkage (Task G1) ----
    // (a) The passed `kass_dao` account must be the one named in the payload.
    assert_key(kass_dao_ai, &kass_dao)?;
    // (b) It must be a real futarchy `Dao`: owned by the futarchy program, with
    //     the `Dao` Anchor account discriminator as its first 8 bytes.
    if !kass_dao_ai.owned_by(&md6::FUTARCHY_ID) {
        return Err(KassandraError::InvalidFutarchyDao.into());
    }
    {
        let data = kass_dao_ai.try_borrow()?;
        if data.len() < 8 || data[..8] != md6::DAO_ACCOUNT_DISCRIMINATOR {
            return Err(KassandraError::InvalidFutarchyDao.into());
        }
    }
    // (c) The payload `dao_authority` must be the Squads v4 vault PDA derived for
    //     this DAO (multisig `create_key == kass_dao` → multisig → vault, idx 0).
    let (multisig, _) = md6::squads_multisig_pda(&kass_dao);
    let (vault, _) = md6::squads_vault_pda(&multisig, 0);
    if dao_authority != vault {
        return Err(KassandraError::DaoAuthorityMismatch.into());
    }

    // --- record the linkage -------------------------------------------------
    protocol.dao_authority = dao_authority;
    protocol.kass_dao = kass_dao;
    protocol.governance_set = 1;
    {
        let mut data = protocol_ai.try_borrow_mut()?;
        data[..Protocol::LEN].copy_from_slice(bytemuck::bytes_of(&protocol));
    }

    Ok(())
}

//! `init_protocol`: one-time creation of the [`Protocol`] singleton.
//!
//! Creates the `[b"protocol"]` PDA recording the admin and the canonical
//! KASS/USDC mints (so a later `create_oracle` fee-burn cannot be spoofed with a
//! fake KASS mint), with the fee-EMA state zeroed (genesis is free; the dynamic
//! fee is Task H2). Idempotency is structural: a second call sees a funded /
//! non-empty PDA and fails [`KassandraError::AlreadyInitialized`].
//!
//! # Protocol PDA seeds (CONTRACT)
//! `[b"protocol"]` (singleton), program = [`crate::ID`].
//!
//! # Accounts
//! 0. protocol PDA   — writable, uninitialized (created here)
//! 1. admin          — signer, writable; pays the rent, recorded as `admin`
//! 2. kass_mint      — canonical KASS mint (owned by the SPL token program)
//! 3. usdc_mint      — canonical USDC mint (owned by the SPL token program)
//! 4. system program
//!
//! # Instruction payload
//! None (any trailing bytes are ignored).

use bytemuck::Zeroable;
use pinocchio::{
    account_info::AccountInfo,
    instruction::Seed,
    program_error::ProgramError,
    pubkey::{find_program_address, Pubkey},
    sysvars::{rent::Rent, Sysvar},
    ProgramResult,
};

use crate::{
    error::KassandraError,
    processor::guards::{assert_key, assert_signer, create_pda},
    state::{AccountType, Protocol},
};

pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], _payload: &[u8]) -> ProgramResult {
    let [protocol_ai, admin_ai, kass_mint_ai, usdc_mint_ai, system_prog_ai, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // --- account validation -------------------------------------------------
    assert_signer(admin_ai)?;
    assert_key(system_prog_ai, &pinocchio_system::ID)?;

    // The protocol PDA must be exactly the singleton address.
    let (expected_protocol, bump) = find_program_address(&[b"protocol"], program_id);
    assert_key(protocol_ai, &expected_protocol)?;

    // One-time: reject if the singleton already exists.
    if protocol_ai.lamports() != 0 || !protocol_ai.data_is_empty() {
        return Err(KassandraError::AlreadyInitialized.into());
    }

    // Cheap defense-in-depth: the recorded mints must be SPL token-program
    // accounts (not arbitrary keys), so H1/H2 can trust them as canonical mints.
    if !kass_mint_ai.is_owned_by(&pinocchio_token::ID)
        || !usdc_mint_ai.is_owned_by(&pinocchio_token::ID)
    {
        return Err(KassandraError::InvalidAccount.into());
    }

    // --- create the Protocol account (program-signed) -----------------------
    let rent = Rent::get()?.minimum_balance(Protocol::LEN);
    let bump_seed = [bump];
    let signer_seeds = [Seed::from(b"protocol".as_ref()), Seed::from(&bump_seed)];
    create_pda(
        admin_ai,
        protocol_ai,
        &signer_seeds,
        rent,
        Protocol::LEN,
        program_id,
    )?;

    // --- initialize the Protocol --------------------------------------------
    let mut protocol = Protocol::zeroed();
    protocol.account_type = AccountType::Protocol.as_u8();
    protocol.admin = *admin_ai.key();
    protocol.kass_mint = *kass_mint_ai.key();
    protocol.usdc_mint = *usdc_mint_ai.key();
    protocol.fee_ema = 0;
    protocol.last_creation_unix = 0;
    protocol.bump = bump;
    {
        let mut data = protocol_ai.try_borrow_mut_data()?;
        data.copy_from_slice(bytemuck::bytes_of(&protocol));
    }

    Ok(())
}

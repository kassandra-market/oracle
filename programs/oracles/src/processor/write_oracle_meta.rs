//! `write_oracle_meta` (Ix=23): write the companion `[b"oracle_meta", oracle]`
//! PDA — the plaintext SUBJECT + option LABELS + a `uri`/`uri_hash` for the
//! extended off-chain JSON. Sized to fit, **write-once**, gated to the oracle's
//! recorded `creator`. Lets other programs read the subject/options straight from
//! chain (no indexer, no URL deref); the PDA's rent is reclaimed at `sweep_oracle`.
//!
//! Payload (== the account body after a fixed 34-byte header):
//!   `subject_len u16 ++ subject`
//!   `options_count u8 ++ [ option_len u16 ++ option ]*`
//!   `uri_len u16 ++ uri`
//!   `uri_hash [u8; 32]`
//!
//! Account layout: `account_type u8 ++ bump u8 ++ oracle Pubkey(32) ++ <payload>`.

use pinocchio::{
    account::AccountView as AccountInfo, address::Address as Pubkey, cpi::Seed,
    error::ProgramError, ProgramResult,
};

use crate::{
    error::KassandraError,
    processor::guards::{assert_key, assert_signer, create_pda, load_oracle},
    rent::minimum_rent,
    state::AccountType,
};

// Caps (reject over) — bound the account size + rent.
const MAX_SUBJECT_LEN: usize = 512;
const MAX_OPTION_LEN: usize = 128;
const MAX_URI_LEN: usize = 256;

/// Fixed account header before the payload body: `account_type(1) + bump(1) +
/// oracle(32)`.
const HEADER_LEN: usize = 34;

fn read_u16(p: &[u8], off: &mut usize) -> Result<usize, ProgramError> {
    let b = p
        .get(*off..*off + 2)
        .ok_or(ProgramError::InvalidInstructionData)?;
    *off += 2;
    Ok(u16::from_le_bytes(b.try_into().unwrap()) as usize)
}

fn advance(p: &[u8], off: &mut usize, n: usize) -> Result<(), ProgramError> {
    *off = off
        .checked_add(n)
        .ok_or(ProgramError::InvalidInstructionData)?;
    if *off > p.len() {
        return Err(ProgramError::InvalidInstructionData);
    }
    Ok(())
}

/// Walk the length-prefixed payload, enforce caps + exact structure (no trailing
/// bytes), and return the declared `options_count`.
fn validate(payload: &[u8]) -> Result<u8, ProgramError> {
    let mut off = 0usize;

    // subject
    let subject_len = read_u16(payload, &mut off)?;
    if subject_len == 0 || subject_len > MAX_SUBJECT_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    advance(payload, &mut off, subject_len)?;

    // options
    let options_count = *payload
        .get(off)
        .ok_or(ProgramError::InvalidInstructionData)?;
    off += 1;
    if options_count < 2 {
        return Err(KassandraError::InvalidOptionsCount.into());
    }
    for _ in 0..options_count {
        let option_len = read_u16(payload, &mut off)?;
        if option_len == 0 || option_len > MAX_OPTION_LEN {
            return Err(ProgramError::InvalidInstructionData);
        }
        advance(payload, &mut off, option_len)?;
    }

    // uri (may be empty)
    let uri_len = read_u16(payload, &mut off)?;
    if uri_len > MAX_URI_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    advance(payload, &mut off, uri_len)?;

    // uri_hash
    advance(payload, &mut off, 32)?;

    // reject trailing bytes
    if off != payload.len() {
        return Err(ProgramError::InvalidInstructionData);
    }
    Ok(options_count)
}

pub fn process(program_id: &Pubkey, accounts: &mut [AccountInfo], payload: &[u8]) -> ProgramResult {
    let options_count = validate(payload)?;

    let [creator_ai, oracle_ai, meta_ai, system_prog_ai, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    assert_signer(creator_ai)?;
    assert_key(system_prog_ai, &pinocchio_system::ID)?;

    // Owner/type/size-checked copy; gate to the oracle's recorded creator so a
    // front-runner can't grab the (write-once) meta PDA with bogus content.
    let oracle = load_oracle(oracle_ai, program_id)?;
    assert_key(creator_ai, &oracle.creator)?;
    if options_count != oracle.options_count {
        return Err(KassandraError::InvalidOptionsCount.into());
    }

    // Meta PDA + write-once guard.
    let (expected_meta, bump) =
        Pubkey::find_program_address(&[b"oracle_meta", oracle_ai.address().as_ref()], program_id);
    assert_key(meta_ai, &expected_meta)?;
    if meta_ai.lamports() != 0 || !meta_ai.is_data_empty() {
        return Err(KassandraError::AlreadyInitialized.into());
    }

    // Create the account sized exactly to the header + validated body.
    let space = HEADER_LEN + payload.len();
    let bump_seed = [bump];
    let signer_seeds = [
        Seed::from(b"oracle_meta".as_ref()),
        Seed::from(oracle_ai.address().as_ref()),
        Seed::from(&bump_seed),
    ];
    create_pda(
        creator_ai,
        meta_ai,
        &signer_seeds,
        minimum_rent(space)?,
        space,
        program_id,
    )?;

    // Header + verbatim body.
    {
        let mut data = meta_ai.try_borrow_mut()?;
        data[0] = AccountType::OracleMeta.as_u8();
        data[1] = bump;
        data[2..HEADER_LEN].copy_from_slice(oracle_ai.address().as_ref());
        data[HEADER_LEN..].copy_from_slice(payload);
    }

    Ok(())
}

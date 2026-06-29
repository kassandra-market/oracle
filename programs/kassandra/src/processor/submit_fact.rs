//! `submit_fact`: propose a supporting fact during the `FactProposal` window.
//!
//! Creates a per-`content_hash` [`Fact`] PDA, escrows the submitter's KASS
//! stake into the oracle's stake vault, and bumps the oracle's fact bookkeeping.
//!
//! # Fact PDA seeds (CONTRACT)
//! `[b"fact", oracle_pubkey, content_hash]`, program = [`crate::ID`].
//!
//! # Instruction payload (after the 1-byte discriminant)
//! `content_hash: [u8; 32]` ++ `stake: u64 LE` ++ `uri_len: u16 LE` ++
//! `uri: [u8; uri_len]`. `uri_len` must be `<= 200` and exactly `uri_len`
//! trailing bytes must be present.
//!
//! # Accounts
//! 0. oracle           — writable, owned by this program
//! 1. fact PDA         — writable, uninitialized (created here)
//! 2. submitter        — signer, writable (funds rent + stake authority)
//! 3. submitter KASS   — writable token account, source of the stake
//! 4. stake vault      — writable token account; must equal `oracle.stake_vault`
//! 5. token program
//! 6. system program

use bytemuck::Zeroable;
use pinocchio::{
    account_info::AccountInfo,
    instruction::{Seed, Signer},
    program_error::ProgramError,
    pubkey::{find_program_address, Pubkey},
    sysvars::{rent::Rent, Sysvar},
    ProgramResult,
};
use pinocchio_system::instructions::CreateAccount;
use pinocchio_token::instructions::Transfer;

use crate::{
    clock::{now, require_before_end, require_phase},
    error::KassandraError,
    processor::guards::{assert_owned_by_program, assert_signer},
    state::{Fact, Oracle, Phase},
};

/// Max length of a fact `uri`, matching the on-chain [`Fact::uri`] buffer.
const MAX_URI_LEN: usize = 200;
/// Fixed-size prefix of the payload: `content_hash[32] ++ stake[8] ++ uri_len[2]`.
const HEADER_LEN: usize = 32 + 8 + 2;

/// Parsed `submit_fact` payload borrowing from the instruction data.
struct Args<'a> {
    content_hash: &'a [u8; 32],
    stake: u64,
    uri: &'a [u8],
}

impl<'a> Args<'a> {
    fn parse(payload: &'a [u8]) -> Result<Self, ProgramError> {
        if payload.len() < HEADER_LEN {
            return Err(ProgramError::InvalidInstructionData);
        }
        let content_hash: &[u8; 32] = payload[0..32]
            .try_into()
            .map_err(|_| ProgramError::InvalidInstructionData)?;
        let stake = u64::from_le_bytes(payload[32..40].try_into().unwrap());
        let uri_len = u16::from_le_bytes(payload[40..42].try_into().unwrap()) as usize;
        if uri_len > MAX_URI_LEN {
            return Err(ProgramError::InvalidInstructionData);
        }
        let uri = payload
            .get(HEADER_LEN..HEADER_LEN + uri_len)
            .ok_or(ProgramError::InvalidInstructionData)?;
        // Reject trailing bytes beyond the declared uri.
        if payload.len() != HEADER_LEN + uri_len {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(Self {
            content_hash,
            stake,
            uri,
        })
    }
}

pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], payload: &[u8]) -> ProgramResult {
    let args = Args::parse(payload)?;

    let [oracle_ai, fact_ai, submitter_ai, submitter_kass_ai, vault_ai, token_prog_ai, system_prog_ai, ..] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // --- account validation -------------------------------------------------
    assert_owned_by_program(oracle_ai, program_id)?;
    assert_signer(submitter_ai)?;
    if token_prog_ai.key() != &pinocchio_token::ID {
        return Err(KassandraError::InvalidAccount.into());
    }
    if system_prog_ai.key() != &pinocchio_system::ID {
        return Err(KassandraError::InvalidAccount.into());
    }
    if oracle_ai.data_len() < Oracle::LEN {
        return Err(KassandraError::InvalidAccount.into());
    }

    // Read an owned copy of the oracle for guards + later mutation.
    let mut oracle: Oracle = {
        let data = oracle_ai.try_borrow_data()?;
        bytemuck::pod_read_unaligned::<Oracle>(&data[..Oracle::LEN])
    };

    // Vault must be exactly the one this oracle escrows into.
    if vault_ai.key() != &oracle.stake_vault {
        return Err(KassandraError::InvalidAccount.into());
    }

    // --- phase / window gates -----------------------------------------------
    require_phase(&oracle, Phase::FactProposal)?;
    require_before_end(&oracle, now()?)?;

    // --- fact PDA derivation + duplicate rejection --------------------------
    let (expected_fact, bump) = find_program_address(
        &[
            b"fact",
            oracle_ai.key().as_ref(),
            args.content_hash.as_ref(),
        ],
        program_id,
    );
    if fact_ai.key() != &expected_fact {
        return Err(KassandraError::InvalidAccount.into());
    }
    // An already-funded PDA means this content_hash was submitted before.
    if fact_ai.lamports() != 0 || !fact_ai.data_is_empty() {
        return Err(KassandraError::DuplicateFact.into());
    }

    // --- create the Fact account (program-signed) ---------------------------
    let rent = Rent::get()?.minimum_balance(Fact::LEN);
    let bump_seed = [bump];
    let signer_seeds = [
        Seed::from(b"fact".as_ref()),
        Seed::from(oracle_ai.key().as_ref()),
        Seed::from(args.content_hash.as_ref()),
        Seed::from(&bump_seed),
    ];
    let signer = Signer::from(&signer_seeds[..]);
    CreateAccount {
        from: submitter_ai,
        to: fact_ai,
        lamports: rent,
        space: Fact::LEN as u64,
        owner: program_id,
    }
    .invoke_signed(&[signer])?;

    // --- escrow the stake into the vault (submitter signs as authority) -----
    Transfer {
        from: submitter_kass_ai,
        to: vault_ai,
        authority: submitter_ai,
        amount: args.stake,
    }
    .invoke()?;

    // --- initialize the Fact ------------------------------------------------
    let mut fact = Fact::zeroed();
    fact.oracle = *oracle_ai.key();
    fact.proposer = *submitter_ai.key();
    fact.content_hash = *args.content_hash;
    fact.stake = args.stake;
    fact.uri_len = args.uri.len() as u16;
    fact.bump = bump;
    fact.uri[..args.uri.len()].copy_from_slice(args.uri);
    {
        let mut data = fact_ai.try_borrow_mut_data()?;
        data.copy_from_slice(bytemuck::bytes_of(&fact));
    }

    // --- bump oracle bookkeeping --------------------------------------------
    oracle.fact_count = oracle
        .fact_count
        .checked_add(1)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    oracle.total_oracle_stake = oracle
        .total_oracle_stake
        .checked_add(args.stake)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    {
        let mut data = oracle_ai.try_borrow_mut_data()?;
        data[..Oracle::LEN].copy_from_slice(bytemuck::bytes_of(&oracle));
    }

    Ok(())
}

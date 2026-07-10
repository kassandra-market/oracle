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
    account::AccountView as AccountInfo, address::Address as Pubkey, cpi::Seed,
    error::ProgramError, ProgramResult,
};
use pinocchio_token::instructions::Transfer;

use crate::{
    clock::{now, require_before_end, require_phase},
    error::KassandraError,
    processor::guards::{assert_key, assert_signer, create_pda, load_oracle},
    rent::minimum_rent,
    state::{AccountType, Fact, Oracle, Phase},
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

pub fn process(program_id: &Pubkey, accounts: &mut [AccountInfo], payload: &[u8]) -> ProgramResult {
    let args = Args::parse(payload)?;

    let [oracle_ai, fact_ai, submitter_ai, submitter_kass_ai, vault_ai, token_prog_ai, system_prog_ai, ..] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // --- account validation -------------------------------------------------
    assert_signer(submitter_ai)?;
    assert_key(token_prog_ai, &pinocchio_token::ID)?;
    assert_key(system_prog_ai, &pinocchio_system::ID)?;

    // Owner + size + account_type check, then an owned copy for later mutation.
    let mut oracle: Oracle = load_oracle(oracle_ai, program_id)?;

    // Bootstrapping: the fact stake must clear the oracle's snapshotted
    // activity-scaled floor (0 at genesis / low activity → any stake, incl. 0).
    if args.stake < oracle.min_stake {
        return Err(KassandraError::BelowMinStake.into());
    }

    // Vault must be exactly the one this oracle escrows into.
    assert_key(vault_ai, &oracle.stake_vault)?;

    // --- phase / window gates -----------------------------------------------
    require_phase(&oracle, Phase::FactProposal)?;
    require_before_end(&oracle, now()?)?;

    // --- fact PDA derivation + duplicate rejection --------------------------
    let (expected_fact, bump) = Pubkey::find_program_address(
        &[
            b"fact",
            oracle_ai.address().as_ref(),
            args.content_hash.as_ref(),
        ],
        program_id,
    );
    assert_key(fact_ai, &expected_fact)?;
    // An already-funded PDA means this content_hash was submitted before.
    //
    // KNOWN LIMITATION (deferred): an attacker can grief a specific
    // content_hash by pre-funding its predicted PDA with 1 lamport, which
    // trips this check before the real submitter creates it. The future fix is
    // to allocate via system Allocate + Assign (which tolerates a pre-funded
    // account) instead of CreateAccount; not worth it now.
    if fact_ai.lamports() != 0 || !fact_ai.is_data_empty() {
        return Err(KassandraError::DuplicateFact.into());
    }

    // --- create the Fact account (program-signed) ---------------------------
    let rent = minimum_rent(Fact::LEN)?;
    let bump_seed = [bump];
    let signer_seeds = [
        Seed::from(b"fact".as_ref()),
        Seed::from(oracle_ai.address().as_ref()),
        Seed::from(args.content_hash.as_ref()),
        Seed::from(&bump_seed),
    ];
    create_pda(
        submitter_ai,
        fact_ai,
        &signer_seeds,
        rent,
        Fact::LEN,
        program_id,
    )?;

    // --- escrow the stake into the vault (submitter signs as authority) -----
    Transfer::new(submitter_kass_ai, vault_ai, submitter_ai, args.stake).invoke()?;

    // --- initialize the Fact ------------------------------------------------
    let mut fact = Fact::zeroed();
    fact.account_type = AccountType::Fact.as_u8();
    fact.oracle = *oracle_ai.address();
    fact.proposer = *submitter_ai.address();
    fact.content_hash = *args.content_hash;
    fact.stake = args.stake;
    fact.uri_len = args.uri.len() as u16;
    fact.bump = bump;
    fact.uri[..args.uri.len()].copy_from_slice(args.uri);
    {
        let mut data = fact_ai.try_borrow_mut()?;
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
        let mut data = oracle_ai.try_borrow_mut()?;
        data[..Oracle::LEN].copy_from_slice(bytemuck::bytes_of(&oracle));
    }

    Ok(())
}

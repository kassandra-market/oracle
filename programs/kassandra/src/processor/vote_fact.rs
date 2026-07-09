//! `vote_fact`: stake-weighted approve/duplicate vote on a fact during the
//! `FactVoting` window.
//!
//! Any KASS holder may vote. A vote escrows `stake` KASS into the oracle's
//! stake vault and records a per-`(fact, voter)` [`FactVote`] PDA, so a voter
//! can vote at most once per fact. Voting is **non-exclusive across facts**: a
//! voter may vote on many facts and their full stake counts on each — stake is
//! never split.
//!
//! # FactVote PDA seeds (CONTRACT)
//! `[b"vote", fact_pubkey, voter_pubkey]`, program = [`crate::ID`].
//!
//! # Instruction payload (after the 1-byte discriminant)
//! `kind: u8` (`VOTE_APPROVE = 0` / `VOTE_DUPLICATE = 1`) ++ `stake: u64 LE`.
//! Any other `kind` is rejected as `InvalidInstructionData`; a `stake` below the
//! oracle's snapshotted activity-scaled floor is rejected as
//! [`KassandraError::BelowMinStake`] (the floor is 0 at genesis / low activity, so
//! any stake — including 0 — is then accepted).
//!
//! # Accounts
//! 0. oracle           — writable, owned by this program
//! 1. fact             — writable, owned by this program; `fact.oracle == oracle`
//! 2. fact_vote PDA    — writable, uninitialized (created here)
//! 3. voter            — signer, writable (funds rent + stake authority)
//! 4. voter KASS       — writable token account, source of the stake
//! 5. stake vault      — writable token account; must equal `oracle.stake_vault`
//! 6. token program
//! 7. system program

use bytemuck::Zeroable;
use pinocchio::{
    account::AccountView as AccountInfo, address::Address as Pubkey, cpi::Seed,
    error::ProgramError, ProgramResult,
};
use pinocchio_token::instructions::Transfer;

use crate::{
    clock::{now, require_before_end, require_phase},
    error::KassandraError,
    processor::guards::{assert_key, assert_signer, create_pda, load_fact, load_oracle},
    rent::minimum_rent,
    state::{AccountType, FactVote, Oracle, Phase, VOTE_APPROVE, VOTE_DUPLICATE},
};

/// Parsed `vote_fact` payload.
struct Args {
    kind: u8,
    stake: u64,
}

impl Args {
    fn parse(payload: &[u8]) -> Result<Self, ProgramError> {
        // kind[1] ++ stake[8]
        if payload.len() != 1 + 8 {
            return Err(ProgramError::InvalidInstructionData);
        }
        let kind = payload[0];
        if kind != VOTE_APPROVE && kind != VOTE_DUPLICATE {
            return Err(ProgramError::InvalidInstructionData);
        }
        let stake = u64::from_le_bytes(payload[1..9].try_into().unwrap());
        Ok(Self { kind, stake })
    }
}

pub fn process(program_id: &Pubkey, accounts: &mut [AccountInfo], payload: &[u8]) -> ProgramResult {
    let args = Args::parse(payload)?;

    let [oracle_ai, fact_ai, fact_vote_ai, voter_ai, voter_kass_ai, vault_ai, token_prog_ai, system_prog_ai, ..] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // --- account validation -------------------------------------------------
    assert_signer(voter_ai)?;
    assert_key(token_prog_ai, &pinocchio_token::ID)?;
    assert_key(system_prog_ai, &pinocchio_system::ID)?;

    let mut oracle: Oracle = load_oracle(oracle_ai, program_id)?;
    assert_key(vault_ai, &oracle.stake_vault)?;

    // Bootstrapping: the vote stake must clear the oracle's snapshotted
    // activity-scaled floor (0 at genesis / low activity → any stake, incl. 0).
    if args.stake < oracle.min_stake {
        return Err(KassandraError::BelowMinStake.into());
    }

    // --- phase / window gates -----------------------------------------------
    require_phase(&oracle, Phase::FactVoting)?;
    require_before_end(&oracle, now()?)?;

    // --- fact + binding -----------------------------------------------------
    let mut fact = load_fact(fact_ai, program_id)?;
    // The fact must belong to this oracle.
    if &fact.oracle != oracle_ai.address() {
        return Err(KassandraError::InvalidAccount.into());
    }

    // --- fact_vote PDA derivation + one-vote-per-voter ----------------------
    let (expected_vote, bump) = Pubkey::find_program_address(
        &[
            b"vote",
            fact_ai.address().as_ref(),
            voter_ai.address().as_ref(),
        ],
        program_id,
    );
    assert_key(fact_vote_ai, &expected_vote)?;
    // An already-funded/initialized PDA means this voter already voted here.
    if fact_vote_ai.lamports() != 0 || !fact_vote_ai.is_data_empty() {
        return Err(KassandraError::DuplicateVote.into());
    }

    // --- create the FactVote account (program-signed) -----------------------
    let rent = minimum_rent(FactVote::LEN)?;
    let bump_seed = [bump];
    let signer_seeds = [
        Seed::from(b"vote".as_ref()),
        Seed::from(fact_ai.address().as_ref()),
        Seed::from(voter_ai.address().as_ref()),
        Seed::from(&bump_seed),
    ];
    create_pda(
        voter_ai,
        fact_vote_ai,
        &signer_seeds,
        rent,
        FactVote::LEN,
        program_id,
    )?;

    // --- escrow the stake into the vault (voter signs as authority) ---------
    Transfer::new(voter_kass_ai, vault_ai, voter_ai, args.stake).invoke()?;

    // --- initialize the FactVote --------------------------------------------
    let mut vote = FactVote::zeroed();
    vote.account_type = AccountType::FactVote.as_u8();
    vote.fact = *fact_ai.address();
    vote.voter = *voter_ai.address();
    vote.stake = args.stake;
    vote.kind = args.kind;
    vote.bump = bump;
    {
        let mut data = fact_vote_ai.try_borrow_mut()?;
        data.copy_from_slice(bytemuck::bytes_of(&vote));
    }

    // --- update the fact tally ----------------------------------------------
    if args.kind == VOTE_APPROVE {
        fact.approve_stake = fact
            .approve_stake
            .checked_add(args.stake)
            .ok_or(ProgramError::ArithmeticOverflow)?;
    } else {
        fact.duplicate_stake = fact
            .duplicate_stake
            .checked_add(args.stake)
            .ok_or(ProgramError::ArithmeticOverflow)?;
    }
    {
        let mut data = fact_ai.try_borrow_mut()?;
        data[..crate::state::Fact::LEN].copy_from_slice(bytemuck::bytes_of(&fact));
    }

    // --- bump oracle quorum denominator -------------------------------------
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

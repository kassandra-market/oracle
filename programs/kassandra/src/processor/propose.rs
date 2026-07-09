//! `propose`: register a proposal against an oracle during its proposal window.
//!
//! After the creation-time `deadline`, anyone registers a proposal = a
//! categorical `option` + a KASS `bond`. The bond is escrowed into the oracle's
//! stake vault and one [`Proposer`] PDA per (oracle, authority) is created. The
//! `MAX_PROPOSERS` cap is enforced ON-CHAIN here: this is the liveness guarantee
//! that keeps the one-shot `finalize_oracle` inside a single transaction's
//! account-lock budget (an oversized proposer set would otherwise brick the
//! oracle in a later phase).
//!
//! # Proposer PDA seeds (CONTRACT)
//! `[b"proposer", oracle_pubkey, authority_pubkey]`, program = [`crate::ID`].
//!
//! # Deadline gate + window logic (design §3)
//! `now >= oracle.deadline` (else [`KassandraError::DeadlineNotReached`]), then:
//! * `now < phase_ends_at` → normal, accept.
//! * `now >= phase_ends_at` AND `proposer_count == 0` → the seeding first
//!   proposal after an EMPTY window: accept AND extend `phase_ends_at = now +
//!   PROPOSAL_WINDOW` so others can still conflict (no Unresolved-from-emptiness).
//! * `now >= phase_ends_at` AND `proposer_count > 0` → window closed →
//!   [`KassandraError::ProposalWindowClosed`] (caller must `finalize_proposals`).
//!
//! # Accounts
//! 0. oracle            — writable, owned by this program
//! 1. proposer PDA      — writable, uninitialized (created here)
//! 2. authority         — signer, writable (funds rent + bond-transfer authority)
//! 3. authority KASS    — writable token account, source of the bond
//! 4. stake vault       — writable token account; must equal `oracle.stake_vault`
//! 5. token program
//! 6. system program
//!
//! # Instruction payload (after the 1-byte discriminant), exactly 9 bytes
//! `option: u8` ++ `bond: u64 LE`.

use bytemuck::Zeroable;
use pinocchio::{
    account::AccountView as AccountInfo, address::Address as Pubkey, cpi::Seed,
    error::ProgramError, ProgramResult,
};
use pinocchio_token::instructions::Transfer;

use crate::{
    clock::{now, require_phase},
    config::MAX_PROPOSERS,
    error::KassandraError,
    processor::guards::{assert_key, assert_signer, create_pda, load_oracle},
    rent::minimum_rent,
    state::{AccountType, Oracle, Phase, Proposer, CLAIM_OPTION_NONE},
};

/// Exact payload length: `option[1] ++ bond[8]`.
const PAYLOAD_LEN: usize = 9;

pub fn process(program_id: &Pubkey, accounts: &mut [AccountInfo], payload: &[u8]) -> ProgramResult {
    // --- payload parse (exact length) --------------------------------------
    if payload.len() != PAYLOAD_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let option = payload[0];
    let bond = u64::from_le_bytes(payload[1..9].try_into().unwrap());

    let [oracle_ai, proposer_ai, authority_ai, authority_kass_ai, vault_ai, token_prog_ai, system_prog_ai, ..] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // --- signer + program ids ----------------------------------------------
    assert_signer(authority_ai)?;
    assert_key(token_prog_ai, &pinocchio_token::ID)?;
    assert_key(system_prog_ai, &pinocchio_system::ID)?;

    // Owner + size + account_type check, then an owned copy for later mutation.
    let mut oracle: Oracle = load_oracle(oracle_ai, program_id)?;

    // Bootstrapping: the bond must clear the oracle's snapshotted activity-scaled
    // floor. At genesis / low activity the floor is 0, so a 0 bond (a weightless
    // proposer — still counted by plurality) is accepted; the floor grows with
    // creation activity to re-price Sybil registration once KASS circulates.
    if bond < oracle.min_stake {
        return Err(KassandraError::BelowMinStake.into());
    }

    // Vault must be exactly the one this oracle escrows into.
    assert_key(vault_ai, &oracle.stake_vault)?;

    // --- phase / deadline / window gates -----------------------------------
    require_phase(&oracle, Phase::Proposal)?;
    let now_ts = now()?;
    if now_ts < oracle.deadline {
        return Err(KassandraError::DeadlineNotReached.into());
    }
    // Window logic: a closed window with existing proposers rejects; an empty
    // window re-opens for the seeding first proposal. `phase_ends_at` is only
    // mutated in the seeding branch and is written back with the counts below.
    if now_ts >= oracle.phase_ends_at {
        if oracle.proposer_count == 0 {
            oracle.phase_ends_at = now_ts
                .checked_add(oracle.proposal_window)
                .ok_or(ProgramError::ArithmeticOverflow)?;
        } else {
            return Err(KassandraError::ProposalWindowClosed.into());
        }
    }

    // --- cap (on-chain liveness guarantee) ---------------------------------
    if oracle.proposer_count >= MAX_PROPOSERS {
        return Err(KassandraError::TooManyProposers.into());
    }

    // --- semantic validations ----------------------------------------------
    if option >= oracle.options_count {
        return Err(KassandraError::InvalidOptionsCount.into());
    }

    // --- proposer PDA derivation + duplicate rejection ---------------------
    let (expected_proposer, bump) = Pubkey::find_program_address(
        &[
            b"proposer",
            oracle_ai.address().as_ref(),
            authority_ai.address().as_ref(),
        ],
        program_id,
    );
    assert_key(proposer_ai, &expected_proposer)?;
    // An already-funded PDA means this authority already registered.
    //
    // KNOWN LIMITATION (deferred, same mechanism as submit_fact's duplicate
    // check): an attacker can grief by pre-funding this predicted PDA with 1
    // lamport, tripping this check before the real registration. It is NARROWER
    // here — the PDA is keyed by `authority`, so it can only block one specific,
    // known authority (not an arbitrary content_hash). The future fix is to
    // allocate via system Allocate + Assign (which tolerates a pre-funded
    // account) instead of CreateAccount; not worth it now.
    if proposer_ai.lamports() != 0 || !proposer_ai.is_data_empty() {
        return Err(KassandraError::DuplicateProposer.into());
    }

    // Defensive: the bond source must be a KASS token account on this oracle's
    // canonical mint. The SPL Transfer additionally proves the authority
    // (signer) owns/delegates it.
    {
        let data = authority_kass_ai.try_borrow()?;
        if data.len() < 32 {
            return Err(KassandraError::InvalidAccount.into());
        }
        if data[0..32] != oracle.kass_mint.to_bytes() {
            return Err(KassandraError::InvalidAccount.into());
        }
    }

    // --- escrow the bond into the vault (authority signs as authority) ------
    // NOTE: this does Transfer-then-create_pda, the reverse of submit_fact's
    // create_pda-then-Transfer. The divergence is insignificant — both run in
    // one atomic instruction, so either order fully reverts on any failure.
    Transfer::new(authority_kass_ai, vault_ai, authority_ai, bond).invoke()?;

    // --- create the Proposer account (program-signed) -----------------------
    let rent = minimum_rent(Proposer::LEN)?;
    let bump_seed = [bump];
    let signer_seeds = [
        Seed::from(b"proposer".as_ref()),
        Seed::from(oracle_ai.address().as_ref()),
        Seed::from(authority_ai.address().as_ref()),
        Seed::from(&bump_seed),
    ];
    create_pda(
        authority_ai,
        proposer_ai,
        &signer_seeds,
        rent,
        Proposer::LEN,
        program_id,
    )?;

    // --- initialize the Proposer --------------------------------------------
    let mut proposer = Proposer::zeroed();
    proposer.account_type = AccountType::Proposer.as_u8();
    proposer.oracle = *oracle_ai.address();
    proposer.authority = *authority_ai.address();
    proposer.bond = bond;
    proposer.original_option = option;
    // CONTRACT: not yet AI-claimed — must be the loud sentinel, NOT zero.
    proposer.claim_option = CLAIM_OPTION_NONE;
    proposer.disqualified = 0;
    proposer.slashed = 0;
    proposer.flipped = 0;
    proposer.bump = bump;
    proposer.ai_finalized = 0;
    proposer.slashed_amount = 0;
    {
        let mut data = proposer_ai.try_borrow_mut()?;
        data.copy_from_slice(bytemuck::bytes_of(&proposer));
    }

    // --- bump oracle bookkeeping (checked) ----------------------------------
    oracle.proposer_count = oracle
        .proposer_count
        .checked_add(1)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    oracle.surviving_count = oracle
        .surviving_count
        .checked_add(1)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    oracle.total_oracle_stake = oracle
        .total_oracle_stake
        .checked_add(bond)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    {
        let mut data = oracle_ai.try_borrow_mut()?;
        data[..Oracle::LEN].copy_from_slice(bytemuck::bytes_of(&oracle));
    }

    Ok(())
}

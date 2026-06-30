//! `claim_proposer` / `claim_fact` / `claim_fact_vote` (Task S2): the first
//! PHYSICAL payouts of the staker-settlement layer.
//!
//! Each is a PERMISSIONLESS, per-staker PULL: anyone may crank a claim for one
//! account, but the KASS lands in the claimant-owner's token account. A claim
//! (1) requires the oracle to be TERMINAL ([`Phase::Resolved`] or
//! [`Phase::InvalidDeadend`]); (2) loads + type-checks the claimant account and
//! binds it to this oracle; (3) computes the entitlement from the matrix below;
//! (4) transfers exactly that KASS from `stake_vault` (program-signed by the
//! oracle PDA) to the claimant-owner's KASS account; and (5) CLOSES the claimant
//! account, draining its rent lamports to the owner. Idempotent BY CLOSURE — a
//! second claim finds the account gone (zero lamports → reaped) and fails the
//! owner/type guard.
//!
//! # CONSERVATION CONTRACT
//! Every payout is sourced from the real `stake_vault` balance + the per-account
//! `slashed_amount` ledger + the resolution-time stamps (`reward_pool`,
//! `total_correct_proposer_stake`, `total_approved_fact_stake`). NOTHING reads
//! `total_oracle_stake` (an idealized accumulator, NOT physical KASS — a
//! successful challenge / external donation can desync it). Σ entitlements ≤
//! `stake_vault` balance; the floor-division dust stays in the vault.
//!
//! # Per-actor matrix
//! Cohort reward buckets are computed once from the oracle's resolution stamps
//! via [`crate::reward::reward_buckets`]; rewards apply ONLY on `Resolved`.
//!
//! * **claim_proposer**
//!   - `InvalidDeadend` → `bond` (full return; no reward).
//!   - `Resolved` + `is_disqualified()` → `bond − slashed_amount` (the
//!     `slashed_amount` already funded `bond_pool`).
//!   - `Resolved` + surviving + `claim_option == resolved_option` (correct) →
//!     `bond + proposer_reward(bond, proposer_bucket, total_correct)`.
//!   - `Resolved` + surviving + `claim_option != resolved_option`
//!     (wrong-but-survived) → `bond` (no reward).
//!     NOTE: a surviving proposer's `slashed_amount` is assumed 0 in scope (only
//!     disqualified proposers are slashed here); the flip-slash cross-path
//!     (surviving + `slashed_amount > 0`) is out of S2 scope.
//! * **claim_fact** (submitter)
//!   - `InvalidDeadend` → `stake`.
//!   - `Resolved` + `is_agreed()` → `stake + fact_reward(stake, fact_bucket,
//!     total_approved)`.
//!   - `Resolved` + `is_duplicate()` → `stake`.
//!   - `Resolved` + rejected → `0` (the stake funded `bond_pool`; still close +
//!     reclaim rent to the submitter).
//! * **claim_fact_vote** (the fact is loaded to read its disposition)
//!   - `InvalidDeadend` → `stake`.
//!   - `Resolved` + `kind == VOTE_DUPLICATE` (any fact) → `stake` (never
//!     slashed/rewarded).
//!   - `Resolved` + `kind == VOTE_APPROVE` + fact `is_agreed()` → `stake +
//!     fact_reward(stake, fact_bucket, total_approved)`.
//!   - `Resolved` + `kind == VOTE_APPROVE` + fact `is_duplicate()` → `stake`
//!     (approve-voter on a duplicate-dominant fact: no reward, no slash).
//!   - `Resolved` + `kind == VOTE_APPROVE` + fact rejected → `stake −
//!     floor(stake · fact_vote_slash_num / fact_vote_slash_den)` (the slashed
//!     fraction already funded `bond_pool`).
//!
//! # Accounts (per claim)
//! `claim_proposer` / `claim_fact`:
//! 0. oracle           — read-only; owned by this program, re-derived from the
//!    payload nonce; the SPL authority of `stake_vault` (signs the payout).
//! 1. claimant         — writable; the `Proposer`/`Fact` account, CLOSED here.
//! 2. dest_kass        — writable; KASS token account, `mint == oracle.kass_mint`
//!    and `owner == claimant.authority` (proposer.authority / fact.proposer).
//! 3. stake_vault      — writable; `== oracle.stake_vault` (the payout source).
//! 4. rent_recipient   — writable; `== claimant.authority` (reclaimed rent).
//! 5. token program.
//!
//! `claim_fact_vote` inserts the fact at index 2 and shifts the rest:
//! 0. oracle, 1. fact_vote(w, closed), 2. fact(w — its running voter-stake
//!    total is decremented, NOT closed), 3. dest_kass(w), 4. stake_vault(w),
//! 5. rent_recipient(w == fact_vote.voter), 6. token program.
//!
//! # Fact-close ordering (no griefing)
//! `claim_fact` CLOSES the `Fact`, but `claim_fact_vote` must read the Fact's
//! disposition. So the submitter's claim runs LAST: each `claim_fact_vote`
//! decrements the Fact's `approve_stake`/`duplicate_stake` running total, and
//! `claim_fact` refuses to close while either is non-zero
//! ([`KassandraError::VotersOutstanding`]). No one can strand a voter by closing
//! the Fact early.
//!
//! # Instruction payload (after the 1-byte discriminant)
//! `oracle_nonce: u64 LE` (exactly 8 bytes) — re-derives + verifies the oracle
//! PDA signer seeds, identical to `settle_challenge`.

use pinocchio::{
    account_info::AccountInfo,
    instruction::{Seed, Signer},
    program_error::ProgramError,
    pubkey::{find_program_address, Pubkey},
    ProgramResult,
};
use pinocchio_token::instructions::Transfer;

use crate::{
    error::KassandraError,
    processor::guards::{assert_key, load_fact, load_oracle},
    reward,
    state::{AccountType, Fact, FactVote, Oracle, Phase, Proposer, VOTE_DUPLICATE},
};

/// Exact payload length: `oracle_nonce[8]`.
const PAYLOAD_LEN: usize = 8;

/// Minimum size of an SPL token account (`spl_token::state::Account::LEN`).
const SPL_TOKEN_ACCOUNT_LEN: usize = 165;
/// `spl_token::state::Account.owner` byte offset.
const SPL_TOKEN_OWNER_OFFSET: usize = 32;

/// Read a 32-byte pubkey at `offset` from `data`.
fn read_pubkey(data: &[u8], offset: usize) -> Result<Pubkey, ProgramError> {
    data.get(offset..offset + 32)
        .and_then(|s| s.try_into().ok())
        .ok_or(KassandraError::InvalidAccount.into())
}

/// Assert `account` is an SPL token account on `expected_mint` whose token
/// authority is `expected_owner`, else [`KassandraError::InvalidAccount`]. Binds
/// the payout destination to the claimant's authority so a cranker cannot
/// redirect the entitlement to an account they control.
fn assert_token_account(
    account: &AccountInfo,
    expected_mint: &Pubkey,
    expected_owner: &Pubkey,
) -> ProgramResult {
    if !account.is_owned_by(&pinocchio_token::ID) {
        return Err(KassandraError::InvalidAccount.into());
    }
    let data = account.try_borrow_data()?;
    if data.len() < SPL_TOKEN_ACCOUNT_LEN {
        return Err(KassandraError::InvalidAccount.into());
    }
    let mint = read_pubkey(&data, 0)?;
    let owner = read_pubkey(&data, SPL_TOKEN_OWNER_OFFSET)?;
    if &mint != expected_mint || &owner != expected_owner {
        return Err(KassandraError::InvalidAccount.into());
    }
    Ok(())
}

/// Decode the terminal phase, rejecting any non-terminal oracle with
/// [`KassandraError::WrongPhase`]. Returns `true` iff the oracle is
/// [`Phase::Resolved`] (so the caller knows whether rewards apply).
fn require_terminal(oracle: &Oracle) -> Result<bool, ProgramError> {
    match oracle.phase().ok_or(KassandraError::InvalidAccount)? {
        Phase::Resolved => Ok(true),
        Phase::InvalidDeadend => Ok(false),
        _ => Err(KassandraError::WrongPhase.into()),
    }
}

/// Re-derive the oracle PDA from `nonce` and verify it matches `oracle_ai` +
/// `oracle.bump`, exactly like `settle_challenge`. The PDA is the SPL authority
/// of `stake_vault`; the returned nonce bytes seed the program signature.
fn verify_oracle_pda(
    program_id: &Pubkey,
    oracle_ai: &AccountInfo,
    oracle: &Oracle,
    nonce: u64,
) -> ProgramResult {
    let (derived, bump) = find_program_address(&[b"oracle", &nonce.to_le_bytes()], program_id);
    if &derived != oracle_ai.key() || bump != oracle.bump {
        return Err(KassandraError::InvalidAccount.into());
    }
    Ok(())
}

/// Transfer `amount` KASS from `stake_vault` → `dest`, program-signed by the
/// oracle PDA (`[b"oracle", nonce_le, [bump]]`). A zero amount is a no-op (a
/// rejected fact submitter still closes + reclaims rent). Then CLOSE `claimant`,
/// draining its rent lamports to `rent_recipient` and zeroing its data so a
/// second claim finds nothing.
#[allow(clippy::too_many_arguments)]
fn payout_and_close(
    oracle_ai: &AccountInfo,
    stake_vault: &AccountInfo,
    dest: &AccountInfo,
    claimant: &AccountInfo,
    rent_recipient: &AccountInfo,
    nonce: u64,
    bump: u8,
    amount: u64,
) -> ProgramResult {
    if amount > 0 {
        let nonce_le = nonce.to_le_bytes();
        let bump_seed = [bump];
        let seeds = [
            Seed::from(b"oracle".as_ref()),
            Seed::from(nonce_le.as_ref()),
            Seed::from(&bump_seed),
        ];
        Transfer {
            from: stake_vault,
            to: dest,
            authority: oracle_ai,
            amount,
        }
        .invoke_signed(&[Signer::from(&seeds)])?;
    }

    // Drain rent lamports to the recipient, then zero the account (data len /
    // lamports / owner). Done in this order so the instruction stays balanced.
    {
        let mut from = claimant.try_borrow_mut_lamports()?;
        let mut to = rent_recipient.try_borrow_mut_lamports()?;
        *to = to
            .checked_add(*from)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        *from = 0;
    }
    claimant.close()
}

/// `value · num / den` (u128, floor). `den == 0` (defended; the snapshot keeps
/// it positive) yields 0 so the entitlement degrades to the full stake rather
/// than dividing by zero.
fn slash_amount(value: u64, num: u64, den: u64) -> u64 {
    if den == 0 {
        return 0;
    }
    ((value as u128) * (num as u128) / (den as u128)) as u64
}

// ---------------------------------------------------------------------------
// claim_proposer
// ---------------------------------------------------------------------------

pub fn claim_proposer(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    if payload.len() != PAYLOAD_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let nonce = u64::from_le_bytes(payload[0..8].try_into().unwrap());

    let [oracle_ai, proposer_ai, dest_kass_ai, stake_vault_ai, rent_recipient_ai, token_prog_ai, ..] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    assert_key(token_prog_ai, &pinocchio_token::ID)?;

    let oracle = load_oracle(oracle_ai, program_id)?;
    let resolved = require_terminal(&oracle)?;
    verify_oracle_pda(program_id, oracle_ai, &oracle, nonce)?;
    assert_key(stake_vault_ai, &oracle.stake_vault)?;

    // Load + bind the proposer (type guard + this-oracle membership).
    let proposer = load_proposer_checked(proposer_ai, program_id, oracle_ai.key())?;
    assert_token_account(dest_kass_ai, &oracle.kass_mint, &proposer.authority)?;
    assert_key(rent_recipient_ai, &proposer.authority)?;

    let amount = if !resolved {
        // InvalidDeadend: full bond return, no reward.
        proposer.bond
    } else if proposer.is_disqualified() {
        // The slashed_amount already funded bond_pool.
        proposer.bond.saturating_sub(proposer.slashed_amount)
    } else if proposer.claim_option == oracle.resolved_option {
        // Correct survivor: bond + pro-rata proposer-cohort reward.
        let (proposer_bucket, _) = reward::reward_buckets(
            oracle.reward_pool,
            oracle.reward_proposer_weight,
            oracle.reward_fact_weight,
            oracle.total_correct_proposer_stake,
            oracle.total_approved_fact_stake,
        );
        let r = reward::proposer_reward(
            proposer.bond,
            proposer_bucket,
            oracle.total_correct_proposer_stake,
        );
        proposer
            .bond
            .checked_add(r)
            .ok_or(ProgramError::ArithmeticOverflow)?
    } else {
        // Wrong-but-survived: bond, no reward.
        proposer.bond
    };

    payout_and_close(
        oracle_ai,
        stake_vault_ai,
        dest_kass_ai,
        proposer_ai,
        rent_recipient_ai,
        nonce,
        oracle.bump,
        amount,
    )
}

/// Load + type-check a [`Proposer`] and require it belongs to `oracle`.
fn load_proposer_checked(
    account: &AccountInfo,
    program_id: &Pubkey,
    oracle: &Pubkey,
) -> Result<Proposer, ProgramError> {
    let proposer = crate::processor::guards::load_proposer(account, program_id)?;
    if &proposer.oracle != oracle {
        return Err(KassandraError::InvalidAccount.into());
    }
    Ok(proposer)
}

// ---------------------------------------------------------------------------
// claim_fact
// ---------------------------------------------------------------------------

pub fn claim_fact(program_id: &Pubkey, accounts: &[AccountInfo], payload: &[u8]) -> ProgramResult {
    if payload.len() != PAYLOAD_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let nonce = u64::from_le_bytes(payload[0..8].try_into().unwrap());

    let [oracle_ai, fact_ai, dest_kass_ai, stake_vault_ai, rent_recipient_ai, token_prog_ai, ..] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    assert_key(token_prog_ai, &pinocchio_token::ID)?;

    let oracle = load_oracle(oracle_ai, program_id)?;
    let resolved = require_terminal(&oracle)?;
    verify_oracle_pda(program_id, oracle_ai, &oracle, nonce)?;
    assert_key(stake_vault_ai, &oracle.stake_vault)?;

    let fact = load_fact(fact_ai, program_id)?;
    if &fact.oracle != oracle_ai.key() {
        return Err(KassandraError::InvalidAccount.into());
    }
    // The fact's submitter authority is `fact.proposer`.
    assert_token_account(dest_kass_ai, &oracle.kass_mint, &fact.proposer)?;
    assert_key(rent_recipient_ai, &fact.proposer)?;

    // The submitter claim CLOSES the Fact, but every `claim_fact_vote` must read
    // the Fact's disposition first. So the submitter must claim LAST: refuse to
    // close while any voter stake is still unclaimed (each `claim_fact_vote`
    // decrements these running totals as a voter claims).
    if fact.approve_stake != 0 || fact.duplicate_stake != 0 {
        return Err(KassandraError::VotersOutstanding.into());
    }

    let amount = fact_submitter_entitlement(&oracle, &fact, resolved)?;

    payout_and_close(
        oracle_ai,
        stake_vault_ai,
        dest_kass_ai,
        fact_ai,
        rent_recipient_ai,
        nonce,
        oracle.bump,
        amount,
    )
}

/// Entitlement for a fact SUBMITTER (see the module matrix).
fn fact_submitter_entitlement(
    oracle: &Oracle,
    fact: &Fact,
    resolved: bool,
) -> Result<u64, ProgramError> {
    if !resolved {
        return Ok(fact.stake); // InvalidDeadend: full stake.
    }
    if fact.is_agreed() {
        let (_, fact_bucket) = reward::reward_buckets(
            oracle.reward_pool,
            oracle.reward_proposer_weight,
            oracle.reward_fact_weight,
            oracle.total_correct_proposer_stake,
            oracle.total_approved_fact_stake,
        );
        let r = reward::fact_reward(fact.stake, fact_bucket, oracle.total_approved_fact_stake);
        let total = fact
            .stake
            .checked_add(r)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        return Ok(total);
    }
    if fact.is_duplicate() {
        return Ok(fact.stake); // Duplicate-dominant: stake returned, no reward.
    }
    Ok(0) // Rejected submitter: 100% forfeit (still close + reclaim rent).
}

// ---------------------------------------------------------------------------
// claim_fact_vote
// ---------------------------------------------------------------------------

pub fn claim_fact_vote(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    if payload.len() != PAYLOAD_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let nonce = u64::from_le_bytes(payload[0..8].try_into().unwrap());

    let [oracle_ai, vote_ai, fact_ai, dest_kass_ai, stake_vault_ai, rent_recipient_ai, token_prog_ai, ..] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    assert_key(token_prog_ai, &pinocchio_token::ID)?;

    let oracle = load_oracle(oracle_ai, program_id)?;
    let resolved = require_terminal(&oracle)?;
    verify_oracle_pda(program_id, oracle_ai, &oracle, nonce)?;
    assert_key(stake_vault_ai, &oracle.stake_vault)?;

    // FactVote carries no oracle field; bind it through the fact:
    // vote.fact == fact_ai and fact.oracle == oracle.
    let vote = load_fact_vote(vote_ai, program_id)?;
    let mut fact = load_fact(fact_ai, program_id)?;
    if &vote.fact != fact_ai.key() || &fact.oracle != oracle_ai.key() {
        return Err(KassandraError::InvalidAccount.into());
    }
    assert_token_account(dest_kass_ai, &oracle.kass_mint, &vote.voter)?;
    assert_key(rent_recipient_ai, &vote.voter)?;

    let amount = if !resolved {
        // InvalidDeadend: full stake.
        vote.stake
    } else if vote.kind == VOTE_DUPLICATE {
        // Duplicate-voter: never slashed or rewarded, on any fact.
        vote.stake
    } else if fact.is_agreed() {
        // Approve-voter on an agreed fact: stake + pro-rata fact reward.
        let (_, fact_bucket) = reward::reward_buckets(
            oracle.reward_pool,
            oracle.reward_proposer_weight,
            oracle.reward_fact_weight,
            oracle.total_correct_proposer_stake,
            oracle.total_approved_fact_stake,
        );
        let r = reward::fact_reward(vote.stake, fact_bucket, oracle.total_approved_fact_stake);
        vote.stake
            .checked_add(r)
            .ok_or(ProgramError::ArithmeticOverflow)?
    } else if fact.is_duplicate() {
        // Approve-voter on a duplicate-dominant fact: stake, no reward, no slash.
        vote.stake
    } else {
        // Approve-voter on a rejected fact: the slashed fraction already funded
        // bond_pool; reclaim only the remainder.
        let slash = slash_amount(
            vote.stake,
            oracle.fact_vote_slash_num,
            oracle.fact_vote_slash_den,
        );
        vote.stake.saturating_sub(slash)
    };

    // Decrement the fact's running voter-stake total so the submitter's
    // `claim_fact` can tell when every voter has claimed (and only THEN close
    // the Fact). This keeps the Fact alive for all voters' disposition reads.
    // `saturating_sub` defends against any stray double-count; in the normal
    // flow `approve_stake`/`duplicate_stake` is exactly Σ voter stakes.
    if vote.kind == VOTE_DUPLICATE {
        fact.duplicate_stake = fact.duplicate_stake.saturating_sub(vote.stake);
    } else {
        fact.approve_stake = fact.approve_stake.saturating_sub(vote.stake);
    }
    {
        let mut data = fact_ai.try_borrow_mut_data()?;
        data[..Fact::LEN].copy_from_slice(bytemuck::bytes_of(&fact));
    }

    payout_and_close(
        oracle_ai,
        stake_vault_ai,
        dest_kass_ai,
        vote_ai,
        rent_recipient_ai,
        nonce,
        oracle.bump,
        amount,
    )
}

/// Load + type-check a [`FactVote`] (owner == program, size, tag).
fn load_fact_vote(account: &AccountInfo, program_id: &Pubkey) -> Result<FactVote, ProgramError> {
    crate::processor::guards::assert_owned_by_program(account, program_id)?;
    if account.data_len() < FactVote::LEN {
        return Err(KassandraError::InvalidAccount.into());
    }
    let vote: FactVote = {
        let data = account.try_borrow_data()?;
        bytemuck::pod_read_unaligned::<FactVote>(&data[..FactVote::LEN])
    };
    if vote.account_type != AccountType::FactVote.as_u8() {
        return Err(KassandraError::InvalidAccount.into());
    }
    Ok(vote)
}

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
//! * **claim_proposer** — UNIFORM base `bond − slashed_amount` (any slash already
//!   funded `bond_pool`), plus the cohort reward only when Resolved + surviving +
//!   correct: `entitlement = (bond − slashed_amount) + (resolved &&
//!   !is_disqualified() && claim_option == resolved_option ? proposer_reward(bond,
//!   proposer_bucket, total_correct) : 0)`. So:
//!   - `InvalidDeadend` → `bond − slashed_amount` (= `bond` for an unslashed
//!     proposer; a flip-slashed survivor that tied into a dead-end keeps only the
//!     un-slashed remainder — never the full bond).
//!   - `Resolved` + `is_disqualified()` → `bond − slashed_amount`, no reward.
//!   - `Resolved` + surviving + correct → `(bond − slashed_amount) +
//!     proposer_reward(...)` (= `bond + reward` for an honest survivor;
//!     `bond − flip_slash + reward` for a flip-slashed-but-correct survivor).
//!   - `Resolved` + surviving + wrong → `bond − slashed_amount`, no reward.
//! * **claim_fact** (submitter) — disposition-based on BOTH terminal phases; the
//!   reward applies ONLY on `Resolved`. On `InvalidDeadend` the slashed
//!   `bond_pool` (incl. rejected-fact stakes) was BURNED out of `stake_vault` at
//!   finalize, so a rejected submitter must forfeit (0) to stay solvent.
//!   - `is_agreed()` → `stake + (resolved ? fact_reward(...) : 0)`.
//!   - `is_duplicate()` → `stake` (either phase).
//!   - rejected → `0` (either phase; the stake funded the now-burned `bond_pool`;
//!     still close + reclaim rent to the submitter).
//! * **claim_fact_vote** (the fact is loaded to read its disposition) —
//!   disposition-based on BOTH terminal phases; reward ONLY on `Resolved`.
//!   - `kind == VOTE_DUPLICATE` (any fact) → `stake` (never slashed/rewarded).
//!   - `kind == VOTE_APPROVE` + fact `is_agreed()` → `stake + (resolved ?
//!     fact_reward(...) : 0)`.
//!   - `kind == VOTE_APPROVE` + fact `is_duplicate()` → `stake` (no reward/slash).
//!   - `kind == VOTE_APPROVE` + fact rejected → `stake − floor(stake ·
//!     fact_vote_slash_num / fact_vote_slash_den)` (either phase; the slashed
//!     fraction funded the now-burned `bond_pool`).
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
    account::AccountView as AccountInfo, address::Address as Pubkey, cpi::Signer,
    error::ProgramError, ProgramResult,
};
use pinocchio_token::instructions::Transfer;

use crate::{
    error::KassandraError,
    processor::guards::{
        assert_key, assert_token_account, drain_lamports, load_fact, load_oracle, require_terminal,
        verify_oracle_pda,
    },
    reward,
    state::{
        AccountType, Fact, FactVote, Oracle, Phase, Proposer, CLAIM_OPTION_NONE, VOTE_DUPLICATE,
    },
};

/// Exact payload length: `oracle_nonce[8]`.
const PAYLOAD_LEN: usize = 8;

/// Require a terminal oracle ([`require_terminal`]) and report whether it is
/// [`Phase::Resolved`] (so the caller knows whether rewards apply).
///
/// # `resolve_deadend` (F4) oracles — no special-casing
/// An oracle force-resolved from `InvalidDeadend` → `Resolved` by the DAO
/// (`resolve_deadend`, F4) carries `reward_pool == 0` and zero cohort totals
/// (`finalize_oracle` only stamps those on the organic Resolved branch; F4 just
/// flips the phase + sets `resolved_option`). So `resolved == true` here but
/// every reward term is 0 → claims pay **non-slashed principal only**, no
/// rewards: IDENTICAL economics to the plain `InvalidDeadend` branch. This is
/// exactly the dead-end settlement rule (a non-outcome distributes nothing): the
/// slashed `bond_pool` + the `reward_emission` were already BURNED out of
/// `stake_vault` at the InvalidDeadend finalize site (`finalize_oracle` /
/// `finalize_no_facts`), so the vault holds only the returnable principal whether
/// or not governance later flips the phase to `Resolved`. No marker / no
/// claim-path branch on "resolved-from-dead-end" is needed — the `reward_pool ==
/// 0` stamp already makes both terminal phases pay identically.
fn is_resolved(oracle: &Oracle) -> Result<bool, ProgramError> {
    require_terminal(oracle)?;
    Ok(oracle.phase() == Some(Phase::Resolved))
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
    claimant: &mut AccountInfo,
    rent_recipient: &mut AccountInfo,
    nonce: u64,
    bump: u8,
    amount: u64,
) -> ProgramResult {
    if amount > 0 {
        let nonce_le = nonce.to_le_bytes();
        let bump_seed = [bump];
        let seeds = Oracle::signer_seeds(&nonce_le, &bump_seed);
        Transfer::new(stake_vault, dest, oracle_ai, amount)
            .invoke_signed(&[Signer::from(&seeds)])?;
    }

    // Drain rent lamports to the recipient, then zero the account (data len /
    // lamports / owner). Done in this order so the instruction stays balanced.
    drain_lamports(claimant, rent_recipient)?;
    claimant.close()
}

/// Per-voter rejected-fact slash: `ceil(value · num / den)` in u128. `den == 0`
/// (defended; the snapshot keeps it positive) yields 0 so the entitlement
/// degrades to the full stake rather than dividing by zero.
///
/// # Why CEIL (conservation, not just rounding)
/// `finalize_facts` credits `bond_pool` with the AGGREGATE
/// `floor(Σ approve_stake · num/den)` for the rejected fact, and that whole
/// credit is later paid out as rewards. If each voter were slashed
/// `floor(stakeᵢ · num/den)`, then `Σ floor(stakeᵢ·r) ≤ floor(Σ stakeᵢ · r)` —
/// the vault could physically retain LESS than the bond_pool credit, shorting
/// the last reward claimant. Slashing each voter `ceil(stakeᵢ·r)` instead gives
/// `Σ ceil(stakeᵢ·r) ≥ (Σ stakeᵢ)·r ≥ floor(Σ·r)`, so the vault is never short;
/// any excess is conservation-safe sub-unit dust. `ceil = (v·num + den − 1)/den`.
fn slash_amount(value: u64, num: u64, den: u64) -> u64 {
    if den == 0 {
        return 0;
    }
    let scaled = (value as u128) * (num as u128) + (den as u128 - 1);
    (scaled / den as u128) as u64
}

// ---------------------------------------------------------------------------
// claim_proposer
// ---------------------------------------------------------------------------

pub fn claim_proposer(
    program_id: &Pubkey,
    accounts: &mut [AccountInfo],
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
    let resolved = is_resolved(&oracle)?;
    verify_oracle_pda(program_id, oracle_ai, &oracle, nonce)?;
    assert_key(stake_vault_ai, &oracle.stake_vault)?;

    // Load + bind the proposer (type guard + this-oracle membership).
    let proposer = load_proposer_checked(proposer_ai, program_id, oracle_ai.address())?;
    assert_token_account(dest_kass_ai, &oracle.kass_mint, &proposer.authority)?;
    assert_key(rent_recipient_ai, &proposer.authority)?;

    // Base return per proposer:
    //  * DISQUALIFIED → 0: a disqualified proposer FORFEITS the whole bond. It has
    //    been fully distributed already — into `bond_pool` (`slashed_amount`) AND,
    //    on a CHALLENGE disqualify, a `kass_fee = bond − slashed_amount` was paid
    //    out of `stake_vault` to the challenger by `settle_challenge`. So
    //    `bond − slashed_amount` would over-pay the fraudster exactly that
    //    already-gone `kass_fee` → vault shortfall for the last claimant. Forfeit
    //    everything. (No-show / no-facts dead-end set `slashed_amount == bond`, so
    //    this is a no-op there; it corrects only the challenge-disqualify row.)
    //  * SURVIVOR → `bond − slashed_amount`. Any survivor slash (flip) already
    //    funded `bond_pool`, so deducting it prevents the flip-survivor double-pay
    //    (full bond returned AND the slash paid out as rewards). Honest survivor →
    //    `slashed_amount == 0` → full `bond`; flip-slashed survivor → `bond − flip`
    //    (applies on BOTH terminal phases — a flipped proposer is NOT disqualified
    //    and can survive to Resolved OR tie into InvalidDeadend).
    // The reward (Resolved + surviving + correct only) keeps `bond` as its
    // pro-rata weight, matching S1's `total_correct_proposer_stake = Σ bond`, so
    // `Σ(bond − slashed) + reward_pool = Σbond − bond_pool + bond_pool = Σbond`.
    let base = if proposer.is_disqualified() {
        0
    } else {
        proposer.bond.saturating_sub(proposer.slashed_amount)
    };
    // "Correct" = the proposer backed the resolved option. On the disputed path
    // that is the AI `claim_option`; on the uncontested (all-agree) path proposers
    // never submitted an AI claim (`claim_option == CLAIM_OPTION_NONE`), so fall
    // back to their `original_option`. A no-show in a DISPUTED oracle also carries
    // `claim_option == NONE`, but it is always disqualified (excluded below), so
    // this fallback rewards ONLY the uncontested cohort.
    let backed_resolved = proposer.claim_option == oracle.resolved_option
        || (proposer.claim_option == CLAIM_OPTION_NONE
            && proposer.original_option == oracle.resolved_option);
    let reward = if resolved && !proposer.is_disqualified() && backed_resolved {
        let (proposer_bucket, _) = reward::reward_buckets(
            oracle.reward_pool,
            oracle.reward_proposer_weight,
            oracle.reward_fact_weight,
            oracle.total_correct_proposer_stake,
            oracle.total_approved_fact_stake,
        );
        reward::proposer_reward(
            proposer.bond,
            proposer_bucket,
            oracle.total_correct_proposer_stake,
        )
    } else {
        0
    };
    let amount = base
        .checked_add(reward)
        .ok_or(ProgramError::ArithmeticOverflow)?;

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

pub fn claim_fact(
    program_id: &Pubkey,
    accounts: &mut [AccountInfo],
    payload: &[u8],
) -> ProgramResult {
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
    let resolved = is_resolved(&oracle)?;
    verify_oracle_pda(program_id, oracle_ai, &oracle, nonce)?;
    assert_key(stake_vault_ai, &oracle.stake_vault)?;

    let fact = load_fact(fact_ai, program_id)?;
    if &fact.oracle != oracle_ai.address() {
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

/// Entitlement for a fact SUBMITTER (see the module matrix). The fact's
/// disposition (agreed / duplicate / rejected) is applied on BOTH terminal
/// phases; only the reward (Resolved only) differs. On `InvalidDeadend` the
/// reward is 0 (reward_pool == 0) AND, crucially, a REJECTED submitter forfeits
/// (returns 0) — its stake funded `bond_pool`, which the InvalidDeadend finalize
/// site BURNED out of the vault, so returning it would short the vault.
fn fact_submitter_entitlement(
    oracle: &Oracle,
    fact: &Fact,
    resolved: bool,
) -> Result<u64, ProgramError> {
    if fact.is_agreed() {
        let r = if resolved {
            let (_, fact_bucket) = reward::reward_buckets(
                oracle.reward_pool,
                oracle.reward_proposer_weight,
                oracle.reward_fact_weight,
                oracle.total_correct_proposer_stake,
                oracle.total_approved_fact_stake,
            );
            reward::fact_reward(fact.stake, fact_bucket, oracle.total_approved_fact_stake)
        } else {
            0 // InvalidDeadend: no reward distribution.
        };
        return fact
            .stake
            .checked_add(r)
            .ok_or(ProgramError::ArithmeticOverflow);
    }
    if fact.is_duplicate() {
        return Ok(fact.stake); // Duplicate-dominant: stake returned, no reward/slash.
    }
    Ok(0) // Rejected submitter: 100% forfeit on both phases (still close + reclaim rent).
}

// ---------------------------------------------------------------------------
// claim_fact_vote
// ---------------------------------------------------------------------------

pub fn claim_fact_vote(
    program_id: &Pubkey,
    accounts: &mut [AccountInfo],
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
    let resolved = is_resolved(&oracle)?;
    verify_oracle_pda(program_id, oracle_ai, &oracle, nonce)?;
    assert_key(stake_vault_ai, &oracle.stake_vault)?;

    // FactVote carries no oracle field; bind it through the fact:
    // vote.fact == fact_ai and fact.oracle == oracle.
    let vote = load_fact_vote(vote_ai, program_id)?;
    let mut fact = load_fact(fact_ai, program_id)?;
    if &vote.fact != fact_ai.address() || &fact.oracle != oracle_ai.address() {
        return Err(KassandraError::InvalidAccount.into());
    }
    assert_token_account(dest_kass_ai, &oracle.kass_mint, &vote.voter)?;
    assert_key(rent_recipient_ai, &vote.voter)?;

    // Disposition-based on BOTH terminal phases; only the reward (Resolved only)
    // differs. On InvalidDeadend reward_pool == 0 (reward 0) AND the rejected-fact
    // approve-voter is STILL slashed: its slashed fraction funded `bond_pool`,
    // which the InvalidDeadend finalize site BURNED out of the vault, so returning
    // the full stake would short the vault.
    let amount = if vote.kind == VOTE_DUPLICATE {
        // Duplicate-voter: never slashed or rewarded, on any fact / either phase.
        vote.stake
    } else if fact.is_agreed() {
        // Approve-voter on an agreed fact: stake + pro-rata fact reward (Resolved
        // only; 0 on InvalidDeadend since reward_pool == 0).
        let r = if resolved {
            let (_, fact_bucket) = reward::reward_buckets(
                oracle.reward_pool,
                oracle.reward_proposer_weight,
                oracle.reward_fact_weight,
                oracle.total_correct_proposer_stake,
                oracle.total_approved_fact_stake,
            );
            reward::fact_reward(vote.stake, fact_bucket, oracle.total_approved_fact_stake)
        } else {
            0
        };
        vote.stake
            .checked_add(r)
            .ok_or(ProgramError::ArithmeticOverflow)?
    } else if fact.is_duplicate() {
        // Approve-voter on a duplicate-dominant fact: stake, no reward, no slash.
        vote.stake
    } else {
        // Approve-voter on a rejected fact: the slashed fraction already funded
        // bond_pool (burned on a dead-end); reclaim only the remainder.
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
        let mut data = fact_ai.try_borrow_mut()?;
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
        let data = account.try_borrow()?;
        bytemuck::pod_read_unaligned::<FactVote>(&data[..FactVote::LEN])
    };
    if vote.account_type != AccountType::FactVote.as_u8() {
        return Err(KassandraError::InvalidAccount.into());
    }
    Ok(vote)
}

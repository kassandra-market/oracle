//! `set_config`: DAO-gated retune of the `Protocol`-resident governable params.
//!
//! A single privileged instruction (Task F3) that overwrites the governable
//! config fields on the [`Protocol`] singleton — both the global MONETARY knobs
//! (`emission_*`, `total_supply_cap`, the fee-EMA params) and the BEHAVIORAL
//! knobs (`threshold_*`, `market_threshold_*`, `flip_slash_*`,
//! `phase_window`/`proposal_window`) plus the settlement-era RESERVED fields
//! (`fact_vote_slash_*`, reward weights). It is gated to the protocol's
//! `dao_authority` (the Squads v4 multisig vault PDA) via the shared
//! [`assert_dao_authority`] guard — a passed v0.6 futarchy proposal executes
//! through that vault.
//!
//! It overwrites the governable fields WHOLESALE (the payload carries every one)
//! — simplest and unambiguous. It deliberately does NOT touch the identity /
//! linkage / accounting fields: `dao_authority`, `kass_dao`, `admin`,
//! `governance_set`, `kass_mint`, `usdc_mint`, `fee_ema`, `last_creation_unix`,
//! `bump`, `account_type`. Only the config knobs move.
//!
//! # Snapshot semantics (F2/F3)
//! `set_config` mutates ONLY the `Protocol`. Existing oracles keep their frozen
//! `create_oracle`-time snapshot, so a mid-dispute governance change can never
//! move the goalposts. Oracles created AFTER this instruction snapshot the new
//! values.
//!
//! # Bounds checks (reject with [`KassandraError::InvalidConfig`])
//! These prevent a later divide-by-zero / nonsensical config on the
//! create_oracle / fact-quorum / slash / settlement paths:
//! * Denominators MUST be `> 0`: `threshold_den`, `market_threshold_den`,
//!   `flip_slash_den`, `fact_vote_slash_den`, `emission_den`,
//!   `challenge_fail_usdc_fee_den`, `challenge_success_kass_fee_den`.
//! * Challenge-fee fractions MUST be `<= 1` (`challenge_fail_usdc_fee`,
//!   `challenge_success_kass_fee`): a fee above 100% of the escrow/bond is
//!   nonsensical.
//! * Fraction numerators MUST be `<= ` their denominator (the value is an
//!   intended `<= 1` fraction): `threshold`, `flip_slash`, `fact_vote_slash`,
//!   `emission`, and `market_threshold`.
//!   - **`market_threshold` <= 1 is a DELIBERATE choice.** It is a RELATIVE
//!     slash-trigger margin (`fail_twap > pass_twap * (1 + num/den)`), so
//!     `num <= den` caps the margin at +100% (fail must beat pass by at most
//!     2x). A margin wider than 100% is economically absurd for an honest-vs-
//!     fraud decision market, so we reject it for consistency with the other
//!     fractions and to block fat-finger configs. If a future design ever needs
//!     a >100% margin, relax THIS one check (the math itself,
//!     `fail*den > pass*(den+num)`, does not require `num <= den`).
//! * Windows MUST be `> 0`: `phase_window`, `proposal_window`, and
//!   `fee_ema_halflife` (a divisor in the fee-EMA decay).
//! * At least one reward weight (`reward_proposer_weight` /
//!   `reward_fact_weight`) MUST be `> 0`, so the settlement-era reward split
//!   denominator (`pw + fw`) is never zero.
//!
//! No bound on `total_supply_cap`, `fee_per_ema_unit`, `fee_ema_increment`
//! (any value, incl. 0, is meaningful: 0 cap / 0 fee / 0 bump).
//!
//! # Accounts
//! 0. protocol PDA  — writable; the `[b"protocol"]` singleton
//! 1. dao_authority — signer; must equal `protocol.dao_authority`
//!
//! # Instruction payload (after the 1-byte discriminant), exactly 176 bytes
//! 22 little-endian 8-byte fields, in this fixed order:
//! `emission_num u64` ++ `emission_den u64` ++ `total_supply_cap u64` ++
//! `fee_ema_halflife i64` ++ `fee_per_ema_unit u64` ++ `fee_ema_increment u64`
//! ++ `threshold_num u64` ++ `threshold_den u64` ++ `market_threshold_num u64`
//! ++ `market_threshold_den u64` ++ `flip_slash_num u64` ++ `flip_slash_den u64`
//! ++ `phase_window i64` ++ `proposal_window i64` ++ `fact_vote_slash_num u64`
//! ++ `fact_vote_slash_den u64` ++ `reward_proposer_weight u64` ++
//! `reward_fact_weight u64` ++ `challenge_fail_usdc_fee_num u64` ++
//! `challenge_fail_usdc_fee_den u64` ++ `challenge_success_kass_fee_num u64` ++
//! `challenge_success_kass_fee_den u64` (the last 4 are the Task C1 challenge
//! fees: each `den > 0`, `num <= den`).

use pinocchio::{
    account_info::AccountInfo, program_error::ProgramError, pubkey::Pubkey, ProgramResult,
};

use crate::{
    error::KassandraError,
    processor::guards::{assert_dao_authority, load_protocol},
    state::Protocol,
};

/// Exact payload length: 22 × 8-byte fields.
const PAYLOAD_LEN: usize = 22 * 8;

/// Read the `i`-th 8-byte little-endian field as `u64`.
#[inline]
fn u64_at(payload: &[u8], i: usize) -> u64 {
    let off = i * 8;
    u64::from_le_bytes(payload[off..off + 8].try_into().unwrap())
}

/// Read the `i`-th 8-byte little-endian field as `i64`.
#[inline]
fn i64_at(payload: &[u8], i: usize) -> i64 {
    let off = i * 8;
    i64::from_le_bytes(payload[off..off + 8].try_into().unwrap())
}

pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], payload: &[u8]) -> ProgramResult {
    let [protocol_ai, dao_authority_ai, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // --- payload parse (exact length) --------------------------------------
    if payload.len() != PAYLOAD_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let emission_num = u64_at(payload, 0);
    let emission_den = u64_at(payload, 1);
    let total_supply_cap = u64_at(payload, 2);
    let fee_ema_halflife = i64_at(payload, 3);
    let fee_per_ema_unit = u64_at(payload, 4);
    let fee_ema_increment = u64_at(payload, 5);
    let threshold_num = u64_at(payload, 6);
    let threshold_den = u64_at(payload, 7);
    let market_threshold_num = u64_at(payload, 8);
    let market_threshold_den = u64_at(payload, 9);
    let flip_slash_num = u64_at(payload, 10);
    let flip_slash_den = u64_at(payload, 11);
    let phase_window = i64_at(payload, 12);
    let proposal_window = i64_at(payload, 13);
    let fact_vote_slash_num = u64_at(payload, 14);
    let fact_vote_slash_den = u64_at(payload, 15);
    let reward_proposer_weight = u64_at(payload, 16);
    let reward_fact_weight = u64_at(payload, 17);
    let challenge_fail_usdc_fee_num = u64_at(payload, 18);
    let challenge_fail_usdc_fee_den = u64_at(payload, 19);
    let challenge_success_kass_fee_num = u64_at(payload, 20);
    let challenge_success_kass_fee_den = u64_at(payload, 21);

    // --- gate: DAO authority signs (load_protocol pins the singleton) -------
    let mut protocol = load_protocol(protocol_ai, program_id)?;
    assert_dao_authority(&protocol, dao_authority_ai)?;

    // --- bounds checks ------------------------------------------------------
    // Denominators must be positive (else a later divide-by-zero).
    if threshold_den == 0
        || market_threshold_den == 0
        || flip_slash_den == 0
        || fact_vote_slash_den == 0
        || emission_den == 0
        || challenge_fail_usdc_fee_den == 0
        || challenge_success_kass_fee_den == 0
    {
        return Err(KassandraError::InvalidConfig.into());
    }
    // Fractions are intended `<= 1` (numerator must not exceed denominator).
    if threshold_num > threshold_den
        || flip_slash_num > flip_slash_den
        || fact_vote_slash_num > fact_vote_slash_den
        || emission_num > emission_den
        || market_threshold_num > market_threshold_den
        || challenge_fail_usdc_fee_num > challenge_fail_usdc_fee_den
        || challenge_success_kass_fee_num > challenge_success_kass_fee_den
    {
        return Err(KassandraError::InvalidConfig.into());
    }
    // Windows must be positive (phase/proposal windows + the fee-EMA half-life
    // divisor).
    if phase_window <= 0 || proposal_window <= 0 || fee_ema_halflife <= 0 {
        return Err(KassandraError::InvalidConfig.into());
    }
    // At least one reward weight must be positive (the settlement split
    // denominator `pw + fw` must never be zero).
    if reward_proposer_weight == 0 && reward_fact_weight == 0 {
        return Err(KassandraError::InvalidConfig.into());
    }

    // --- overwrite ONLY the governable fields -------------------------------
    // Identity / linkage / accounting fields are left untouched.
    protocol.emission_num = emission_num;
    protocol.emission_den = emission_den;
    protocol.total_supply_cap = total_supply_cap;
    protocol.fee_ema_halflife = fee_ema_halflife;
    protocol.fee_per_ema_unit = fee_per_ema_unit;
    protocol.fee_ema_increment = fee_ema_increment;
    protocol.threshold_num = threshold_num;
    protocol.threshold_den = threshold_den;
    protocol.market_threshold_num = market_threshold_num;
    protocol.market_threshold_den = market_threshold_den;
    protocol.flip_slash_num = flip_slash_num;
    protocol.flip_slash_den = flip_slash_den;
    protocol.phase_window = phase_window;
    protocol.proposal_window = proposal_window;
    protocol.fact_vote_slash_num = fact_vote_slash_num;
    protocol.fact_vote_slash_den = fact_vote_slash_den;
    protocol.reward_proposer_weight = reward_proposer_weight;
    protocol.reward_fact_weight = reward_fact_weight;
    protocol.challenge_fail_usdc_fee_num = challenge_fail_usdc_fee_num;
    protocol.challenge_fail_usdc_fee_den = challenge_fail_usdc_fee_den;
    protocol.challenge_success_kass_fee_num = challenge_success_kass_fee_num;
    protocol.challenge_success_kass_fee_den = challenge_success_kass_fee_den;
    {
        let mut data = protocol_ai.try_borrow_mut_data()?;
        data[..Protocol::LEN].copy_from_slice(bytemuck::bytes_of(&protocol));
    }

    Ok(())
}

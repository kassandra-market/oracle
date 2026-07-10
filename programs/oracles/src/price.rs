//! `kass_price` — governance-anchored KASS/USDC spot TWAP read (Task F5).
//!
//! The manipulation-resistant KASS/USDC price the protocol governs around is the
//! **futarchy program's embedded spot `TwapOracle`**, NOT a Meteora pool (cp-amm
//! has no cumulative price observation — see [`crate::cpi::metadao_v06`] finding
//! #2). That oracle lives INSIDE the futarchy `Dao` account itself: the spot
//! `Pool` is the first payload element of `Dao.amm.state` (both `PoolState`
//! variants), so its `TwapOracle` fields sit at FIXED, variant-independent byte
//! offsets (`aggregator`@9, `last_updated`@25, `created_at`@33,
//! `start_delay`@105). F0 validated those offsets + the `get_twap` math against a
//! hand-built `Dao` blob; this module REUSES that primitive
//! ([`metadao_v06::futarchy_spot_twap`]) rather than re-deriving the math.
//!
//! # Read source (confirmed)
//! The price is read **directly from the `Protocol.kass_dao` account's bytes** —
//! there is NO separate pool account to bind. `kass_dao` IS the futarchy `Dao`
//! account, recorded by `set_governance` (F1), and the embedded spot oracle is at
//! the fixed offsets above. So the "binding from `kass_dao` to the spot-oracle
//! account" is the identity: they are the same account.
//!
//! # Anchoring (why an attacker can't substitute a fake price)
//! 1. **Governance anchor** — the passed account's key MUST equal
//!    `Protocol.kass_dao` (the DAO blessed by the one-shot `set_governance`
//!    handoff). An attacker-supplied account with a doctored oracle is rejected.
//! 2. **Owner anchor (defense-in-depth)** — the account MUST be owned by the
//!    futarchy program ([`metadao_v06::FUTARCHY_ID`]); a program-owned-elsewhere
//!    look-alike at the same address is impossible, but the check is cheap and
//!    documents intent. (Before `set_governance`, `kass_dao` is the zero pubkey
//!    and no real account matches it, so the read denies — no "unset" branch
//!    needed.)
//!
//! # No-observation handling
//! Per F0's contract, [`metadao_v06::futarchy_spot_twap`] returns
//! [`KassandraError::InvalidAccount`] when the TWAP is not yet observable: a zero
//! `aggregator` (no observation), a non-positive elapsed window, or a too-short
//! buffer. `kass_price` propagates that unchanged.
//!
//! # No on-chain consumer yet
//! This ships as a validated primitive. The challenge-market rework (next
//! milestone) is its first consumer; the optional [`crate::instruction::Ix::KassPrice`]
//! instruction wrapper exposes it for that seam + off-chain queries today.

use pinocchio::{account::AccountView as AccountInfo, error::ProgramError};

use crate::{
    cpi::metadao_v06,
    processor::guards::{assert_key, assert_owned_by_program},
    state::Protocol,
};

/// Read the KASS/USDC spot TWAP from the governance-blessed futarchy `Dao`
/// account, scaled by `1e12` (quote units per base unit; see
/// [`metadao_v06::futarchy_spot_twap`]).
///
/// `kass_dao_ai` must be the account whose key equals `protocol.kass_dao` and
/// which is owned by the futarchy program; otherwise this returns
/// [`KassandraError::InvalidAccount`](crate::error::KassandraError::InvalidAccount).
/// A not-yet-observable TWAP (zero aggregator / non-positive window) returns the
/// same error, per F0's `futarchy_spot_twap` contract.
pub fn kass_price(protocol: &Protocol, kass_dao_ai: &AccountInfo) -> Result<u128, ProgramError> {
    // (1) Governance anchor: only the DAO recorded by `set_governance`.
    assert_key(kass_dao_ai, &protocol.kass_dao)?;
    // (2) Owner anchor (defense-in-depth): it must be a futarchy `Dao` account.
    assert_owned_by_program(kass_dao_ai, &metadao_v06::FUTARCHY_ID)?;
    // (3) Read the embedded spot oracle at the F0-validated fixed offsets.
    let data = kass_dao_ai.try_borrow()?;
    metadao_v06::futarchy_spot_twap(&data)
}

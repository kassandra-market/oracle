//! Rent-exempt balance helper.
//!
//! # Why this exists (pinocchio 0.11 workaround)
//! pinocchio 0.8's `Rent` sysvar carried all three fields
//! (`lamports_per_byte_year`, `exemption_threshold`, `burn_percent`) and its
//! `minimum_balance` multiplied by the `exemption_threshold`. pinocchio 0.11
//! shrank `Rent` to a single `lamports_per_byte` field that it reads from the
//! first 8 bytes of the sysvar — i.e. `lamports_per_byte_year` — and then
//! computes `(ACCOUNT_STORAGE_OVERHEAD + len) * lamports_per_byte` WITHOUT the
//! exemption threshold. That yields exactly HALF the real rent-exempt minimum,
//! so a `CreateAccount` funded with it fails `InsufficientFundsForRent`.
//!
//! Solana's `exemption_threshold` is a fixed genesis constant of `2.0` on every
//! cluster (and LiteSVM) and has never changed — pinocchio's own
//! `DEFAULT_LAMPORTS_PER_BYTE == 6960 == 3480 * 2` bakes in the same assumption.
//! We restore it by doubling pinocchio's (halved) result.

use pinocchio::{
    error::ProgramError,
    sysvars::{rent::Rent, Sysvar},
};

/// Solana's fixed rent `exemption_threshold`, which pinocchio 0.11's `Rent`
/// drops (see the module docs).
const RENT_EXEMPTION_THRESHOLD: u64 = 2;

/// Minimum lamports for a `len`-byte account to be rent-exempt.
pub fn minimum_rent(len: usize) -> Result<u64, ProgramError> {
    Rent::get()?
        .try_minimum_balance(len)?
        .checked_mul(RENT_EXEMPTION_THRESHOLD)
        .ok_or(ProgramError::ArithmeticOverflow)
}

//! Minimal, self-contained view of the sibling Kassandra oracle account.
//!
//! The market program reads a Kassandra `Oracle` account only as a resolution
//! gate: it needs the account-type tag, `options_count`, the dispute `phase`,
//! and the final `resolved_option`. Rather than depend on the whole
//! `kassandra-program` crate (a cross-repo `path` dependency), we hardcode the
//! handful of constants and field OFFSETS the gate needs and read those four
//! bytes directly. This keeps this repo self-contained and its Docker build
//! single-context.
//!
//! Every value here is COPIED VERBATIM from the Kassandra program and MUST stay
//! byte-identical to it (a wrong offset/id silently breaks the oracle gate):
//!   - program id  ← `kassandra/programs/kassandra/src/lib.rs`
//!   - tag / phase / offsets ← `kassandra/programs/kassandra/src/state.rs`
//!     (`AccountType::Oracle == 1`, `Phase::Resolved == 7`,
//!     `Phase::InvalidDeadend == 8`, `Oracle` is `#[repr(C)]` packed with
//!     `size_of == 392`; `options_count`/`phase`/`resolved_option` sit at the
//!     offsets below). The market test harness stamps the same offsets, and the
//!     oracle-gated create/cancel/activate/resolve tests prove the reads match.

use pinocchio::address::Address;

/// The Kassandra program that OWNS every oracle account (owner check).
/// From `kassandra/programs/kassandra/src/lib.rs`.
pub const KASSANDRA_PROGRAM_ID: Address =
    Address::from_str_const("KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY");

/// `AccountType::Oracle` discriminant, stored as byte 0 (type-confusion guard).
pub const ORACLE_ACCOUNT_TYPE: u8 = 1;

/// `size_of::<kassandra::state::Oracle>()`; minimum data length of a real oracle.
pub const ORACLE_LEN: usize = 392;

/// Byte offset of `options_count: u8` within the `Oracle` struct.
pub const OPTIONS_COUNT_OFFSET: usize = 160;
/// Byte offset of `phase: u8` within the `Oracle` struct.
pub const PHASE_OFFSET: usize = 161;
/// Byte offset of `resolved_option: u8` within the `Oracle` struct.
pub const RESOLVED_OPTION_OFFSET: usize = 197;

/// `Phase::Resolved` discriminant.
pub const PHASE_RESOLVED: u8 = 7;
/// `Phase::InvalidDeadend` discriminant.
pub const PHASE_INVALID_DEADEND: u8 = 8;

/// The four Kassandra-oracle fields the market resolution gate reads.
#[derive(Clone, Copy, Debug)]
pub struct KassOracle {
    pub account_type: u8,
    pub options_count: u8,
    pub phase: u8,
    pub resolved_option: u8,
}

impl KassOracle {
    /// Decode the four gated fields by offset from an oracle account's data.
    ///
    /// `data` MUST be at least [`ORACLE_LEN`] bytes (the caller len-checks); the
    /// offsets are all `< ORACLE_LEN`.
    pub fn read(data: &[u8]) -> Self {
        Self {
            account_type: data[0],
            options_count: data[OPTIONS_COUNT_OFFSET],
            phase: data[PHASE_OFFSET],
            resolved_option: data[RESOLVED_OPTION_OFFSET],
        }
    }
}

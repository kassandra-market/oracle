//! # kassandra-oracles-sdk
//!
//! A hand-written Rust client SDK for the Kassandra dispute-core program. It
//! provides three things, and nothing that duplicates the on-chain contract:
//!
//! * [`pda`] — every program PDA derivation (the seed conventions are the
//!   program's public contract).
//! * [`ix`] — instruction builders returning [`solana_instruction::Instruction`],
//!   one per [`kassandra_oracles_program::instruction::Ix`] variant.
//! * [`accounts`] — thin, zero-copy decoders over the shared on-chain account
//!   structs (re-exported from [`kassandra_oracles_program::state`]).
//!
//! The canonical source of the wire format (discriminants, seeds, field layouts)
//! is [`kassandra_oracles_program`] itself, depended on with the `no-entrypoint` feature
//! so it links as a plain host library. Tests, the runner, and any other Rust
//! client build instructions through this crate instead of hand-rolling account
//! metas and payload bytes.

pub mod accounts;
pub mod ix;
pub mod pda;

mod config_params;
pub use config_params::ConfigParams;

pub use solana_instruction::{AccountMeta, Instruction};
pub use solana_pubkey::Pubkey;

// Re-export the discriminant enum so callers can match on it without a second
// dependency edge to the program crate.
pub use kassandra_oracles_program::instruction::Ix;

/// The Kassandra dispute-core program ID (`programs/oracles/src/lib.rs`).
pub const PROGRAM_ID: Pubkey = Pubkey::new_from_array(kassandra_oracles_program::ID.to_bytes());

/// SPL Token program ID.
pub const TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

/// System program ID (all-zero pubkey).
pub const SYSTEM_PROGRAM_ID: Pubkey = Pubkey::new_from_array([0u8; 32]);

/// SPL Associated Token Account program ID — the DAO treasury is the KASS ATA of
/// the `dao_authority` under this program.
pub const ATA_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

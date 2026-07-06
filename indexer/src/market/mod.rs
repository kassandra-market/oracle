//! Market side of the unified indexer.
//!
//! The oracle side crawls transactions into an event log (`crate::db` → `events`);
//! this side indexes the kassandra-market program's *accounts* (Config / Market /
//! Contribution) via a Carbon pipeline (gpa snapshot + optional program-subscribe
//! live tail) into the `market_accounts` Postgres table, and serves a read + tx
//! gateway under `/api/*`.
//!
//! Ported from the standalone `kassandra-market-indexer`: the in-memory `Store`
//! became `db` (Postgres), and the program id + AMM discriminator are sourced
//! locally (`kassandra_market_program::ID` + the const below) so this crate never
//! pulls the solana-sdk-v2 `kassandra-market-sdk`.

pub mod api;
pub mod db;
pub mod decoder;
pub mod json;
pub mod processor;
pub mod rpc;

use solana_pubkey::Pubkey;

/// The kassandra-market program id as a v3 `Pubkey`. Sourced from the program
/// crate's `ID` (a `solana-address` v2 `Address`) via its raw bytes, so the
/// shipped indexer stays on the granular v3 client stack.
pub fn default_program_id() -> Pubkey {
    let bytes: [u8; 32] = kassandra_market_program::ID
        .as_ref()
        .try_into()
        .expect("program id is 32 bytes");
    Pubkey::new_from_array(bytes)
}

/// MetaDAO `Amm` account discriminator (first 8 bytes) — used to sanity-check an
/// AMM account before reading its reserves. Inlined here (was
/// `kassandra_market_sdk::metadao::AMM_ACCOUNT_DISCRIMINATOR`) to avoid the
/// solana-sdk-v2 SDK dependency.
pub const AMM_ACCOUNT_DISCRIMINATOR: [u8; 8] = [0x8f, 0xf5, 0xc8, 0x11, 0x4a, 0xd6, 0xc4, 0x87];

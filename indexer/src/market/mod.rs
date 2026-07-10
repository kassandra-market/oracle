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
//! pulls the solana-sdk-v2 `kassandra-markets-sdk`.

pub mod api;
pub mod db;
pub mod decoder;
pub mod json;
pub mod price_subscribe;
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
/// `kassandra_markets_sdk::metadao::AMM_ACCOUNT_DISCRIMINATOR`) to avoid the
/// solana-sdk-v2 SDK dependency.
pub const AMM_ACCOUNT_DISCRIMINATOR: [u8; 8] = [0x8f, 0xf5, 0xc8, 0x11, 0x4a, 0xd6, 0xc4, 0x87];

/// MetaDAO `Amm` reserve offsets (base/quote `u64` LE), after the 8-byte Anchor
/// account discriminator. base = cYES, quote = cNO.
const AMM_BASE_AMOUNT_OFFSET: usize = 115;
const AMM_QUOTE_AMOUNT_OFFSET: usize = 123;

/// Decode a MetaDAO `Amm` account's `(base, quote)` reserves — `(cYES, cNO)` raw
/// base units — after verifying the account discriminator. Returns `None` for a
/// non-AMM / truncated account. Shared by the read API (live reserves) and the
/// reconcile-loop price sampler.
pub fn decode_amm_reserves(data: &[u8]) -> Option<(u64, u64)> {
    if data.len() < AMM_QUOTE_AMOUNT_OFFSET + 8 {
        return None;
    }
    if data.get(..8) != Some(&AMM_ACCOUNT_DISCRIMINATOR[..]) {
        return None;
    }
    let read = |off: usize| -> Option<u64> {
        let bytes: [u8; 8] = data.get(off..off + 8)?.try_into().ok()?;
        Some(u64::from_le_bytes(bytes))
    };
    Some((
        read(AMM_BASE_AMOUNT_OFFSET)?,
        read(AMM_QUOTE_AMOUNT_OFFSET)?,
    ))
}

/// Implied YES probability `P(YES) = quote / (base + quote)` from `(base, quote)`
/// reserves, or `None` when the pool is empty (probability undefined). Mirrors the
/// app's `impliedYesProbability`.
pub fn implied_yes_probability(base: u64, quote: u64) -> Option<f64> {
    let total = (base as u128) + (quote as u128);
    if total == 0 {
        return None;
    }
    Some(quote as f64 / total as f64)
}

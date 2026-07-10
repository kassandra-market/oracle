//! serde DTOs mapping the on-chain Pod structs to JSON.
//!
//! Conventions for the browser client: pubkeys are base58 strings, and **all
//! `u64` are emitted as strings** to avoid JS `Number` precision loss.

use kassandra_markets_program::state::{Config, Contribution, Market};
use serde::Serialize;

type PubkeyBytes = [u8; 32];

/// Base58-encode any 32-byte key. Accepts both the raw `[u8; 32]` account
/// address and the program's `Address` state fields (which are
/// `repr(transparent) [u8; 32]` and `AsRef<[u8]>`), so the emitted JSON stays
/// byte-identical (base58 of the same 32 bytes).
fn b58(bytes: impl AsRef<[u8]>) -> String {
    bs58::encode(bytes.as_ref()).into_string()
}

fn status_label(status: u8) -> &'static str {
    match status {
        0 => "funding",
        1 => "active",
        2 => "resolved",
        3 => "void",
        4 => "cancelled",
        _ => "unknown",
    }
}

/// One OHLC candle of implied YES probability (`0..1`) over a time bucket. `time`
/// is the bucket-start unix seconds (the format lightweight-charts expects).
#[derive(Serialize)]
pub struct CandleDto {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

impl CandleDto {
    pub fn new(c: &crate::market::db::Candle) -> Self {
        Self {
            time: c.time,
            open: c.open,
            high: c.high,
            low: c.low,
            close: c.close,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigDto {
    pub address: String,
    pub authority: String,
    pub kass_mint: String,
    pub min_liquidity: String,
    pub bump: u8,
    pub fee_bps: u16,
    pub fee_destination: String,
    pub slot: String,
}

impl ConfigDto {
    pub fn new(address: &PubkeyBytes, c: &Config, slot: u64) -> Self {
        Self {
            address: b58(address),
            authority: b58(c.authority),
            kass_mint: b58(c.kass_mint),
            min_liquidity: c.min_liquidity.to_string(),
            bump: c.bump,
            fee_bps: c.fee_bps,
            fee_destination: b58(c.fee_destination),
            slot: slot.to_string(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketDto {
    pub address: String,
    pub status: u8,
    pub status_label: String,
    pub oracle: String,
    pub creator: String,
    pub kass_mint: String,
    pub escrow_vault: String,
    pub min_liquidity: String,
    pub total_contributed: String,
    pub open_contributions: u16,
    pub bump: u8,
    pub escrow_bump: u8,
    pub outcome_index: u8,
    pub fee_bps: u16,
    pub fee_collected: u8,
    pub settled: u8,
    // Phase-2a MetaDAO bindings (zeroed until `activate`).
    pub question: String,
    pub vault: String,
    pub yes_mint: String,
    pub no_mint: String,
    pub amm: String,
    pub lp_mint: String,
    pub lp_vault: String,
    pub lp_total: String,
    pub slot: String,
}

impl MarketDto {
    pub fn new(address: &PubkeyBytes, m: &Market, slot: u64) -> Self {
        Self {
            address: b58(address),
            status: m.status,
            status_label: status_label(m.status).to_string(),
            oracle: b58(m.oracle),
            creator: b58(m.creator),
            kass_mint: b58(m.kass_mint),
            escrow_vault: b58(m.escrow_vault),
            min_liquidity: m.min_liquidity.to_string(),
            total_contributed: m.total_contributed.to_string(),
            open_contributions: m.open_contributions,
            bump: m.bump,
            escrow_bump: m.escrow_bump,
            outcome_index: m.outcome_index,
            fee_bps: m.fee_bps,
            fee_collected: m.fee_collected,
            settled: m.settled,
            question: b58(m.question),
            vault: b58(m.vault),
            yes_mint: b58(m.yes_mint),
            no_mint: b58(m.no_mint),
            amm: b58(m.amm),
            lp_mint: b58(m.lp_mint),
            lp_vault: b58(m.lp_vault),
            lp_total: m.lp_total.to_string(),
            slot: slot.to_string(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContributionDto {
    pub market: String,
    pub contributor: String,
    pub amount: String,
    pub claimed: bool,
    pub bump: u8,
}

impl ContributionDto {
    pub fn new(c: &Contribution) -> Self {
        Self {
            market: b58(c.market),
            contributor: b58(c.contributor),
            amount: c.amount.to_string(),
            claimed: c.claimed != 0,
            bump: c.bump,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OracleDto {
    pub options_count: u8,
    pub phase: u8,
    pub resolved_option: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReservesDto {
    pub base: String,
    pub quote: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketDetailDto {
    pub market: MarketDto,
    pub contributions: Vec<ContributionDto>,
    pub oracle: Option<OracleDto>,
    pub reserves: Option<ReservesDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDto {
    /// base64 of the raw account data.
    pub data: String,
    pub owner: String,
    pub lamports: String,
}

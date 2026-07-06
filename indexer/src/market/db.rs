//! Postgres persistence for the indexed kassandra-market accounts.
//!
//! Replaces the standalone indexer's in-memory `Store`. One table
//! (`market_accounts`) holds the raw Pod bytes of every Config / Market /
//! Contribution account, keyed by pubkey and slot-gated so an out-of-order
//! (older) datasource event can never clobber a newer one. Reads decode the
//! bytes back into `kassandra_market_program::state` structs on demand.

use std::collections::HashSet;
use std::str::FromStr;

use anyhow::{Context, Result};
use kassandra_market_program::state::{Config, Contribution, Market};
use solana_pubkey::Pubkey;
use tokio_postgres::Client;

/// `account_type` tag values (mirror `state::AccountType`).
pub const TYPE_CONFIG: i16 = 1;
pub const TYPE_MARKET: i16 = 2;
pub const TYPE_CONTRIBUTION: i16 = 3;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS market_accounts (
  pubkey       TEXT     PRIMARY KEY,
  account_type SMALLINT NOT NULL,   -- 1=Config 2=Market 3=Contribution
  market_ref   TEXT,                -- Contribution.market (base58) for indexed lookup
  slot         BIGINT   NOT NULL,
  data         BYTEA    NOT NULL    -- raw Pod bytes; decoded on read
);
CREATE INDEX IF NOT EXISTS market_accounts_type_idx ON market_accounts (account_type);
CREATE INDEX IF NOT EXISTS market_accounts_market_ref_idx ON market_accounts (market_ref);
"#;

/// Create the market schema (idempotent; runs alongside the oracle schema on the
/// same Postgres connection).
pub async fn create_schema(client: &Client) -> Result<()> {
    client.batch_execute(SCHEMA).await?;
    Ok(())
}

/// Slot-gated upsert of one decoded account's raw bytes. The `WHERE` gate means an
/// update only applies if its slot is `>=` the stored slot (last-writer-wins on an
/// equal slot), mirroring the old in-memory store's gate.
pub async fn upsert_account(
    client: &Client,
    pubkey: &str,
    account_type: i16,
    market_ref: Option<&str>,
    slot: i64,
    data: &[u8],
) -> Result<()> {
    client
        .execute(
            "INSERT INTO market_accounts (pubkey, account_type, market_ref, slot, data)
             VALUES ($1,$2,$3,$4,$5)
             ON CONFLICT (pubkey) DO UPDATE
               SET account_type = EXCLUDED.account_type,
                   market_ref   = EXCLUDED.market_ref,
                   slot         = EXCLUDED.slot,
                   data         = EXCLUDED.data
               WHERE market_accounts.slot <= EXCLUDED.slot",
            &[&pubkey, &account_type, &market_ref, &slot, &data],
        )
        .await?;
    Ok(())
}

/// Prune accounts CLOSED on-chain — i.e. absent from an authoritative
/// getProgramAccounts snapshot taken at `snapshot_slot`, whose stored slot is
/// `<= snapshot_slot` (a row newer than the snapshot is kept: the snapshot simply
/// predates it, it isn't closed). Pass the base58 pubkeys present in the snapshot.
/// Only call after a snapshot that fetched successfully.
pub async fn prune(client: &Client, snapshot_slot: i64, present: &HashSet<String>) -> Result<u64> {
    let present_vec: Vec<String> = present.iter().cloned().collect();
    let n = client
        .execute(
            "DELETE FROM market_accounts
             WHERE slot <= $1 AND NOT (pubkey = ANY($2))",
            &[&snapshot_slot, &present_vec],
        )
        .await
        .context("prune market_accounts")?;
    Ok(n)
}

fn decode<T: bytemuck::AnyBitPattern>(data: &[u8], len: usize) -> Option<T> {
    (data.len() >= len).then(|| bytemuck::pod_read_unaligned::<T>(&data[..len]))
}

/// The governed singleton `Config` (pubkey, value, slot), if indexed.
pub async fn get_config(client: &Client) -> Result<Option<(Pubkey, Config, u64)>> {
    let rows = client
        .query(
            "SELECT pubkey, slot, data FROM market_accounts
             WHERE account_type = $1 ORDER BY slot DESC LIMIT 1",
            &[&TYPE_CONFIG],
        )
        .await?;
    Ok(rows.first().and_then(|r| {
        let pk: String = r.get(0);
        let slot: i64 = r.get(1);
        let data: Vec<u8> = r.get(2);
        let pubkey = Pubkey::from_str(&pk).ok()?;
        let cfg = decode::<Config>(&data, Config::LEN)?;
        Some((pubkey, cfg, slot as u64))
    }))
}

/// All indexed markets (pubkey, value, slot).
pub async fn get_markets(client: &Client) -> Result<Vec<(Pubkey, Market, u64)>> {
    let rows = client
        .query(
            "SELECT pubkey, slot, data FROM market_accounts WHERE account_type = $1",
            &[&TYPE_MARKET],
        )
        .await?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            let pk: String = r.get(0);
            let slot: i64 = r.get(1);
            let data: Vec<u8> = r.get(2);
            let pubkey = Pubkey::from_str(&pk).ok()?;
            let m = decode::<Market>(&data, Market::LEN)?;
            Some((pubkey, m, slot as u64))
        })
        .collect())
}

/// One market by pubkey (value, slot).
pub async fn get_market(client: &Client, pubkey: &str) -> Result<Option<(Market, u64)>> {
    let rows = client
        .query(
            "SELECT slot, data FROM market_accounts WHERE pubkey = $1 AND account_type = $2",
            &[&pubkey, &TYPE_MARKET],
        )
        .await?;
    Ok(rows.first().and_then(|r| {
        let slot: i64 = r.get(0);
        let data: Vec<u8> = r.get(1);
        let m = decode::<Market>(&data, Market::LEN)?;
        Some((m, slot as u64))
    }))
}

/// All contributions whose `market` field points at `market` (base58).
pub async fn contributions_for(client: &Client, market: &str) -> Result<Vec<Contribution>> {
    let rows = client
        .query(
            "SELECT data FROM market_accounts WHERE account_type = $1 AND market_ref = $2",
            &[&TYPE_CONTRIBUTION, &market],
        )
        .await?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            let data: Vec<u8> = r.get(0);
            decode::<Contribution>(&data, Contribution::LEN)
        })
        .collect())
}

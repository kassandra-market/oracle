//! Postgres persistence for the indexed kassandra-market accounts.
//!
//! Replaces the standalone indexer's in-memory `Store`. One table
//! (`market_accounts`) holds the raw Pod bytes of every Config / Market /
//! Contribution account, keyed by pubkey and slot-gated so an out-of-order
//! (older) datasource event can never clobber a newer one. Reads decode the
//! bytes back into `kassandra_markets_program::state` structs on demand.

use std::collections::HashSet;
use std::str::FromStr;

use anyhow::{Context, Result};
use kassandra_markets_program::state::{Config, Contribution, Market};
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

-- Price time-series for each Active market's cYES/cNO pool, recorded by the
-- websocket price subscriber (`price_subscribe`): one row per pool account update
-- (i.e. per swap), keyed (market, slot). The (market, slot) key dedups a repeated
-- reading of an unchanged pool. `price` is the implied YES probability
-- P(YES) = quote / (base + quote), 0..1. Candles are aggregated from this on read
-- (see `get_candles`).
CREATE TABLE IF NOT EXISTS market_price (
  market TEXT   NOT NULL,          -- Market pubkey (base58)
  slot   BIGINT NOT NULL,          -- slot the sample was read at
  ts     BIGINT NOT NULL,          -- unix seconds at capture (server clock)
  base   BIGINT NOT NULL,          -- cYES reserve (raw base units)
  quote  BIGINT NOT NULL,          -- cNO reserve (raw base units)
  price  DOUBLE PRECISION NOT NULL,-- implied P(YES) = quote / (base + quote)
  PRIMARY KEY (market, slot)
);
CREATE INDEX IF NOT EXISTS market_price_market_ts_idx ON market_price (market, ts);
"#;

/// One OHLC candle aggregated from `market_price` samples in a time bucket.
pub struct Candle {
    /// Bucket start, unix seconds (aligned to the bucket width).
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

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

/// The `(market_pubkey_base58, amm)` of every ACTIVE market with a composed pool.
/// The price subscriber uses this to (un)subscribe to each live cYES/cNO pool:
/// status 1 = Active, and a zeroed `amm` means the pool isn't composed yet.
pub async fn active_market_amms(client: &Client) -> Result<Vec<(String, Pubkey)>> {
    Ok(get_markets(client)
        .await?
        .into_iter()
        .filter_map(|(pk, m, _slot)| {
            let amm = Pubkey::new_from_array(m.amm.to_bytes());
            (m.status == 1 && amm != Pubkey::default()).then(|| (pk.to_string(), amm))
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

/// Append one price sample for `market`. Idempotent per (market, slot): a repeated
/// sample of an unchanged pool (same slot) is dropped, so a flat pool doesn't
/// inflate the series with duplicate points.
pub async fn insert_price(
    client: &Client,
    market: &str,
    slot: i64,
    ts: i64,
    base: i64,
    quote: i64,
    price: f64,
) -> Result<()> {
    client
        .execute(
            "INSERT INTO market_price (market, slot, ts, base, quote, price)
             VALUES ($1,$2,$3,$4,$5,$6)
             ON CONFLICT (market, slot) DO NOTHING",
            &[&market, &slot, &ts, &base, &quote, &price],
        )
        .await?;
    Ok(())
}

/// OHLC candles for `market`, bucketed by `bucket_secs`, most-recent `limit`
/// buckets returned in ascending time order. `open`/`close` are the first/last
/// sample (by slot) in each bucket; `high`/`low` the extremes.
pub async fn get_candles(
    client: &Client,
    market: &str,
    bucket_secs: i64,
    limit: i64,
) -> Result<Vec<Candle>> {
    let rows = client
        .query(
            "SELECT bucket_ts, open, high, low, close FROM (
               SELECT (ts / $2) * $2 AS bucket_ts,
                      (array_agg(price ORDER BY slot ASC))[1]  AS open,
                      MAX(price)                               AS high,
                      MIN(price)                               AS low,
                      (array_agg(price ORDER BY slot DESC))[1] AS close
               FROM market_price
               WHERE market = $1
               GROUP BY bucket_ts
               ORDER BY bucket_ts DESC
               LIMIT $3
             ) b
             ORDER BY bucket_ts ASC",
            &[&market, &bucket_secs, &limit],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| Candle {
            time: r.get(0),
            open: r.get(1),
            high: r.get(2),
            low: r.get(3),
            close: r.get(4),
        })
        .collect())
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

#[cfg(test)]
mod db_it {
    //! Postgres integration test for the price series → candle aggregation. The
    //! OHLC SQL (integer-bucketed `GROUP BY`, `array_agg … ORDER BY`,
    //! `ON CONFLICT DO NOTHING`) is Postgres-specific, so it runs on the real
    //! engine. Self-skips (never fails) when `TEST_DATABASE_URL` is unset — the
    //! dedicated CI `db-it` job provides a Postgres service and sets it.

    use super::*;
    use std::sync::Arc;

    async fn test_client() -> Option<Arc<Client>> {
        let url = std::env::var("TEST_DATABASE_URL").ok()?;
        let client = match crate::db::connect(&url).await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[db_it] connect to TEST_DATABASE_URL failed ({e}); skipping");
                return None;
            }
        };
        create_schema(&client).await.ok()?;
        Some(client)
    }

    #[tokio::test]
    async fn candles_aggregate_ohlc_from_price_samples() {
        let Some(client) = test_client().await else {
            eprintln!("[db_it] TEST_DATABASE_URL unset — skipping Postgres integration test");
            return;
        };
        client
            .batch_execute("TRUNCATE market_price")
            .await
            .expect("truncate");

        // Bucket 0 (width 60s): three samples at ts 0/30/30, slots 1/2/3.
        insert_price(&client, "MktA", 1, 0, 100, 100, 0.50)
            .await
            .unwrap();
        insert_price(&client, "MktA", 2, 30, 100, 300, 0.75)
            .await
            .unwrap();
        insert_price(&client, "MktA", 3, 30, 100, 9900, 0.99)
            .await
            .unwrap();
        // Same (market, slot) as the 0.75 point → ON CONFLICT DO NOTHING (ignored).
        insert_price(&client, "MktA", 2, 30, 1, 1, 0.01)
            .await
            .unwrap();
        // Bucket 60: a single sample at ts 90, slot 4.
        insert_price(&client, "MktA", 4, 90, 100, 100, 0.50)
            .await
            .unwrap();
        // A different market must not bleed into MktA's candles.
        insert_price(&client, "MktB", 9, 0, 1, 1, 0.10)
            .await
            .unwrap();

        let candles = get_candles(&client, "MktA", 60, 100).await.unwrap();
        assert_eq!(candles.len(), 2, "two 60s buckets");

        // Bucket 0: open=first-by-slot(0.50), close=last-by-slot(0.99), hi/lo extremes.
        let b0 = &candles[0];
        assert_eq!(b0.time, 0);
        assert!((b0.open - 0.50).abs() < 1e-9, "open {}", b0.open);
        assert!((b0.close - 0.99).abs() < 1e-9, "close {}", b0.close);
        assert!((b0.high - 0.99).abs() < 1e-9, "high {}", b0.high);
        assert!((b0.low - 0.50).abs() < 1e-9, "low {}", b0.low);

        // Bucket 60: a lone sample → O=H=L=C.
        let b1 = &candles[1];
        assert_eq!(b1.time, 60);
        assert!((b1.close - 0.50).abs() < 1e-9);

        // `limit` keeps the most-recent buckets, still returned ascending.
        let recent = get_candles(&client, "MktA", 60, 1).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].time, 60);
    }
}

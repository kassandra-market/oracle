//! Postgres persistence: the event log + the durable resume cursor.

use std::sync::Arc;

use anyhow::Result;
use tokio_postgres::{Client, NoTls};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS events (
  signature    TEXT     NOT NULL,
  ix_index     INT      NOT NULL,
  ix_type      TEXT     NOT NULL,
  discriminant SMALLINT NOT NULL,
  slot         BIGINT   NOT NULL,
  block_time   BIGINT,
  account0     TEXT,
  accounts     JSONB    NOT NULL,
  data_base64  TEXT     NOT NULL,
  PRIMARY KEY (signature, ix_index)
);
CREATE INDEX IF NOT EXISTS events_account0_idx ON events (account0);
CREATE INDEX IF NOT EXISTS events_ix_type_idx  ON events (ix_type);
CREATE INDEX IF NOT EXISTS events_slot_idx      ON events (slot DESC);

-- Durable resume cursor: the crawler is (re)started with `until = signature`, so
-- it re-fetches everything newer than this point. Only promoted forward once the
-- indexer has verifiably caught up to chain head (see the promotion task).
CREATE TABLE IF NOT EXISTS indexer_cursor (
  id        SMALLINT PRIMARY KEY DEFAULT 1,
  signature TEXT,
  slot      BIGINT,
  CONSTRAINT cursor_singleton CHECK (id = 1)
);

-- Oracle metadata INDEXED from the on-chain `oracle_meta` account (via the
-- `write_oracle_meta` instruction): the plaintext subject + option labels are
-- on-chain (authoritative), plus a `uri`/`uri_hash` referencing the extended
-- off-chain JSON. This table is a queryable mirror of chain — clients can also
-- read the account directly.
CREATE TABLE IF NOT EXISTS oracle_metadata (
  oracle    TEXT   PRIMARY KEY,
  subject   TEXT   NOT NULL,
  options   JSONB  NOT NULL,      -- array of option-label strings
  uri       TEXT   NOT NULL,      -- extended-metadata JSON URL (may be empty)
  uri_hash  TEXT   NOT NULL,      -- hex sha256 binding the off-chain JSON
  slot      BIGINT NOT NULL,
  signature TEXT   NOT NULL
);

-- The extended off-chain metadata JSON, hosted for app-created oracles (the app
-- POSTs it at creation; the public app server proxies GET/POST here). Served only
-- when its sha256 matches the on-chain `uri_hash` in `oracle_metadata`.
CREATE TABLE IF NOT EXISTS oracle_meta_json (
  oracle TEXT PRIMARY KEY,
  json   TEXT NOT NULL,
  sha256 TEXT NOT NULL            -- hex sha256 of `json`
);
"#;

/// One indexed Kassandra instruction.
pub struct Event {
    pub signature: String,
    pub ix_index: i32,
    pub ix_type: String,
    pub discriminant: i16,
    pub slot: i64,
    pub block_time: Option<i64>,
    pub account0: Option<String>,
    /// The instruction's account list, as a JSONB value (jsonb `?` account lookups).
    pub accounts: serde_json::Value,
    pub data_base64: String,
}

/// Connect, spawn the connection driver, and create the schema.
pub async fn connect(database_url: &str) -> Result<Arc<Client>> {
    let (client, connection) = tokio_postgres::connect(database_url, NoTls).await?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            log::error!("postgres connection error: {e}");
        }
    });
    client.batch_execute(SCHEMA).await?;
    Ok(Arc::new(client))
}

/// Insert one event, ignoring duplicates (idempotent re-processing).
pub async fn insert_event(client: &Client, e: &Event) -> Result<()> {
    client
        .execute(
            "INSERT INTO events
               (signature, ix_index, ix_type, discriminant, slot, block_time, account0, accounts, data_base64)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
             ON CONFLICT (signature, ix_index) DO NOTHING",
            &[
                &e.signature,
                &e.ix_index,
                &e.ix_type,
                &e.discriminant,
                &e.slot,
                &e.block_time,
                &e.account0,
                &e.accounts,
                &e.data_base64,
            ],
        )
        .await?;
    Ok(())
}

/// The durable resume cursor (signature to pass as the crawler's `until`).
pub async fn get_cursor(client: &Client) -> Result<Option<(String, i64)>> {
    let rows = client
        .query(
            "SELECT signature, slot FROM indexer_cursor WHERE id = 1 AND signature IS NOT NULL",
            &[],
        )
        .await?;
    Ok(rows
        .first()
        .map(|r| (r.get::<_, String>(0), r.get::<_, i64>(1))))
}

/// Promote the durable resume cursor forward.
pub async fn set_cursor(client: &Client, signature: &str, slot: i64) -> Result<()> {
    client
        .execute(
            "INSERT INTO indexer_cursor (id, signature, slot) VALUES (1, $1, $2)
             ON CONFLICT (id) DO UPDATE SET signature = EXCLUDED.signature, slot = EXCLUDED.slot",
            &[&signature, &slot],
        )
        .await?;
    Ok(())
}

/// `(event_count, cursor)` for the status endpoint.
pub async fn stats(client: &Client) -> Result<(i64, Option<(String, i64)>)> {
    let count = client
        .query_one("SELECT COUNT(*)::bigint FROM events", &[])
        .await?
        .get::<_, i64>(0);
    Ok((count, get_cursor(client).await?))
}

/// Query events with optional filters, newest first.
pub async fn query_events(
    client: &Client,
    ix_type: Option<&str>,
    account: Option<&str>,
    before_slot: Option<i64>,
    limit: i64,
) -> Result<Vec<serde_json::Value>> {
    let mut where_clauses: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = Vec::new();
    if let Some(t) = ix_type {
        params.push(Box::new(t.to_string()));
        where_clauses.push(format!("ix_type = ${}", params.len()));
    }
    if let Some(a) = account {
        params.push(Box::new(a.to_string()));
        where_clauses.push(format!(
            "(account0 = ${0} OR accounts ? ${0})",
            params.len()
        ));
    }
    if let Some(s) = before_slot {
        params.push(Box::new(s));
        where_clauses.push(format!("slot < ${}", params.len()));
    }
    params.push(Box::new(limit.min(1000)));
    let sql = format!(
        "SELECT signature, ix_index, ix_type, discriminant, slot, block_time, account0, accounts, data_base64
         FROM events {} ORDER BY slot DESC, ix_index DESC LIMIT ${}",
        if where_clauses.is_empty() { String::new() } else { format!("WHERE {}", where_clauses.join(" AND ")) },
        params.len(),
    );
    let refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
        .iter()
        .map(|b| b.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
        .collect();
    let rows = client.query(&sql, &refs).await?;
    Ok(rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "signature": r.get::<_, String>(0),
                "ixIndex": r.get::<_, i32>(1),
                "ixType": r.get::<_, String>(2),
                "discriminant": r.get::<_, i16>(3),
                "slot": r.get::<_, i64>(4),
                "blockTime": r.get::<_, Option<i64>>(5),
                "account0": r.get::<_, Option<String>>(6),
                "accounts": r.get::<_, serde_json::Value>(7),
                "dataBase64": r.get::<_, String>(8),
            })
        })
        .collect())
}

/// Index oracle metadata from a `write_oracle_meta` instruction. The account is
/// write-once on-chain, so keep the first row (idempotent re-processing).
#[allow(clippy::too_many_arguments)]
pub async fn insert_oracle_meta(
    client: &Client,
    oracle: &str,
    subject: &str,
    options: &serde_json::Value,
    uri: &str,
    uri_hash: &str,
    slot: i64,
    signature: &str,
) -> Result<()> {
    client
        .execute(
            "INSERT INTO oracle_metadata (oracle, subject, options, uri, uri_hash, slot, signature)
             VALUES ($1,$2,$3,$4,$5,$6,$7)
             ON CONFLICT (oracle) DO NOTHING",
            &[
                &oracle, &subject, options, &uri, &uri_hash, &slot, &signature,
            ],
        )
        .await?;
    Ok(())
}

fn meta_json(r: &tokio_postgres::Row) -> serde_json::Value {
    serde_json::json!({
        "oracle": r.get::<_, String>(0),
        "subject": r.get::<_, String>(1),
        "options": r.get::<_, serde_json::Value>(2),
        "uri": r.get::<_, String>(3),
        "uriHash": r.get::<_, String>(4),
        "slot": r.get::<_, i64>(5),
    })
}

const META_COLS: &str = "oracle, subject, options, uri, uri_hash, slot";

/// Oracle metadata for a single oracle PDA, if indexed.
pub async fn get_oracle_meta(client: &Client, oracle: &str) -> Result<Option<serde_json::Value>> {
    let sql = format!("SELECT {META_COLS} FROM oracle_metadata WHERE oracle = $1");
    let rows = client.query(&sql, &[&oracle]).await?;
    Ok(rows.first().map(meta_json))
}

/// Oracle metadata for a batch of oracle PDAs (browse view). Empty input → all
/// indexed metadata (capped), so the list page can prefetch in one call.
pub async fn list_oracle_meta(
    client: &Client,
    oracles: &[String],
    limit: i64,
) -> Result<Vec<serde_json::Value>> {
    let rows = if oracles.is_empty() {
        let sql = format!("SELECT {META_COLS} FROM oracle_metadata ORDER BY slot DESC LIMIT $1");
        client.query(&sql, &[&limit.min(1000)]).await?
    } else {
        let sql = format!("SELECT {META_COLS} FROM oracle_metadata WHERE oracle = ANY($1)");
        client.query(&sql, &[&oracles]).await?
    };
    Ok(rows.iter().map(meta_json).collect())
}

/// The on-chain `uri_hash` (hex) indexed for an oracle — the gate the JSON host
/// checks a POSTed/served JSON against.
pub async fn get_oracle_uri_hash(client: &Client, oracle: &str) -> Result<Option<String>> {
    let rows = client
        .query(
            "SELECT uri_hash FROM oracle_metadata WHERE oracle = $1",
            &[&oracle],
        )
        .await?;
    Ok(rows.first().map(|r| r.get::<_, String>(0)))
}

/// Store the hosted extended-metadata JSON for an oracle (app POST). Upsert:
/// the latest POST wins (the serve path gates it against the on-chain uri_hash).
pub async fn upsert_oracle_meta_json(
    client: &Client,
    oracle: &str,
    json: &str,
    sha256: &str,
) -> Result<()> {
    client
        .execute(
            "INSERT INTO oracle_meta_json (oracle, json, sha256) VALUES ($1,$2,$3)
             ON CONFLICT (oracle) DO UPDATE SET json = EXCLUDED.json, sha256 = EXCLUDED.sha256",
            &[&oracle, &json, &sha256],
        )
        .await?;
    Ok(())
}

/// The hosted JSON + its sha256 for an oracle, if any was POSTed.
pub async fn get_oracle_meta_json(
    client: &Client,
    oracle: &str,
) -> Result<Option<(String, String)>> {
    let rows = client
        .query(
            "SELECT json, sha256 FROM oracle_meta_json WHERE oracle = $1",
            &[&oracle],
        )
        .await?;
    Ok(rows
        .first()
        .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1))))
}

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

/// Oracles that have a non-empty on-chain `uri` but NO stored JSON matching their
/// `uri_hash` yet — the work list for the autonomous metadata fetcher. Naturally
/// excludes oracles whose JSON was already POSTed (self-hosted) and keeps failed
/// fetches in the set so they retry next tick.
pub async fn oracles_missing_meta_json(
    client: &Client,
    limit: i64,
) -> Result<Vec<(String, String, String)>> {
    let rows = client
        .query(
            "SELECT m.oracle, m.uri, m.uri_hash
             FROM oracle_metadata m
             LEFT JOIN oracle_meta_json j
               ON j.oracle = m.oracle AND j.sha256 = m.uri_hash
             WHERE m.uri <> '' AND j.oracle IS NULL
             ORDER BY m.slot DESC
             LIMIT $1",
            &[&limit],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| {
            (
                r.get::<_, String>(0),
                r.get::<_, String>(1),
                r.get::<_, String>(2),
            )
        })
        .collect())
}

#[cfg(test)]
mod it {
    //! Integration tests for the db layer against a REAL, ephemeral Postgres —
    //! the SQL is Postgres-specific (`JSONB`, `ANY($1)` arrays, `$N` placeholders,
    //! `ON CONFLICT … EXCLUDED`), so it must run on the real engine, not a SQLite
    //! stand-in that would exercise a rewritten query. Self-skips (never fails)
    //! when `TEST_DATABASE_URL` is unset — the dedicated CI `db-it` job provides a
    //! Postgres service and sets it; every other run just skips.

    use super::*;

    async fn test_client() -> Option<Arc<Client>> {
        let url = std::env::var("TEST_DATABASE_URL").ok()?;
        match connect(&url).await {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!("[db_it] connect to TEST_DATABASE_URL failed ({e}); skipping");
                None
            }
        }
    }

    #[tokio::test]
    async fn oracle_meta_db_round_trips_against_postgres() {
        let Some(client) = test_client().await else {
            eprintln!("[db_it] TEST_DATABASE_URL unset — skipping Postgres integration test");
            return;
        };
        // Isolate: one comprehensive scenario, clean slate (runs alone; no other db
        // test touches these tables).
        client
            .batch_execute("TRUNCATE oracle_metadata, oracle_meta_json")
            .await
            .expect("truncate");

        let opts = serde_json::json!(["Yes", "No"]);

        // insert + get: subject/options/uri/uriHash/slot round-trip.
        insert_oracle_meta(
            &client,
            "OraA",
            "Q A?",
            &opts,
            "https://h/a.json",
            "aa",
            10,
            "sigA",
        )
        .await
        .unwrap();
        let meta = get_oracle_meta(&client, "OraA")
            .await
            .unwrap()
            .expect("row A");
        assert_eq!(meta["subject"], "Q A?");
        assert_eq!(meta["options"], opts);
        assert_eq!(meta["uri"], "https://h/a.json");
        assert_eq!(meta["uriHash"], "aa");
        assert_eq!(meta["slot"], 10);
        assert!(get_oracle_meta(&client, "absent").await.unwrap().is_none());

        // idempotent: the account is write-once on-chain, so a re-processed ix keeps
        // the first row (ON CONFLICT DO NOTHING).
        insert_oracle_meta(&client, "OraA", "CHANGED", &opts, "u2", "bb", 11, "sigA2")
            .await
            .unwrap();
        assert_eq!(
            get_oracle_meta(&client, "OraA").await.unwrap().unwrap()["subject"],
            "Q A?",
            "second insert must not overwrite"
        );

        // get_oracle_uri_hash.
        assert_eq!(
            get_oracle_uri_hash(&client, "OraA")
                .await
                .unwrap()
                .as_deref(),
            Some("aa")
        );
        assert!(get_oracle_uri_hash(&client, "absent")
            .await
            .unwrap()
            .is_none());

        // list_oracle_meta batch via `ANY($1)` — the Postgres array bind SQLite lacks.
        insert_oracle_meta(&client, "OraB", "Q B?", &opts, "", "bb", 20, "sigB")
            .await
            .unwrap();
        insert_oracle_meta(
            &client,
            "OraC",
            "Q C?",
            &opts,
            "https://h/c.json",
            "cc",
            30,
            "sigC",
        )
        .await
        .unwrap();
        let list = list_oracle_meta(&client, &["OraA".into(), "OraC".into()], 500)
            .await
            .unwrap();
        let mut got: Vec<&str> = list.iter().map(|m| m["oracle"].as_str().unwrap()).collect();
        got.sort();
        assert_eq!(got, vec!["OraA", "OraC"]);

        // meta_json upsert + get; upsert overwrites (latest POST wins).
        upsert_oracle_meta_json(&client, "OraA", "{\"v\":1}", "sha_a")
            .await
            .unwrap();
        assert_eq!(
            get_oracle_meta_json(&client, "OraA")
                .await
                .unwrap()
                .unwrap(),
            ("{\"v\":1}".to_string(), "sha_a".to_string())
        );
        upsert_oracle_meta_json(&client, "OraA", "{\"v\":2}", "sha_a2")
            .await
            .unwrap();
        assert_eq!(
            get_oracle_meta_json(&client, "OraA")
                .await
                .unwrap()
                .unwrap()
                .1,
            "sha_a2"
        );
        assert!(get_oracle_meta_json(&client, "absent")
            .await
            .unwrap()
            .is_none());

        // oracles_missing_meta_json — the LEFT JOIN work list. At this point:
        //   OraA: uri set, uri_hash "aa"; stored json sha "sha_a2" ≠ "aa" → MISSING (stale).
        //   OraB: empty uri                                          → excluded.
        //   OraC: uri set, uri_hash "cc", no stored json            → MISSING.
        let missing: Vec<String> = oracles_missing_meta_json(&client, 100)
            .await
            .unwrap()
            .into_iter()
            .map(|(o, _, _)| o)
            .collect();
        assert!(
            missing.contains(&"OraA".to_string()),
            "stale-hash oracle in work list"
        );
        assert!(
            missing.contains(&"OraC".to_string()),
            "no-json oracle in work list"
        );
        assert!(
            !missing.contains(&"OraB".to_string()),
            "empty-uri oracle excluded"
        );

        // Store MATCHING json for both → the work list drains to empty.
        upsert_oracle_meta_json(&client, "OraA", "{}", "aa")
            .await
            .unwrap();
        upsert_oracle_meta_json(&client, "OraC", "{}", "cc")
            .await
            .unwrap();
        let missing_after = oracles_missing_meta_json(&client, 100).await.unwrap();
        assert!(
            missing_after.is_empty(),
            "all committed json now matches: {missing_after:?}"
        );
    }
}

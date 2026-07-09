//! Account-level indexing for the kassandra ORACLE program.
//!
//! A `getProgramAccounts` snapshot on startup + a live `programSubscribe` tail
//! mirror every Oracle and its child accounts (Proposer/Fact/FactVote/AiClaim/
//! Market) into `oracle_accounts` as raw Pod bytes, keyed by account type and by
//! the parent oracle. The app then reads oracle lists + detail from Postgres (an
//! indexed lookup) instead of slow client-side `getProgramAccounts`, decoding the
//! served raw bytes with the SAME SDK decoders (no decode logic duplicated here).
//!
//! Mirrors the market-side account pipeline (`crate::market`): the subscribe tail
//! can't observe an account CLOSE, so a periodic gpa snapshot + slot-aware prune is
//! the only path that removes closed rows.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use carbon_core::account::{AccountDecoder, AccountProcessorInputType, DecodedAccount};
use carbon_core::error::CarbonResult;
use carbon_core::processor::Processor;
use solana_pubkey::Pubkey;
use tokio_postgres::Client;

use crate::market::rpc::Rpc;

/// `account_type` tag bytes — from `programs/kassandra/src/state.rs` (`AccountType`).
/// Oracle=1, Proposer=2, Fact=3, FactVote=4, AiClaim=5, Market=6. (Protocol=7 and
/// OracleMeta=8 are indexed elsewhere / not needed by the oracle browse+detail.)
const TAG_ORACLE: u8 = 1;
/// The child account types the browse/detail views read. Every child stores its
/// parent `oracle` pubkey at byte offset 8 (right after the 8-byte header).
const CHILD_TAGS: [u8; 5] = [2, 3, 4, 5, 6];
const CHILD_ORACLE_OFFSET: usize = 8;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS oracle_accounts (
  pubkey       TEXT     PRIMARY KEY,
  account_type SMALLINT NOT NULL,   -- 1=Oracle 2=Proposer 3=Fact 4=FactVote 5=AiClaim 6=Market
  oracle_ref   TEXT     NOT NULL,   -- parent oracle (== self for an Oracle row) for scoped lookups
  slot         BIGINT   NOT NULL,
  data         BYTEA    NOT NULL    -- raw Pod bytes; decoded on read with the SDK
);
CREATE INDEX IF NOT EXISTS oracle_accounts_type_idx ON oracle_accounts (account_type);
CREATE INDEX IF NOT EXISTS oracle_accounts_ref_idx  ON oracle_accounts (oracle_ref);
"#;

/// Create the oracle-accounts schema (idempotent; shares the connection).
pub async fn create_schema(client: &Client) -> Result<()> {
    client.batch_execute(SCHEMA).await?;
    Ok(())
}

// --- carbon account decoder --------------------------------------------------

/// The minimal decode the pipeline needs: the account_type tag + the parent oracle
/// for children (the Oracle row's parent is itself, resolved in the processor from
/// the account pubkey).
#[derive(Clone, Debug)]
pub struct RawOracleAccount {
    pub tag: u8,
    /// The parent oracle pubkey (base58) for a child; `None` for the Oracle itself.
    pub child_oracle: Option<String>,
}

/// A Carbon `AccountDecoder` that keeps only this program's Oracle + child accounts
/// and extracts their (tag, parent-oracle) without a full Pod decode — the raw
/// bytes are persisted verbatim and decoded by the SDK on read.
pub struct OracleAccountDecoder {
    pub program_id: Pubkey,
}

impl<'a> AccountDecoder<'a> for OracleAccountDecoder {
    type AccountType = RawOracleAccount;

    fn decode_account(
        &self,
        account: &'a solana_account::Account,
    ) -> Option<DecodedAccount<Self::AccountType>> {
        if account.owner.to_bytes() != self.program_id.to_bytes() {
            return None;
        }
        let data = account.data.as_slice();
        let tag = *data.first()?;
        let child_oracle = if tag == TAG_ORACLE {
            None
        } else if CHILD_TAGS.contains(&tag) {
            let bytes: [u8; 32] = data
                .get(CHILD_ORACLE_OFFSET..CHILD_ORACLE_OFFSET + 32)?
                .try_into()
                .ok()?;
            Some(bs58::encode(bytes).into_string())
        } else {
            return None; // Protocol / OracleMeta / unknown — not part of browse+detail.
        };
        Some(DecodedAccount {
            lamports: account.lamports,
            data: RawOracleAccount { tag, child_oracle },
            owner: account.owner,
            executable: account.executable,
            rent_epoch: account.rent_epoch,
        })
    }
}

// --- carbon processor + shared persist --------------------------------------

pub struct OracleAccountProcessor {
    pub client: Arc<Client>,
}

impl Processor<AccountProcessorInputType<'_, RawOracleAccount>> for OracleAccountProcessor {
    async fn process(
        &mut self,
        input: &AccountProcessorInputType<'_, RawOracleAccount>,
    ) -> CarbonResult<()> {
        persist(
            &self.client,
            &input.metadata.pubkey.to_string(),
            &input.decoded_account.data,
            input.raw_account.data.as_slice(),
            input.metadata.slot as i64,
        )
        .await;
        Ok(())
    }
}

/// Slot-gated upsert of one account's raw bytes. Shared by the live pipeline and the
/// reconcile loop. Errors are logged, not propagated (one bad row never kills it).
pub async fn persist(
    client: &Client,
    pubkey: &str,
    decoded: &RawOracleAccount,
    data: &[u8],
    slot: i64,
) {
    // The Oracle row's parent is itself; children carry their parent from offset 8.
    let oracle_ref = decoded.child_oracle.as_deref().unwrap_or(pubkey);
    if let Err(e) = upsert_account(client, pubkey, decoded.tag as i16, oracle_ref, slot, data).await
    {
        log::warn!("[oracle-acct] upsert {pubkey} failed: {e}");
    }
}

// --- db ----------------------------------------------------------------------

/// Slot-gated upsert (an update only applies if its slot `>=` the stored slot).
pub async fn upsert_account(
    client: &Client,
    pubkey: &str,
    account_type: i16,
    oracle_ref: &str,
    slot: i64,
    data: &[u8],
) -> Result<()> {
    client
        .execute(
            "INSERT INTO oracle_accounts (pubkey, account_type, oracle_ref, slot, data)
             VALUES ($1,$2,$3,$4,$5)
             ON CONFLICT (pubkey) DO UPDATE
               SET account_type = EXCLUDED.account_type,
                   oracle_ref   = EXCLUDED.oracle_ref,
                   slot         = EXCLUDED.slot,
                   data         = EXCLUDED.data
               WHERE oracle_accounts.slot <= EXCLUDED.slot",
            &[&pubkey, &account_type, &oracle_ref, &slot, &data],
        )
        .await?;
    Ok(())
}

/// Remove rows CLOSED on-chain — absent from an authoritative snapshot at
/// `snapshot_slot` whose stored slot is `<= snapshot_slot` (newer rows are kept).
pub async fn prune(client: &Client, snapshot_slot: i64, present: &HashSet<String>) -> Result<u64> {
    let present_vec: Vec<String> = present.iter().cloned().collect();
    let n = client
        .execute(
            "DELETE FROM oracle_accounts WHERE slot <= $1 AND NOT (pubkey = ANY($2))",
            &[&snapshot_slot, &present_vec],
        )
        .await
        .context("prune oracle_accounts")?;
    Ok(n)
}

/// One served account: its pubkey + raw Pod bytes (the API base64-encodes `data`).
pub struct StoredAccount {
    pub pubkey: String,
    pub data: Vec<u8>,
}

/// Every indexed Oracle account (for the browse list).
pub async fn list_oracles(client: &Client) -> Result<Vec<StoredAccount>> {
    let rows = client
        .query(
            "SELECT pubkey, data FROM oracle_accounts WHERE account_type = $1 ORDER BY slot DESC",
            &[&(TAG_ORACLE as i16)],
        )
        .await?;
    Ok(rows.iter().map(row_account).collect())
}

/// The oracle itself + every child scoped to it (for the detail view).
pub async fn list_by_oracle(client: &Client, oracle: &str) -> Result<Vec<(i16, StoredAccount)>> {
    let rows = client
        .query(
            "SELECT account_type, pubkey, data FROM oracle_accounts WHERE oracle_ref = $1",
            &[&oracle],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| {
            (
                r.get::<_, i16>(0),
                StoredAccount {
                    pubkey: r.get::<_, String>(1),
                    data: r.get::<_, Vec<u8>>(2),
                },
            )
        })
        .collect())
}

fn row_account(r: &tokio_postgres::Row) -> StoredAccount {
    StoredAccount {
        pubkey: r.get::<_, String>(0),
        data: r.get::<_, Vec<u8>>(1),
    }
}

// --- reconcile (startup snapshot + periodic prune) ---------------------------

/// One gpa snapshot: upsert every current account + prune the closed ones. Called
/// once at startup (authoritative seed before the subscribe tail) and periodically.
pub async fn reconcile_once(rpc: &Rpc, client: &Client, program_id: &Pubkey) -> Result<usize> {
    let decoder = OracleAccountDecoder {
        program_id: *program_id,
    };
    let slot = rpc.get_slot().await? as i64;
    let accounts = rpc.get_program_accounts(program_id).await?;
    let mut present: HashSet<String> = HashSet::new();
    let mut n = 0;
    for (pubkey, account) in accounts {
        if let Some(decoded) = decoder.decode_account(&account) {
            let key = pubkey.to_string();
            persist(client, &key, &decoded.data, account.data.as_slice(), slot).await;
            present.insert(key);
            n += 1;
        }
    }
    prune(client, slot, &present).await?;
    Ok(n)
}

/// Periodic reconcile — the only path that removes accounts closed on-chain (the
/// subscribe tail never observes a close).
pub async fn reconcile_loop(
    rpc: Arc<Rpc>,
    client: Arc<Client>,
    program_id: Pubkey,
    interval_ms: u64,
) {
    loop {
        tokio::time::sleep(Duration::from_millis(interval_ms)).await;
        match reconcile_once(&rpc, &client, &program_id).await {
            Ok(n) => log::debug!("[oracle-acct] reconcile: {n} accounts"),
            Err(e) => log::warn!("[oracle-acct] reconcile failed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_account::Account;

    fn account(owner: Pubkey, data: Vec<u8>) -> Account {
        Account {
            lamports: 42,
            data,
            owner,
            executable: false,
            rent_epoch: 0,
        }
    }

    #[test]
    fn decoder_extracts_tag_and_parent_oracle() {
        let prog = Pubkey::new_unique();
        let dec = OracleAccountDecoder { program_id: prog };

        // Oracle account (tag 1): parent is itself → child_oracle None (resolved to
        // the pubkey in the processor).
        let mut oracle_data = vec![0u8; 368];
        oracle_data[0] = TAG_ORACLE;
        let d = dec.decode_account(&account(prog, oracle_data)).unwrap();
        assert_eq!(d.data.tag, TAG_ORACLE);
        assert!(d.data.child_oracle.is_none());

        // Fact child (tag 3): parent oracle pubkey at offset 8.
        let oracle_pk = Pubkey::new_unique();
        let mut fact_data = vec![0u8; 336];
        fact_data[0] = 3;
        fact_data[CHILD_ORACLE_OFFSET..CHILD_ORACLE_OFFSET + 32]
            .copy_from_slice(&oracle_pk.to_bytes());
        let d = dec.decode_account(&account(prog, fact_data)).unwrap();
        assert_eq!(d.data.tag, 3);
        assert_eq!(
            d.data.child_oracle.as_deref(),
            Some(bs58::encode(oracle_pk.to_bytes()).into_string().as_str())
        );

        // Foreign owner → skipped.
        assert!(dec
            .decode_account(&account(Pubkey::new_unique(), vec![TAG_ORACLE; 368]))
            .is_none());
        // Unknown tag (Protocol=7 / OracleMeta=8) → skipped.
        let mut proto = vec![0u8; 100];
        proto[0] = 7;
        assert!(dec.decode_account(&account(prog, proto)).is_none());
    }

    async fn test_client() -> Option<Arc<Client>> {
        let url = std::env::var("TEST_DATABASE_URL").ok()?;
        let client = crate::db::connect(&url).await.ok()?;
        create_schema(&client).await.ok()?;
        Some(client)
    }

    #[tokio::test]
    async fn oracle_accounts_round_trip_against_postgres() {
        let Some(client) = test_client().await else {
            eprintln!("[oracle-acct it] TEST_DATABASE_URL unset — skipping");
            return;
        };
        client
            .batch_execute("TRUNCATE oracle_accounts")
            .await
            .unwrap();

        upsert_account(&client, "OraA", 1, "OraA", 10, b"oracle-A")
            .await
            .unwrap();
        upsert_account(&client, "FactA1", 3, "OraA", 11, b"fact-A1")
            .await
            .unwrap();
        upsert_account(&client, "PropA1", 2, "OraA", 12, b"prop-A1")
            .await
            .unwrap();
        upsert_account(&client, "FactB1", 3, "OraB", 13, b"fact-B1")
            .await
            .unwrap();

        // list_oracles → only the Oracle-type rows.
        let oracles = list_oracles(&client).await.unwrap();
        assert_eq!(oracles.len(), 1);
        assert_eq!(oracles[0].pubkey, "OraA");
        assert_eq!(oracles[0].data, b"oracle-A");

        // list_by_oracle → the oracle + its two children (scoped by oracle_ref).
        let a = list_by_oracle(&client, "OraA").await.unwrap();
        assert_eq!(a.len(), 3);
        assert_eq!(list_by_oracle(&client, "OraB").await.unwrap().len(), 1);

        // slot-gated: a stale (lower-slot) update is ignored; a newer one wins.
        upsert_account(&client, "OraA", 1, "OraA", 5, b"STALE")
            .await
            .unwrap();
        assert_eq!(list_oracles(&client).await.unwrap()[0].data, b"oracle-A");
        upsert_account(&client, "OraA", 1, "OraA", 20, b"oracle-A2")
            .await
            .unwrap();
        assert_eq!(list_oracles(&client).await.unwrap()[0].data, b"oracle-A2");

        // prune: a snapshot at slot 100 with only OraA present removes the children
        // (slot <= 100, absent) but keeps OraA.
        let mut present = HashSet::new();
        present.insert("OraA".to_string());
        let removed = prune(&client, 100, &present).await.unwrap();
        assert_eq!(removed, 3); // FactA1, PropA1, FactB1
        assert_eq!(list_by_oracle(&client, "OraA").await.unwrap().len(), 1);
    }
}

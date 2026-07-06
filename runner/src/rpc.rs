//! On-chain Oracle/Fact fetch over Solana JSON-RPC + the off-chain
//! prompt-text-by-hash source (Task I3).
//!
//! The runner can build its config from an oracle pubkey instead of an explicit
//! full config: it reads the `Oracle` account (and its agreed `Fact` accounts)
//! straight off chain and decodes them through the SHARED
//! `kassandra_sdk::accounts` `Pod` structs — zero new decode code. The
//! interpretation TEXT is NOT on chain; it lives in the `oracle_meta` uri JSON,
//! bound by `uri_hash`. [`fetch_oracle_meta`] reads the (program-owned) meta
//! account; the caller (`build_config_from_chain`) fetches the uri and verifies
//! `sha256(json) == uri_hash` before using it — the same fetch-by-uri, check-the-
//! hash contract as the fact content.
//!
//! # Transport (no solana-client)
//!
//! Everything is plain JSON-RPC over the same `reqwest` stack the fact fetcher
//! uses — no `solana-client`/`solana-sdk` dependency. Requests go through the
//! [`JsonRpc`] trait so the whole decode path is exercised OFFLINE in tests via
//! [`MockRpc`] (mirroring [`crate::fetch::MockFactFetcher`]).
//!
//! # Account validation before decode
//!
//! Every fetched account is validated before it is `bytemuck`-decoded:
//! - the account MUST be owned by the Kassandra program (`kassandra_sdk::PROGRAM_ID`);
//! - its first byte (the [`AccountType`] tag) MUST match the expected type;
//! - its length MUST be at least the struct's `LEN`.
//!
//! This rejects type-confusion (a `Fact` handed where an `Oracle` is expected)
//! and foreign accounts before any bytes are interpreted.
//!
//! # Enumerating an oracle's agreed facts
//!
//! A `Fact` PDA is `[b"fact", oracle, content_hash]`, so the fact set CANNOT be
//! enumerated from the oracle alone (the `content_hash`es aren't known up
//! front). Instead [`fetch_agreed_facts`] uses `getProgramAccounts` with a
//! `dataSize == Fact::LEN` filter plus a `memcmp` on the `Fact.oracle` field
//! (offset [`FACT_ORACLE_OFFSET`]) to pull exactly this oracle's `Fact`
//! accounts, decodes each, and keeps the ones whose `agreed` flag is set.

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde_json::{json, Value};
use std::str::FromStr;

use kassandra_sdk::accounts::{AccountType, Fact, Oracle};

/// Byte offset of the `oracle` field inside a `Fact` account — the
/// `getProgramAccounts` `memcmp` anchor used to enumerate an oracle's facts.
/// Tied to the shared struct so a layout change breaks the build.
pub const FACT_ORACLE_OFFSET: usize = core::mem::offset_of!(Fact, oracle);
const _: () = assert!(FACT_ORACLE_OFFSET == 8);

/// A transport- or protocol-level failure talking to the RPC endpoint, or an
/// account that failed validation before decode.
#[derive(Debug, thiserror::Error)]
pub enum RpcError {
    /// A transport/network error (DNS, connection, TLS, timeout).
    #[error("RPC request to `{url}` failed: {message}")]
    Transport {
        /// The RPC url.
        url: String,
        /// The rendered underlying error.
        message: String,
    },
    /// A non-success HTTP status from the RPC endpoint.
    #[error("RPC `{url}` returned non-success HTTP status {status}")]
    HttpStatus {
        /// The RPC url.
        url: String,
        /// The HTTP status code.
        status: u16,
    },
    /// The JSON-RPC envelope carried an `error` object.
    #[error("RPC method `{method}` returned error {code}: {message}")]
    JsonRpc {
        /// The RPC method that failed.
        method: String,
        /// The JSON-RPC error code.
        code: i64,
        /// The JSON-RPC error message.
        message: String,
    },
    /// The RPC response JSON didn't have the expected shape.
    #[error("malformed RPC response for `{method}`: {detail}")]
    Malformed {
        /// The RPC method whose response was malformed.
        method: String,
        /// What was wrong.
        detail: String,
    },
    /// `getAccountInfo` returned `null` — the account doesn't exist.
    #[error("account `{pubkey}` not found on chain")]
    AccountNotFound {
        /// The queried pubkey (base58).
        pubkey: String,
    },
    /// The account is not owned by the Kassandra program.
    #[error("account `{pubkey}` is owned by `{owner}`, not the Kassandra program `{expected}`")]
    WrongOwner {
        /// The queried pubkey (base58).
        pubkey: String,
        /// The account's actual owner (base58).
        owner: String,
        /// The expected owner (the program id, base58).
        expected: String,
    },
    /// The account's [`AccountType`] tag byte did not match the expected type.
    #[error(
        "account `{pubkey}` has account_type tag {actual}, expected {expected} ({expected_name})"
    )]
    WrongAccountType {
        /// The queried pubkey (base58).
        pubkey: String,
        /// The expected tag byte.
        expected: u8,
        /// A human name for the expected type.
        expected_name: &'static str,
        /// The actual tag byte found.
        actual: u8,
    },
    /// The account data was shorter than the decoded struct requires.
    #[error("account `{pubkey}` data is {actual} bytes, need at least {needed} for {type_name}")]
    ShortData {
        /// The queried pubkey (base58).
        pubkey: String,
        /// The struct that was being decoded.
        type_name: &'static str,
        /// Bytes required.
        needed: usize,
        /// Bytes present.
        actual: usize,
    },
}

/// A minimal Solana JSON-RPC transport: one `call(method, params) -> result`.
///
/// Behind a trait so the whole account-decode path runs OFFLINE in tests via
/// [`MockRpc`]. [`HttpJsonRpc`] is the real `reqwest`-backed default.
#[async_trait]
pub trait JsonRpc {
    /// Invoke a JSON-RPC `method` with `params`, returning the `result` value
    /// (or an [`RpcError`] for transport / HTTP / JSON-RPC-error failures).
    async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError>;
}

/// The real `reqwest`-based JSON-RPC client (POSTs the standard
/// `{jsonrpc, id, method, params}` envelope).
#[derive(Clone, Debug)]
pub struct HttpJsonRpc {
    client: reqwest::Client,
    url: String,
}

impl HttpJsonRpc {
    /// Build a client for `url` with the fact fetcher's default timeout.
    pub fn new(url: impl Into<String>) -> Result<Self, RpcError> {
        let url = url.into();
        let client = reqwest::Client::builder()
            .timeout(crate::fetch::DEFAULT_FETCH_TIMEOUT)
            .build()
            .map_err(|e| RpcError::Transport {
                url: url.clone(),
                message: e.to_string(),
            })?;
        Ok(Self { client, url })
    }
}

#[async_trait]
impl JsonRpc for HttpJsonRpc {
    async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });
        let resp = self
            .client
            .post(&self.url)
            .json(&body)
            .send()
            .await
            .map_err(|e| RpcError::Transport {
                url: self.url.clone(),
                message: e.to_string(),
            })?;

        let status = resp.status();
        if !status.is_success() {
            return Err(RpcError::HttpStatus {
                url: self.url.clone(),
                status: status.as_u16(),
            });
        }

        let value: Value = resp.json().await.map_err(|e| RpcError::Transport {
            url: self.url.clone(),
            message: e.to_string(),
        })?;

        if let Some(err) = value.get("error") {
            let code = err.get("code").and_then(Value::as_i64).unwrap_or(0);
            let message = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("<no message>")
                .to_string();
            return Err(RpcError::JsonRpc {
                method: method.to_string(),
                code,
                message,
            });
        }

        value
            .get("result")
            .cloned()
            .ok_or_else(|| RpcError::Malformed {
                method: method.to_string(),
                detail: "response had neither `result` nor `error`".to_string(),
            })
    }
}

/// A decoded RPC account: the raw data bytes + the owner program pubkey.
struct RawAccount {
    data: Vec<u8>,
    owner: [u8; 32],
}

/// Decode a `{ data: [base64, "base64"], owner: "<base58>" }` account JSON
/// object into raw bytes + owner pubkey.
fn parse_account(method: &str, account: &Value) -> Result<RawAccount, RpcError> {
    let malformed = |detail: &str| RpcError::Malformed {
        method: method.to_string(),
        detail: detail.to_string(),
    };

    let data_arr = account
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| malformed("account.data is not an array"))?;
    let b64 = data_arr
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| malformed("account.data[0] is not a string"))?;
    // Guard the encoding tag: we always request base64.
    if data_arr.get(1).and_then(Value::as_str) != Some("base64") {
        return Err(malformed("account.data encoding is not base64"));
    }
    let data = BASE64
        .decode(b64)
        .map_err(|e| malformed(&format!("account.data base64 decode failed: {e}")))?;

    let owner_str = account
        .get("owner")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed("account.owner is not a string"))?;
    let owner = decode_pubkey(owner_str)
        .map_err(|e| malformed(&format!("account.owner is not a valid pubkey: {e}")))?;

    Ok(RawAccount { data, owner })
}

/// Base58-decode a 32-byte pubkey string.
fn decode_pubkey(s: &str) -> Result<[u8; 32], String> {
    let bytes = bs58::decode(s)
        .into_vec()
        .map_err(|e| format!("base58 decode failed: {e}"))?;
    bytes
        .try_into()
        .map_err(|v: Vec<u8>| format!("expected 32 bytes, got {}", v.len()))
}

/// Validate `raw` as an account of `expected_type` (owner + tag + length),
/// returning the trimmed byte slice to decode from.
fn validate<'a>(
    pubkey: &str,
    raw: &'a RawAccount,
    expected_type: AccountType,
    type_name: &'static str,
    needed: usize,
) -> Result<&'a [u8], RpcError> {
    if raw.owner != kassandra_sdk::PROGRAM_ID.to_bytes() {
        return Err(RpcError::WrongOwner {
            pubkey: pubkey.to_string(),
            owner: bs58::encode(raw.owner).into_string(),
            expected: kassandra_sdk::PROGRAM_ID.to_string(),
        });
    }
    if raw.data.len() < needed {
        return Err(RpcError::ShortData {
            pubkey: pubkey.to_string(),
            type_name,
            needed,
            actual: raw.data.len(),
        });
    }
    let tag = raw.data[0];
    if tag != expected_type.as_u8() {
        return Err(RpcError::WrongAccountType {
            pubkey: pubkey.to_string(),
            expected: expected_type.as_u8(),
            expected_name: type_name,
            actual: tag,
        });
    }
    Ok(&raw.data[..needed])
}

/// Fetch + validate + `Pod`-decode the `Oracle` account at `oracle_pubkey`.
///
/// Verifies the account is owned by the Kassandra program and carries the
/// [`AccountType::Oracle`] tag before decoding through the shared
/// `kassandra_sdk::accounts::Oracle` struct.
pub async fn fetch_oracle(rpc: &dyn JsonRpc, oracle_pubkey: &str) -> Result<Oracle, RpcError> {
    let params = json!([
        oracle_pubkey,
        { "encoding": "base64", "commitment": "confirmed" }
    ]);
    let result = rpc.call("getAccountInfo", params).await?;
    let value = result.get("value").ok_or_else(|| RpcError::Malformed {
        method: "getAccountInfo".to_string(),
        detail: "response `result` had no `value`".to_string(),
    })?;
    if value.is_null() {
        return Err(RpcError::AccountNotFound {
            pubkey: oracle_pubkey.to_string(),
        });
    }
    let raw = parse_account("getAccountInfo", value)?;
    let bytes = validate(
        oracle_pubkey,
        &raw,
        AccountType::Oracle,
        "Oracle",
        Oracle::LEN,
    )?;
    // The SDK's `read` copies (unaligned-safe), so the RPC `Vec<u8>` is fine.
    kassandra_sdk::accounts::read::<Oracle>(bytes).map_err(|e| RpcError::Malformed {
        method: "getAccountInfo".to_string(),
        detail: format!("Oracle decode failed: {e}"),
    })
}

/// Fetch + validate + decode the companion `oracle_meta` account for an oracle.
///
/// Derives the `[b"oracle_meta", oracle]` PDA, reads it, verifies program
/// ownership + the [`AccountType::OracleMeta`] tag, and parses the length-prefixed
/// layout (`subject` / `options` / `uri` / `uri_hash`). This is how the runner
/// reads the interpretation source: the `uri` points at the metadata JSON and
/// `uri_hash` binds it (verified by the caller after fetching).
pub async fn fetch_oracle_meta(
    rpc: &dyn JsonRpc,
    oracle_pubkey: &str,
) -> Result<kassandra_sdk::accounts::OracleMeta, RpcError> {
    let oracle =
        solana_pubkey::Pubkey::from_str(oracle_pubkey).map_err(|e| RpcError::Malformed {
            method: "oracle_meta".to_string(),
            detail: format!("invalid oracle pubkey `{oracle_pubkey}`: {e}"),
        })?;
    let (meta_pda, _) = kassandra_sdk::pda::oracle_meta(&kassandra_sdk::PROGRAM_ID, &oracle);
    let meta_pk = meta_pda.to_string();

    let params = json!([
        meta_pk,
        { "encoding": "base64", "commitment": "confirmed" }
    ]);
    let result = rpc.call("getAccountInfo", params).await?;
    let value = result.get("value").ok_or_else(|| RpcError::Malformed {
        method: "getAccountInfo".to_string(),
        detail: "response `result` had no `value`".to_string(),
    })?;
    if value.is_null() {
        return Err(RpcError::AccountNotFound { pubkey: meta_pk });
    }
    let raw = parse_account("getAccountInfo", value)?;
    // Owner + tag + min-header-length checks (variable-length account).
    validate(&meta_pk, &raw, AccountType::OracleMeta, "OracleMeta", 34)?;
    kassandra_sdk::accounts::decode_oracle_meta(&raw.data).ok_or_else(|| RpcError::Malformed {
        method: "getAccountInfo".to_string(),
        detail: "OracleMeta decode failed".to_string(),
    })
}

/// An agreed fact read from chain: the on-chain `content_hash` commitment plus
/// the `uri` its off-chain content is served from — exactly what
/// [`crate::fetch::FactRef`] needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchedFact {
    /// The on-chain 32-byte `content_hash` commitment.
    pub content_hash: [u8; 32],
    /// The fact content `uri` (decoded from the on-chain `uri[..uri_len]`).
    pub uri: String,
}

/// Enumerate an oracle's AGREED facts via `getProgramAccounts`.
///
/// Filters to accounts of `dataSize == Fact::LEN` whose `Fact.oracle` field (at
/// [`FACT_ORACLE_OFFSET`]) equals `oracle_pubkey` (the `memcmp` `bytes` are
/// base58, the RPC default), decodes each through the shared
/// `kassandra_sdk::accounts::Fact` struct, and keeps the ones with the
/// `agreed` flag set. Facts are returned sorted by `content_hash` so the result
/// is deterministic regardless of RPC ordering (prompt assembly re-sorts too,
/// but a stable order keeps logs/tests predictable).
pub async fn fetch_agreed_facts(
    rpc: &dyn JsonRpc,
    oracle_pubkey: &str,
) -> Result<Vec<FetchedFact>, RpcError> {
    let program_id = kassandra_sdk::PROGRAM_ID.to_string();
    let params = json!([
        program_id,
        {
            "encoding": "base64",
            "commitment": "confirmed",
            "filters": [
                { "dataSize": Fact::LEN },
                { "memcmp": { "offset": FACT_ORACLE_OFFSET, "bytes": oracle_pubkey } }
            ]
        }
    ]);
    let result = rpc.call("getProgramAccounts", params).await?;
    let entries = result.as_array().ok_or_else(|| RpcError::Malformed {
        method: "getProgramAccounts".to_string(),
        detail: "result is not an array".to_string(),
    })?;

    let mut facts = Vec::new();
    for entry in entries {
        let pubkey = entry
            .get("pubkey")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>")
            .to_string();
        let account = entry.get("account").ok_or_else(|| RpcError::Malformed {
            method: "getProgramAccounts".to_string(),
            detail: "entry had no `account`".to_string(),
        })?;
        let raw = parse_account("getProgramAccounts", account)?;
        let bytes = validate(&pubkey, &raw, AccountType::Fact, "Fact", Fact::LEN)?;
        let fact =
            kassandra_sdk::accounts::read::<Fact>(bytes).map_err(|e| RpcError::Malformed {
                method: "getProgramAccounts".to_string(),
                detail: format!("Fact decode failed: {e}"),
            })?;

        if !fact.is_agreed() {
            continue;
        }
        let uri = decode_uri(&pubkey, &fact)?;
        facts.push(FetchedFact {
            content_hash: fact.content_hash,
            uri,
        });
    }

    facts.sort_by(|a, b| a.content_hash.cmp(&b.content_hash));
    Ok(facts)
}

/// Decode a `Fact`'s `uri[..uri_len]` as UTF-8.
fn decode_uri(pubkey: &str, fact: &Fact) -> Result<String, RpcError> {
    let len = fact.uri_len as usize;
    let malformed = |detail: String| RpcError::Malformed {
        method: "getProgramAccounts".to_string(),
        detail,
    };
    if len > fact.uri.len() {
        return Err(malformed(format!(
            "fact `{pubkey}` uri_len {len} exceeds the {}-byte uri field",
            fact.uri.len()
        )));
    }
    String::from_utf8(fact.uri[..len].to_vec())
        .map_err(|e| malformed(format!("fact `{pubkey}` uri is not valid UTF-8: {e}")))
}

// --- offline mock transport -------------------------------------------------

/// A deterministic, no-network [`JsonRpc`] backed by a `method -> canned result`
/// map. Used by tests (and cross-module tests) to serve canned
/// `getAccountInfo` / `getProgramAccounts` responses built from real
/// `Oracle`/`Fact` Pod byte layouts — mirrors [`crate::fetch::MockFactFetcher`].
#[derive(Clone, Debug, Default)]
pub struct MockRpc {
    responses: std::collections::HashMap<String, Value>,
}

impl MockRpc {
    /// An empty mock (every method errors as malformed/absent).
    pub fn new() -> Self {
        Self::default()
    }

    /// Register the `result` value returned for `method` (builder-style).
    pub fn with(mut self, method: impl Into<String>, result: Value) -> Self {
        self.responses.insert(method.into(), result);
        self
    }

    /// Base64-encode raw account bytes for a canned `account.data` field.
    pub fn base64(bytes: &[u8]) -> String {
        BASE64.encode(bytes)
    }

    /// The Kassandra program id as base58 (the canned account `owner`).
    pub fn program_owner() -> String {
        kassandra_sdk::PROGRAM_ID.to_string()
    }
}

#[async_trait]
impl JsonRpc for MockRpc {
    async fn call(&self, method: &str, _params: Value) -> Result<Value, RpcError> {
        self.responses
            .get(method)
            .cloned()
            .ok_or_else(|| RpcError::Malformed {
                method: method.to_string(),
                detail: "no canned response registered".to_string(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;
    use sha2::{Digest, Sha256};

    fn sha256(bytes: &[u8]) -> [u8; 32] {
        Sha256::digest(bytes).into()
    }

    /// Unwrap the error side of a `fetch_oracle` result (Oracle isn't `Debug`,
    /// so `unwrap_err` can't be used directly).
    fn expect_oracle_err(result: Result<Oracle, RpcError>) -> RpcError {
        match result {
            Ok(_) => panic!("expected an error, got a decoded Oracle"),
            Err(e) => e,
        }
    }

    /// Build a canned `getAccountInfo` result wrapping `data` owned by `owner`.
    fn account_info_result(data: &[u8], owner: &str) -> Value {
        json!({
            "context": { "slot": 1 },
            "value": {
                "data": [MockRpc::base64(data), "base64"],
                "owner": owner,
                "lamports": 1_000_000u64,
                "executable": false,
                "rentEpoch": 0u64,
                "space": data.len(),
            }
        })
    }

    /// Build a canned `getProgramAccounts` result: one entry per (pubkey, data).
    fn program_accounts_result(accounts: &[(&str, Vec<u8>)], owner: &str) -> Value {
        Value::Array(
            accounts
                .iter()
                .map(|(pk, data)| {
                    json!({
                        "pubkey": pk,
                        "account": {
                            "data": [MockRpc::base64(data), "base64"],
                            "owner": owner,
                            "lamports": 1_000_000u64,
                            "executable": false,
                            "rentEpoch": 0u64,
                            "space": data.len(),
                        }
                    })
                })
                .collect(),
        )
    }

    fn sample_oracle() -> Oracle {
        let mut o = Oracle::zeroed();
        o.account_type = AccountType::Oracle.as_u8();
        o.options_count = 3;
        o.deadline = 1_900_000_000;
        o
    }

    fn sample_fact(oracle: [u8; 32], content_hash: [u8; 32], uri: &str, agreed: bool) -> Fact {
        let mut f = Fact::zeroed();
        f.account_type = AccountType::Fact.as_u8();
        f.oracle = oracle.into();
        f.content_hash = content_hash;
        f.uri_len = uri.len() as u16;
        f.uri[..uri.len()].copy_from_slice(uri.as_bytes());
        f.agreed = u8::from(agreed);
        f
    }

    const ORACLE_PK: &str = "So11111111111111111111111111111111111111112";

    // --- oracle decode ------------------------------------------------------

    #[tokio::test]
    async fn fetch_oracle_decodes_shared_pod_fields() {
        let oracle = sample_oracle();
        let rpc = MockRpc::new().with(
            "getAccountInfo",
            account_info_result(bytemuck::bytes_of(&oracle), &MockRpc::program_owner()),
        );

        let got = fetch_oracle(&rpc, ORACLE_PK).await.unwrap();
        assert_eq!(got.options_count, 3);
        assert_eq!(got.deadline, 1_900_000_000);
    }

    #[tokio::test]
    async fn fetch_oracle_rejects_wrong_owner() {
        let oracle = sample_oracle();
        // Owned by some other program.
        let rpc = MockRpc::new().with(
            "getAccountInfo",
            account_info_result(bytemuck::bytes_of(&oracle), ORACLE_PK),
        );
        let err = expect_oracle_err(fetch_oracle(&rpc, ORACLE_PK).await);
        assert!(matches!(err, RpcError::WrongOwner { .. }), "{err}");
    }

    #[tokio::test]
    async fn fetch_oracle_rejects_wrong_account_type() {
        // A Fact-tagged blob padded to Oracle::LEN, owned by the program.
        let mut data = vec![0u8; Oracle::LEN];
        data[0] = AccountType::Fact.as_u8();
        let rpc = MockRpc::new().with(
            "getAccountInfo",
            account_info_result(&data, &MockRpc::program_owner()),
        );
        let err = expect_oracle_err(fetch_oracle(&rpc, ORACLE_PK).await);
        assert!(matches!(err, RpcError::WrongAccountType { .. }), "{err}");
    }

    #[tokio::test]
    async fn fetch_oracle_reports_not_found() {
        let rpc = MockRpc::new().with(
            "getAccountInfo",
            json!({ "context": { "slot": 1 }, "value": Value::Null }),
        );
        let err = expect_oracle_err(fetch_oracle(&rpc, ORACLE_PK).await);
        assert!(matches!(err, RpcError::AccountNotFound { .. }), "{err}");
    }

    // --- fact enumeration ---------------------------------------------------

    #[tokio::test]
    async fn fetch_agreed_facts_decodes_and_filters() {
        let oracle_bytes = decode_pubkey(ORACLE_PK).unwrap();
        let ch_a = sha256(b"fact A");
        let ch_b = sha256(b"fact B");
        let ch_c = sha256(b"not agreed");
        let agreed_a = sample_fact(oracle_bytes, ch_a, "https://f/a", true);
        let agreed_b = sample_fact(oracle_bytes, ch_b, "https://f/b", true);
        let not_agreed = sample_fact(oracle_bytes, ch_c, "https://f/c", false);

        let rpc = MockRpc::new().with(
            "getProgramAccounts",
            program_accounts_result(
                &[
                    (
                        "Fact111111111111111111111111111111111111111",
                        bytemuck::bytes_of(&agreed_a).to_vec(),
                    ),
                    (
                        "Fact222222222222222222222222222222222222222",
                        bytemuck::bytes_of(&not_agreed).to_vec(),
                    ),
                    (
                        "Fact333333333333333333333333333333333333333",
                        bytemuck::bytes_of(&agreed_b).to_vec(),
                    ),
                ],
                &MockRpc::program_owner(),
            ),
        );

        let facts = fetch_agreed_facts(&rpc, ORACLE_PK).await.unwrap();
        // Only the two agreed facts, sorted by content_hash.
        assert_eq!(facts.len(), 2);
        let mut expected = vec![
            FetchedFact {
                content_hash: ch_a,
                uri: "https://f/a".to_string(),
            },
            FetchedFact {
                content_hash: ch_b,
                uri: "https://f/b".to_string(),
            },
        ];
        expected.sort_by(|a, b| a.content_hash.cmp(&b.content_hash));
        assert_eq!(facts, expected);
    }

    #[tokio::test]
    async fn fetch_agreed_facts_empty_when_none() {
        let rpc = MockRpc::new().with("getProgramAccounts", Value::Array(vec![]));
        let facts = fetch_agreed_facts(&rpc, ORACLE_PK).await.unwrap();
        assert!(facts.is_empty());
    }
}

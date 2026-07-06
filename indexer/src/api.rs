//! Read-only JSON API over the indexed events.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use tokio_postgres::Client;
use tower_http::cors::{Any, CorsLayer};

use crate::db;

#[derive(Clone)]
pub struct ApiState {
    pub client: Arc<Client>,
    pub program_id: String,
    /// The upstream Solana RPC — the gateway forwards to it. Never leaves the
    /// backend (the app has no RPC endpoint of its own).
    pub rpc_url: String,
    pub http: reqwest::Client,
}

#[derive(Deserialize)]
pub struct EventsQuery {
    #[serde(rename = "type")]
    ix_type: Option<String>,
    account: Option<String>,
    #[serde(rename = "beforeSlot")]
    before_slot: Option<i64>,
    limit: Option<i64>,
}

pub fn router(state: ApiState) -> Router {
    // The read API is public + cross-origin (the dApp on another origin reads it),
    // so allow any origin for GETs. No credentials/cookies are involved.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/status", get(status))
        .route("/events", get(events))
        .route("/accounts/{pubkey}/events", get(account_events))
        // Oracle metadata indexed from `write_oracle_meta` (subject + option labels
        // + uri/uri_hash). `/oracles/meta?accounts=pk1,pk2` batches the browse list;
        // `/oracles/{pubkey}/meta` is the single-oracle detail read.
        .route("/oracles/meta", get(oracles_meta))
        .route("/oracles/{pubkey}/meta", get(oracle_meta))
        // Extended off-chain metadata JSON host (called by the PUBLIC app server,
        // which proxies its `/api/oracle/{pk}/metadata.json` here — the indexer is
        // private). POST stores the app-supplied JSON; GET serves it ONLY when its
        // sha256 matches the on-chain `uri_hash` (tamper-evident).
        .route(
            "/oracles/{pubkey}/meta-json",
            get(get_oracle_meta_json).post(post_oracle_meta_json),
        )
        // JSON-RPC gateway: the app performs ALL its chain work (reads, blockhash
        // for building txs, sendRawTransaction) through here, so the browser never
        // holds a Solana RPC endpoint.
        .route("/rpc", post(rpc_gateway))
        .layer(cors)
        .with_state(state)
}

/// Forward a JSON-RPC request body to the upstream RPC and relay the response.
async fn rpc_gateway(State(s): State<ApiState>, body: Bytes) -> impl IntoResponse {
    match s
        .http
        .post(&s.rpc_url)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
    {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            match resp.bytes().await {
                Ok(bytes) => {
                    (status, [(header::CONTENT_TYPE, "application/json")], bytes).into_response()
                }
                Err(e) => err(e).into_response(),
            }
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": format!("rpc upstream: {e}") })),
        )
            .into_response(),
    }
}

fn err(e: impl std::fmt::Display) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": e.to_string() })),
    )
}

async fn status(State(s): State<ApiState>) -> impl IntoResponse {
    match db::stats(&s.client).await {
        Ok((count, cursor)) => Json(serde_json::json!({
            "programId": s.program_id,
            "eventCount": count,
            "cursor": cursor.map(|(sig, slot)| serde_json::json!({ "signature": sig, "slot": slot })),
        }))
        .into_response(),
        Err(e) => err(e).into_response(),
    }
}

async fn events(State(s): State<ApiState>, Query(q): Query<EventsQuery>) -> impl IntoResponse {
    match db::query_events(
        &s.client,
        q.ix_type.as_deref(),
        q.account.as_deref(),
        q.before_slot,
        q.limit.unwrap_or(100),
    )
    .await
    {
        Ok(rows) => {
            Json(serde_json::json!({ "count": rows.len(), "events": rows })).into_response()
        }
        Err(e) => err(e).into_response(),
    }
}

async fn account_events(
    State(s): State<ApiState>,
    Path(pubkey): Path<String>,
    Query(q): Query<EventsQuery>,
) -> impl IntoResponse {
    match db::query_events(
        &s.client,
        None,
        Some(&pubkey),
        q.before_slot,
        q.limit.unwrap_or(100),
    )
    .await
    {
        Ok(rows) => {
            Json(serde_json::json!({ "account": pubkey, "count": rows.len(), "events": rows }))
                .into_response()
        }
        Err(e) => err(e).into_response(),
    }
}

async fn oracle_meta(State(s): State<ApiState>, Path(pubkey): Path<String>) -> impl IntoResponse {
    match db::get_oracle_meta(&s.client, &pubkey).await {
        Ok(Some(v)) => Json(v).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "no metadata for this oracle" })),
        )
            .into_response(),
        Err(e) => err(e).into_response(),
    }
}

/// POST the extended metadata JSON for an oracle (raw JSON body). Stores it with
/// its computed sha256; the serve path gates it against the on-chain uri_hash, so
/// storing an unverified/pending blob is harmless.
async fn post_oracle_meta_json(
    State(s): State<ApiState>,
    Path(pubkey): Path<String>,
    body: Bytes,
) -> impl IntoResponse {
    use sha2::{Digest, Sha256};
    let json = match std::str::from_utf8(&body) {
        Ok(j) => j,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "body is not valid UTF-8" })),
            )
                .into_response()
        }
    };
    let sha256: String = Sha256::digest(json.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    match db::upsert_oracle_meta_json(&s.client, &pubkey, json, &sha256).await {
        Ok(()) => Json(serde_json::json!({ "ok": true, "sha256": sha256 })).into_response(),
        Err(e) => err(e).into_response(),
    }
}

/// Serve the extended metadata JSON — ONLY when its sha256 matches the on-chain
/// `uri_hash` indexed for the oracle (tamper-evident). 404 if absent/not-yet-
/// indexed; 409 if a JSON is stored but does not match the committed hash.
async fn get_oracle_meta_json(
    State(s): State<ApiState>,
    Path(pubkey): Path<String>,
) -> impl IntoResponse {
    let stored = match db::get_oracle_meta_json(&s.client, &pubkey).await {
        Ok(v) => v,
        Err(e) => return err(e).into_response(),
    };
    let (json, sha256) = match stored {
        Some(v) => v,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "no metadata json for this oracle" })),
            )
                .into_response()
        }
    };
    let uri_hash = match db::get_oracle_uri_hash(&s.client, &pubkey).await {
        Ok(v) => v,
        Err(e) => return err(e).into_response(),
    };
    match uri_hash {
        Some(h) if h == sha256 => {
            ([(header::CONTENT_TYPE, "application/json")], json).into_response()
        }
        _ => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "stored json does not match the on-chain uri_hash (or oracle not yet indexed)"
            })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct MetaQuery {
    /// Comma-separated oracle PDAs to fetch (empty → the most-recent captured set).
    accounts: Option<String>,
}

async fn oracles_meta(State(s): State<ApiState>, Query(q): Query<MetaQuery>) -> impl IntoResponse {
    let oracles: Vec<String> = q
        .accounts
        .as_deref()
        .map(|a| {
            a.split(',')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    match db::list_oracle_meta(&s.client, &oracles, 500).await {
        Ok(rows) => Json(serde_json::json!({ "count": rows.len(), "meta": rows })).into_response(),
        Err(e) => err(e).into_response(),
    }
}

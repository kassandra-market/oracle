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
    let cors = CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any);
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/status", get(status))
        .route("/events", get(events))
        .route("/accounts/{pubkey}/events", get(account_events))
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
            let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            match resp.bytes().await {
                Ok(bytes) => (status, [(header::CONTENT_TYPE, "application/json")], bytes).into_response(),
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
        Ok(rows) => Json(serde_json::json!({ "count": rows.len(), "events": rows })).into_response(),
        Err(e) => err(e).into_response(),
    }
}

async fn account_events(
    State(s): State<ApiState>,
    Path(pubkey): Path<String>,
    Query(q): Query<EventsQuery>,
) -> impl IntoResponse {
    match db::query_events(&s.client, None, Some(&pubkey), q.before_slot, q.limit.unwrap_or(100)).await
    {
        Ok(rows) => {
            Json(serde_json::json!({ "account": pubkey, "count": rows.len(), "events": rows }))
                .into_response()
        }
        Err(e) => err(e).into_response(),
    }
}

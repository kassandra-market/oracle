//! Read-only JSON API over the indexed events.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
    Router,
};
use serde::Deserialize;
use tokio_postgres::Client;

use crate::db;

#[derive(Clone)]
pub struct ApiState {
    pub client: Arc<Client>,
    pub program_id: String,
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
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/status", get(status))
        .route("/events", get(events))
        .route("/accounts/{pubkey}/events", get(account_events))
        .with_state(state)
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

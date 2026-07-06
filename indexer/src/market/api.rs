//! axum data + transaction gateway (market side) over Postgres + on-demand RPC.
//!
//! Mounted under `/api/*` and merged with the oracle router in `main`. The
//! store-backed routes (`/api/config`, `/api/markets`) read `market_accounts`;
//! the enrichment + tx routes use the on-demand [`Rpc`] (503 if none configured).
//! No `/health` here — the oracle router owns the shared `/health`.

use std::str::FromStr;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::json;
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_transaction_status::TransactionConfirmationStatus;
use tokio_postgres::Client;
use tower_http::cors::{Any, CorsLayer};

use crate::market::json::{
    AccountDto, ConfigDto, ContributionDto, MarketDetailDto, MarketDto, OracleDto, ReservesDto,
};
use crate::market::rpc::Rpc;
use crate::market::AMM_ACCOUNT_DISCRIMINATOR;

// KASSANDRA oracle account field offsets (the oracle belongs to the Kassandra
// program, not ours — the 3 status bytes are read directly).
const ORACLE_OPTIONS_COUNT_OFFSET: usize = 160;
const ORACLE_PHASE_OFFSET: usize = 161;
const ORACLE_RESOLVED_OPTION_OFFSET: usize = 197;

// MetaDAO `Amm` account reserve offsets (base/quote `u64` LE), after the 8-byte
// Anchor account discriminator.
const AMM_BASE_AMOUNT_OFFSET: usize = 115;
const AMM_QUOTE_AMOUNT_OFFSET: usize = 123;

#[derive(Clone)]
pub struct AppState {
    pub client: Arc<Client>,
    pub rpc: Option<Arc<Rpc>>,
}

pub fn router(state: AppState) -> Router {
    let allowed = std::env::var("ALLOWED_ORIGIN").ok();
    let cors = match allowed.as_deref() {
        Some(origin) if origin != "*" && !origin.is_empty() => CorsLayer::new()
            .allow_origin(
                origin
                    .parse::<axum::http::HeaderValue>()
                    .expect("valid ALLOWED_ORIGIN"),
            )
            .allow_methods(Any)
            .allow_headers(Any),
        _ => CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any),
    };

    Router::new()
        .route("/api/config", get(get_config))
        .route("/api/markets", get(get_markets))
        .route("/api/markets/{pubkey}", get(get_market_detail))
        .route("/api/account/{pubkey}", get(get_account))
        .route("/api/blockhash", get(get_blockhash))
        .route("/api/transaction", post(post_transaction))
        .route("/api/transaction/{sig}", get(get_transaction_status))
        .layer(cors)
        .with_state(Arc::new(state))
}

type ApiError = (StatusCode, Json<serde_json::Value>);

fn error(code: StatusCode, msg: impl std::fmt::Display) -> ApiError {
    (code, Json(json!({ "error": msg.to_string() })))
}

fn parse_pubkey(s: &str) -> Result<Pubkey, ApiError> {
    Pubkey::from_str(s).map_err(|_| error(StatusCode::BAD_REQUEST, "invalid pubkey"))
}

fn rpc(state: &AppState) -> Result<&Arc<Rpc>, ApiError> {
    state
        .rpc
        .as_ref()
        .ok_or_else(|| error(StatusCode::SERVICE_UNAVAILABLE, "no RPC configured"))
}

fn db_err(e: impl std::fmt::Display) -> ApiError {
    error(StatusCode::INTERNAL_SERVER_ERROR, e)
}

async fn get_config(State(state): State<Arc<AppState>>) -> Result<Json<ConfigDto>, ApiError> {
    match crate::market::db::get_config(&state.client)
        .await
        .map_err(db_err)?
    {
        Some((pk, cfg, slot)) => Ok(Json(ConfigDto::new(&pk.to_bytes(), &cfg, slot))),
        None => Err(error(StatusCode::NOT_FOUND, "config not indexed")),
    }
}

async fn get_markets(State(state): State<Arc<AppState>>) -> Result<Json<Vec<MarketDto>>, ApiError> {
    let markets = crate::market::db::get_markets(&state.client)
        .await
        .map_err(db_err)?;
    Ok(Json(
        markets
            .iter()
            .map(|(pk, m, slot)| MarketDto::new(&pk.to_bytes(), m, *slot))
            .collect(),
    ))
}

async fn get_market_detail(
    State(state): State<Arc<AppState>>,
    Path(pubkey): Path<String>,
) -> Result<Json<MarketDetailDto>, ApiError> {
    let market_pk = parse_pubkey(&pubkey)?;

    let (m, slot) = crate::market::db::get_market(&state.client, &pubkey)
        .await
        .map_err(db_err)?
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "market not indexed"))?;
    let contributions: Vec<ContributionDto> =
        crate::market::db::contributions_for(&state.client, &pubkey)
            .await
            .map_err(db_err)?
            .iter()
            .map(ContributionDto::new)
            .collect();

    let oracle_pk = Pubkey::new_from_array(m.oracle.to_bytes());
    let amm_pk = Pubkey::new_from_array(m.amm.to_bytes());
    let mut detail = MarketDetailDto {
        market: MarketDto::new(&market_pk.to_bytes(), &m, slot),
        contributions,
        oracle: None,
        reserves: None,
    };

    // On-demand RPC enrichment (best-effort; absent RPC or account => None).
    if let Some(rpc) = state.rpc.as_ref() {
        if let Ok(Some(acct)) = rpc.get_account(&oracle_pk).await {
            detail.oracle = decode_oracle(&acct.data);
        }
        // Zeroed amm (pre-activation) => don't bother reading.
        if amm_pk != Pubkey::default() {
            if let Ok(Some(acct)) = rpc.get_account(&amm_pk).await {
                detail.reserves = decode_reserves(&acct.data);
            }
        }
    }
    Ok(Json(detail))
}

fn decode_oracle(data: &[u8]) -> Option<OracleDto> {
    Some(OracleDto {
        options_count: *data.get(ORACLE_OPTIONS_COUNT_OFFSET)?,
        phase: *data.get(ORACLE_PHASE_OFFSET)?,
        resolved_option: *data.get(ORACLE_RESOLVED_OPTION_OFFSET)?,
    })
}

fn decode_reserves(data: &[u8]) -> Option<ReservesDto> {
    if data.len() < AMM_QUOTE_AMOUNT_OFFSET + 8 {
        return None;
    }
    if data.get(..8) != Some(&AMM_ACCOUNT_DISCRIMINATOR[..]) {
        return None;
    }
    let base = read_u64_le(data, AMM_BASE_AMOUNT_OFFSET)?;
    let quote = read_u64_le(data, AMM_QUOTE_AMOUNT_OFFSET)?;
    Some(ReservesDto {
        base: base.to_string(),
        quote: quote.to_string(),
    })
}

fn read_u64_le(data: &[u8], offset: usize) -> Option<u64> {
    let bytes: [u8; 8] = data.get(offset..offset + 8)?.try_into().ok()?;
    Some(u64::from_le_bytes(bytes))
}

async fn get_account(
    State(state): State<Arc<AppState>>,
    Path(pubkey): Path<String>,
) -> Result<Json<AccountDto>, ApiError> {
    let pk = parse_pubkey(&pubkey)?;
    let rpc = rpc(&state)?;
    match rpc.get_account(&pk).await {
        Ok(Some(acct)) => Ok(Json(AccountDto {
            data: base64::engine::general_purpose::STANDARD.encode(&acct.data),
            owner: acct.owner.to_string(),
            lamports: acct.lamports.to_string(),
        })),
        Ok(None) => Err(error(StatusCode::NOT_FOUND, "account not found")),
        Err(e) => Err(error(StatusCode::BAD_GATEWAY, e)),
    }
}

async fn get_blockhash(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let rpc = rpc(&state)?;
    match rpc.latest_blockhash().await {
        Ok(hash) => Ok(Json(json!({ "blockhash": hash.to_string() }))),
        Err(e) => Err(error(StatusCode::BAD_GATEWAY, e)),
    }
}

#[derive(Deserialize)]
pub struct TxBody {
    pub tx: String,
}

async fn post_transaction(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TxBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let rpc = rpc(&state)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(body.tx.as_bytes())
        .map_err(|e| error(StatusCode::BAD_REQUEST, format!("invalid base64: {e}")))?;
    match rpc.send_raw_transaction(&bytes).await {
        Ok(sig) => Ok(Json(json!({ "signature": sig.to_string() }))),
        Err(e) => Err(error(StatusCode::BAD_REQUEST, e)),
    }
}

async fn get_transaction_status(
    State(state): State<Arc<AppState>>,
    Path(sig): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let rpc = rpc(&state)?;
    let signature = Signature::from_str(&sig)
        .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid signature"))?;
    match rpc.signature_status(&signature).await {
        Ok(Some(status)) => {
            let label = if status.err.is_some() {
                "failed"
            } else {
                match status.confirmation_status {
                    Some(TransactionConfirmationStatus::Finalized) => "finalized",
                    Some(TransactionConfirmationStatus::Confirmed) => "confirmed",
                    Some(TransactionConfirmationStatus::Processed) => "processed",
                    None => "processed",
                }
            };
            Ok(Json(json!({
                "status": label,
                "err": status.err.map(|e| e.to_string()),
            })))
        }
        Ok(None) => Ok(Json(json!({ "status": "pending", "err": null }))),
        Err(e) => Err(error(StatusCode::BAD_GATEWAY, e)),
    }
}

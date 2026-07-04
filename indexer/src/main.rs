//! Kassandra indexer — a Carbon pipeline over the RPC transaction crawler.
//!
//! The crawler is (re)started with `until = <durable cursor>`, so every launch —
//! including after downtime — fetches EVERY signature newer than the last
//! processed one and streams it through the decoder → Postgres processor. Inserts
//! are idempotent, and the durable cursor is only promoted forward once the
//! backfill has caught up, so nothing is ever missed.

mod api;
mod db;
mod decoder;
mod processor;
mod state;

use std::{str::FromStr, time::Duration};

use anyhow::{Context, Result};
use carbon_core::pipeline::Pipeline;
use carbon_rpc_transaction_crawler_datasource::{
    ConnectionConfig, Filters, RetryConfig, RpcTransactionCrawler,
};
use solana_commitment_config::CommitmentConfig;
use solana_signature::Signature;

use crate::api::ApiState;
use crate::decoder::{program_id, KassandraDecoder, PROGRAM_ID_STR};
use crate::processor::KassandraProcessor;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let rpc_url = std::env::var("RPC_URL")
        .or_else(|_| std::env::var("SOLANA_RPC_URL"))
        .context("RPC_URL (or SOLANA_RPC_URL) is required")?;
    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL is required")?;
    let port: u16 = env_num("PORT", 3000);
    let poll_ms: u64 = env_num("POLL_INTERVAL_MS", 10_000);
    let promote_ms: u64 = env_num("PROMOTE_INTERVAL_MS", 30_000);
    let commitment = if std::env::var("COMMITMENT").as_deref() == Ok("confirmed") {
        CommitmentConfig::confirmed()
    } else {
        CommitmentConfig::finalized()
    };

    let client = db::connect(&database_url).await?;
    let session = state::shared_session();

    // Resume point: the durable cursor becomes the crawler's `until`.
    let until_signature = match db::get_cursor(&client).await? {
        Some((sig, slot)) => {
            session.lock().await.head = Some((sig.clone(), slot));
            Signature::from_str(&sig).ok()
        }
        None => None,
    };
    log::info!(
        "[indexer] resuming from cursor: {}",
        until_signature
            .map(|s| s.to_string())
            .unwrap_or_else(|| "<none — full backfill>".into())
    );

    // Read-only API.
    {
        let state = ApiState { client: client.clone(), program_id: PROGRAM_ID_STR.to_string() };
        let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
        log::info!("[indexer] API listening on :{port}");
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, api::router(state)).await {
                log::error!("api server error: {e}");
            }
        });
    }

    // Durable-cursor promotion: advance the cursor to the run's head once the
    // backfill frontier (oldest slot processed) goes STABLE across a tick — i.e.
    // the backlog has drained and we're caught up. Never advances mid-backfill, so
    // a crash re-scans from the last cursor instead of skipping events.
    {
        let client = client.clone();
        let session = session.clone();
        let interval = Duration::from_millis(promote_ms);
        tokio::spawn(async move {
            let mut last_min = i64::MAX;
            loop {
                tokio::time::sleep(interval).await;
                let s = session.lock().await.clone();
                if state::frontier_stable(last_min, s.min_slot) {
                    if let Some((sig, slot)) = s.head {
                        match db::set_cursor(&client, &sig, slot).await {
                            Ok(()) => log::info!("[indexer] cursor promoted to slot {slot}"),
                            Err(e) => log::warn!("cursor promote failed: {e}"),
                        }
                    }
                }
                last_min = s.min_slot;
            }
        });
    }

    // The Carbon pipeline over the RPC transaction crawler.
    let conn = ConnectionConfig::new(
        1000,                           // batch_limit
        Duration::from_millis(poll_ms), // polling_interval
        10,                             // max_concurrent_requests
        RetryConfig::new(5, 500, 30_000, 2.0),
        Some(1000), // max_signature_channel_size
        Some(1000), // max_transaction_channel_size
        false,      // blocking_send
    );
    let crawler = RpcTransactionCrawler::new(
        rpc_url,
        program_id(),
        conn,
        Filters::new(None, None, until_signature),
        Some(commitment),
    );

    let mut pipeline = Pipeline::builder()
        .datasource(crawler)
        .instruction(
            KassandraDecoder { program_id: program_id() },
            KassandraProcessor { client: client.clone(), session: session.clone() },
        )
        .build()?;

    log::info!("[indexer] program {PROGRAM_ID_STR}; crawler starting");
    tokio::select! {
        r = pipeline.run() => { r.map_err(|e| anyhow::anyhow!("pipeline: {e}"))?; }
        _ = tokio::signal::ctrl_c() => { log::info!("[indexer] SIGINT — shutting down"); }
    }
    Ok(())
}

fn env_num<T: FromStr>(key: &str, default: T) -> T {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

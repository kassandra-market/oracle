//! Kassandra indexer — one binary, two Carbon pipelines, one Postgres, one axum.
//!
//! * **Oracle side** crawls transactions (RPC transaction crawler → instruction
//!   decoder → `events` table). The crawler is (re)started with `until =
//!   <durable cursor>`, so every launch re-fetches EVERY signature newer than the
//!   last processed one; inserts are idempotent and the cursor only advances once
//!   the backfill has caught up, so nothing is missed.
//! * **Market side** indexes the kassandra-market program's *accounts* (gpa
//!   snapshot + optional program-subscribe live tail → `market_accounts` table),
//!   with a periodic getProgramAccounts reconcile that also prunes closed
//!   accounts. Serves `/api/*` alongside the oracle routes on the same server.

mod api;
mod db;
mod decoder;
mod market;
mod meta_fetch;
mod oracle_accounts;
mod processor;
mod state;

use std::collections::HashSet;
use std::sync::Arc;
use std::{str::FromStr, time::Duration};

use anyhow::{Context, Result};
use carbon_core::account::AccountDecoder;
use carbon_core::pipeline::Pipeline;
use carbon_rpc_gpa_datasource::GpaDatasource;
use carbon_rpc_program_subscribe_datasource::{Filters as SubscribeFilters, RpcProgramSubscribe};
use carbon_rpc_transaction_crawler_datasource::{
    ConnectionConfig, Filters, RetryConfig, RpcTransactionCrawler,
};
use solana_commitment_config::CommitmentConfig;
use solana_pubkey::Pubkey;
use solana_signature::Signature;

use crate::api::ApiState;
use crate::decoder::{program_id, program_id_str, KassandraDecoder};
use crate::market::rpc::Rpc as MarketRpc;
use crate::processor::KassandraProcessor;

/// Subscribe-mode snapshot cadence (ms): the ws tail handles freshness, so this
/// slower getProgramAccounts pass only needs to prune accounts closed on-chain.
const MARKET_PRUNE_INTERVAL_MS: u64 = 60_000;

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
    let meta_fetch_ms: u64 = env_num("METADATA_FETCH_INTERVAL_MS", 30_000);
    let commitment = if std::env::var("COMMITMENT").as_deref() == Ok("confirmed") {
        CommitmentConfig::confirmed()
    } else {
        CommitmentConfig::finalized()
    };

    let client = db::connect(&database_url).await?;
    market::db::create_schema(&client).await?;
    oracle_accounts::create_schema(&client).await?;
    let session = state::shared_session();

    // Market side config.
    let market_program_id = match std::env::var("MARKET_PROGRAM_ID") {
        Ok(s) => Pubkey::from_str(&s).context("invalid MARKET_PROGRAM_ID")?,
        Err(_) => market::default_program_id(),
    };
    // >0 => run the periodic getProgramAccounts reconcile as the freshness path
    // (for RPCs without a working ws `programSubscribe`, e.g. surfpool in the e2e).
    let market_reconcile_ms: u64 = env_num("INDEXER_RECONCILE_MS", 0);
    let market_rpc = Arc::new(MarketRpc::new(rpc_url.clone()));

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

    // One axum server for BOTH sides: the oracle read API + `/rpc` gateway, merged
    // with the market `/api/*` data + tx gateway. Route namespaces don't collide
    // (`/status`,`/events`,`/accounts`,`/rpc` vs `/api/*`); `/health` lives on the
    // oracle router only.
    {
        let oracle_state = ApiState {
            client: client.clone(),
            program_id: program_id_str(),
            rpc_url: rpc_url.clone(),
            http: reqwest::Client::new(),
        };
        let market_state = market::api::AppState {
            client: client.clone(),
            rpc: Some(market_rpc.clone()),
        };
        let app = api::router(oracle_state).merge(market::api::router(market_state));
        let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
        log::info!("[indexer] API listening on :{port}");
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                log::error!("api server error: {e}");
            }
        });
    }

    // Market account pipeline: gpa snapshot always; the live program-subscribe tail
    // only when NOT reconciling and a ws url is set (reconcile mode replaces the ws
    // tail with polling). The pipeline is non-fatal — if it dies, the reconcile loop
    // keeps `market_accounts` correct (degraded freshness in subscribe mode).
    {
        let gpa = GpaDatasource::new(rpc_url.clone(), market_program_id);
        let mut builder = Pipeline::builder().datasource(gpa);
        if market_reconcile_ms == 0 {
            match std::env::var("SOLANA_WS_URL") {
                Ok(ws_url) => {
                    builder = builder.datasource(RpcProgramSubscribe::new(
                        ws_url,
                        SubscribeFilters::new(market_program_id, None),
                    ));
                }
                Err(_) => log::warn!(
                    "[market] no SOLANA_WS_URL and INDEXER_RECONCILE_MS=0; gpa snapshot + prune only"
                ),
            }
        }
        let mut market_pipeline = builder
            .account(
                market::decoder::KassandraAccountDecoder {
                    program_id: market_program_id,
                },
                market::processor::KassandraAccountProcessor {
                    client: client.clone(),
                },
            )
            .build()?;
        let reconcile_interval = if market_reconcile_ms > 0 {
            market_reconcile_ms
        } else {
            MARKET_PRUNE_INTERVAL_MS
        };
        log::info!(
            "[market] program {market_program_id}; reconcile_ms={market_reconcile_ms}; pipeline starting"
        );
        tokio::spawn(async move {
            if let Err(e) = market_pipeline.run().await {
                log::warn!("[market] pipeline exited (reconcile keeps accounts fresh): {e}");
            }
        });
        tokio::spawn(market_reconcile_loop(
            market_rpc.clone(),
            client.clone(),
            market_program_id,
            reconcile_interval,
        ));
    }

    // Oracle ACCOUNT pipeline (distinct from the tx crawler below): a gpa snapshot
    // on startup + a live programSubscribe tail mirror every Oracle + child account
    // into `oracle_accounts`, so the app reads oracle lists/detail from Postgres
    // instead of slow client-side getProgramAccounts. A periodic gpa reconcile is
    // the only path that prunes accounts closed on-chain (the tail can't see a close).
    {
        let oracle_program = program_id();
        let gpa = GpaDatasource::new(rpc_url.clone(), oracle_program);
        let mut builder = Pipeline::builder().datasource(gpa);
        match std::env::var("SOLANA_WS_URL") {
            Ok(ws_url) => {
                builder = builder.datasource(RpcProgramSubscribe::new(
                    ws_url,
                    SubscribeFilters::new(oracle_program, None),
                ));
            }
            Err(_) => log::warn!(
                "[oracle-acct] no SOLANA_WS_URL; gpa snapshot + periodic reconcile only (no live tail)"
            ),
        }
        let mut oracle_acct_pipeline = builder
            .account(
                oracle_accounts::OracleAccountDecoder {
                    program_id: oracle_program,
                },
                oracle_accounts::OracleAccountProcessor {
                    client: client.clone(),
                },
            )
            .build()?;
        log::info!("[oracle-acct] account pipeline starting for {oracle_program}");
        tokio::spawn(async move {
            if let Err(e) = oracle_acct_pipeline.run().await {
                log::warn!("[oracle-acct] pipeline exited (reconcile keeps accounts fresh): {e}");
            }
        });
        tokio::spawn(oracle_accounts::reconcile_loop(
            market_rpc.clone(),
            client.clone(),
            oracle_program,
            MARKET_PRUNE_INTERVAL_MS,
        ));
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

    // Autonomous metadata fetcher: pull in externally-hosted oracle-metadata JSON
    // (any host), verify sha256 == the on-chain uri_hash, and store it — so served
    // metadata is host-agnostic, not limited to what the app POSTs to us.
    {
        let client = client.clone();
        tokio::spawn(meta_fetch::reconcile_loop(client, meta_fetch_ms));
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
            KassandraDecoder {
                program_id: program_id(),
            },
            KassandraProcessor {
                client: client.clone(),
                session: session.clone(),
            },
        )
        .build()?;

    log::info!("[indexer] program {}; crawler starting", program_id_str());
    tokio::select! {
        r = pipeline.run() => { r.map_err(|e| anyhow::anyhow!("pipeline: {e}"))?; }
        _ = tokio::signal::ctrl_c() => { log::info!("[indexer] SIGINT — shutting down"); }
    }
    Ok(())
}

fn env_num<T: FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// Periodically re-snapshot the market program's accounts (getProgramAccounts),
/// upsert them into `market_accounts`, and prune those closed on-chain. Runs in
/// BOTH modes: the freshness path in reconcile mode (no ws), and a slower
/// close-pruning pass in subscribe mode (the ws tail never observes a close).
async fn market_reconcile_loop(
    rpc: Arc<MarketRpc>,
    client: Arc<tokio_postgres::Client>,
    program_id: Pubkey,
    interval_ms: u64,
) {
    let decoder = market::decoder::KassandraAccountDecoder { program_id };
    loop {
        tokio::time::sleep(Duration::from_millis(interval_ms)).await;
        match market_reconcile_once(&rpc, &client, &decoder).await {
            Ok(n) => log::debug!("[market] reconcile: {n} accounts"),
            Err(e) => log::warn!("[market] reconcile failed: {e}"),
        }
    }
}

async fn market_reconcile_once(
    rpc: &MarketRpc,
    client: &tokio_postgres::Client,
    decoder: &market::decoder::KassandraAccountDecoder,
) -> Result<usize> {
    let slot = rpc.get_slot().await? as i64;
    let accounts = rpc.get_program_accounts(&decoder.program_id).await?;
    let mut present: HashSet<String> = HashSet::new();
    let mut n = 0;
    for (pubkey, account) in accounts {
        if let Some(decoded) = decoder.decode_account(&account) {
            let key = pubkey.to_string();
            market::processor::persist(client, &key, &decoded.data, account.data.as_slice(), slot)
                .await;
            present.insert(key);
            n += 1;
        }
    }
    // The ONLY path that removes closed accounts (the subscribe tail can't observe
    // a close). Slot-aware, so a just-created account ahead of this snapshot isn't
    // wrongly dropped.
    market::db::prune(client, slot, &present).await?;
    Ok(n)
}

//! Per-market AMM price subscription → the `market_price` time-series.
//!
//! Instead of polling, we open a websocket `accountSubscribe` on each ACTIVE
//! market's cYES/cNO pool. Every on-chain change to a pool (i.e. a swap) pushes a
//! notification carrying the fresh reserves; we decode them, compute the implied
//! YES probability, and append one `(market, slot, ts, base, quote, price)` row.
//! This gives intra-block granularity — a price point per swap, stamped with the
//! wall-clock receipt time — where a periodic sample gave only one per interval.
//!
//! A manager task periodically reconciles the live subscription set against the
//! DB's Active markets: a newly-activated pool gets a task; a pool whose market
//! left Active (resolved / closed) has its task aborted (dropping the ws sub).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use solana_account_decoder_client_types::UiAccountEncoding;
use solana_commitment_config::CommitmentConfig;
use solana_pubkey::Pubkey;
use solana_pubsub_client::nonblocking::pubsub_client::PubsubClient;
use solana_rpc_client_types::config::RpcAccountInfoConfig;
use tokio::task::JoinHandle;
use tokio_postgres::Client;

use crate::market::rpc::Rpc as MarketRpc;
use crate::market::{decode_amm_reserves, implied_yes_probability};

/// How often the manager re-reads the DB to (un)subscribe pools as markets are
/// activated / resolved. Subscriptions are event-driven; this only governs how
/// fast a *newly-activated* market gets wired up (not price freshness).
const REFRESH_INTERVAL_MS: u64 = 10_000;

/// Delay before a dropped subscription task reconnects its websocket.
const RECONNECT_DELAY_MS: u64 = 3_000;

/// Wall-clock unix seconds — the capture timestamp of a price sample. Updates
/// arrive in near-real-time, so server time ≈ block time; using the receipt clock
/// (not an extra `getBlockTime` RPC) keeps each notification a single cheap write.
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Record one price point from decoded reserves at `slot`. A zero-reserve (empty)
/// pool has an undefined probability and is skipped.
async fn record(client: &Client, market: &str, slot: i64, base: u64, quote: u64) {
    let Some(price) = implied_yes_probability(base, quote) else {
        return;
    };
    if let Err(e) = crate::market::db::insert_price(
        client,
        market,
        slot,
        unix_now(),
        base as i64,
        quote as i64,
        price,
    )
    .await
    {
        log::warn!("[market-price] insert {market} failed: {e}");
    }
}

/// Subscribe to one AMM pool and stream its reserves into `market_price` until the
/// task is aborted. Reconnects on a dropped stream. A baseline read at subscribe
/// time seeds an initial point (accountSubscribe pushes only on *change*, so a
/// just-activated pool would otherwise have no candle until its first swap).
async fn subscribe_amm(
    ws_url: String,
    rpc: Arc<MarketRpc>,
    client: Arc<Client>,
    market: String,
    amm: Pubkey,
) {
    let config = RpcAccountInfoConfig {
        encoding: Some(UiAccountEncoding::Base64),
        commitment: Some(CommitmentConfig::confirmed()),
        data_slice: None,
        min_context_slot: None,
    };

    loop {
        // Baseline: seed the current reserves so the chart has a point immediately.
        if let Ok(Some(acct)) = rpc.get_account(&amm).await {
            if let Some((base, quote)) = decode_amm_reserves(&acct.data) {
                let slot = rpc.get_slot().await.unwrap_or(0) as i64;
                record(&client, &market, slot, base, quote).await;
            }
        }

        let pubsub = match PubsubClient::new(&ws_url).await {
            Ok(c) => c,
            Err(e) => {
                log::warn!("[market-price] {market}: ws connect failed: {e}");
                tokio::time::sleep(Duration::from_millis(RECONNECT_DELAY_MS)).await;
                continue;
            }
        };
        let (mut stream, _unsub) = match pubsub.account_subscribe(&amm, Some(config.clone())).await
        {
            Ok(s) => s,
            Err(e) => {
                log::warn!("[market-price] {market}: account_subscribe failed: {e}");
                tokio::time::sleep(Duration::from_millis(RECONNECT_DELAY_MS)).await;
                continue;
            }
        };
        log::info!("[market-price] subscribed pool {amm} for market {market}");

        while let Some(update) = stream.next().await {
            let slot = update.context.slot as i64;
            let Some(bytes) = update.value.data.decode() else {
                continue;
            };
            if let Some((base, quote)) = decode_amm_reserves(&bytes) {
                record(&client, &market, slot, base, quote).await;
            }
        }

        log::warn!("[market-price] {market}: stream closed, reconnecting");
        tokio::time::sleep(Duration::from_millis(RECONNECT_DELAY_MS)).await;
    }
}

/// The subscription manager: reconciles the live per-pool subscription set against
/// the DB's Active markets every {@link REFRESH_INTERVAL_MS}. Runs forever.
pub async fn run_price_subscriber(ws_url: String, rpc: Arc<MarketRpc>, client: Arc<Client>) {
    // market pubkey (base58) → its running subscription task.
    let mut live: HashMap<String, JoinHandle<()>> = HashMap::new();

    log::info!("[market-price] subscriber started (ws {ws_url})");
    loop {
        match crate::market::db::active_market_amms(&client).await {
            Ok(active) => {
                let want: HashMap<String, Pubkey> = active.into_iter().collect();

                // Drop tasks for markets no longer Active (or whose task died).
                live.retain(|market, handle| {
                    let keep = want.contains_key(market) && !handle.is_finished();
                    if !keep {
                        handle.abort();
                        log::info!("[market-price] unsubscribed market {market}");
                    }
                    keep
                });

                // Spawn a task for each newly-Active pool.
                for (market, amm) in want {
                    if live.contains_key(&market) {
                        continue;
                    }
                    let handle = tokio::spawn(subscribe_amm(
                        ws_url.clone(),
                        rpc.clone(),
                        client.clone(),
                        market.clone(),
                        amm,
                    ));
                    live.insert(market, handle);
                }
            }
            Err(e) => log::warn!("[market-price] active-market scan failed: {e}"),
        }
        tokio::time::sleep(Duration::from_millis(REFRESH_INTERVAL_MS)).await;
    }
}

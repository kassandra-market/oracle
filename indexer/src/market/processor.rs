//! The Carbon account processor: slot-gated upsert of each decoded market account
//! into Postgres (`market_accounts`), keyed by `metadata.pubkey` / `metadata.slot`.
//!
//! The raw account bytes (`raw_account.data`) are what we persist — reads decode
//! them back via `db`. For a `Contribution` we also stamp its `market` field
//! (base58) into `market_ref` so `contributions_for` is an indexed lookup.

use std::sync::Arc;

use carbon_core::account::AccountProcessorInputType;
use carbon_core::error::CarbonResult;
use carbon_core::processor::Processor;
use tokio_postgres::Client;

use crate::market::db;
use crate::market::decoder::KassandraAccount;

pub struct KassandraAccountProcessor {
    pub client: Arc<Client>,
}

impl Processor<AccountProcessorInputType<'_, KassandraAccount>> for KassandraAccountProcessor {
    async fn process(
        &mut self,
        input: &AccountProcessorInputType<'_, KassandraAccount>,
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

/// Slot-gated upsert of one decoded account + its raw bytes into `market_accounts`.
/// Shared by the live pipeline (above) and the reconcile loop (`main`). Errors are
/// logged, not propagated, so one bad row never kills the pipeline.
pub async fn persist(
    client: &Client,
    pubkey: &str,
    decoded: &KassandraAccount,
    data: &[u8],
    slot: i64,
) {
    let (account_type, market_ref) = match decoded {
        KassandraAccount::Config(_) => (db::TYPE_CONFIG, None),
        KassandraAccount::Market(_) => (db::TYPE_MARKET, None),
        KassandraAccount::Contribution(c) => (
            db::TYPE_CONTRIBUTION,
            Some(bs58::encode(c.market).into_string()),
        ),
    };
    if let Err(e) = db::upsert_account(
        client,
        pubkey,
        account_type,
        market_ref.as_deref(),
        slot,
        data,
    )
    .await
    {
        log::warn!("[market] upsert {pubkey} failed: {e}");
    }
}

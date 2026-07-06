//! On-demand RPC access for the tx gateway + foreign-account reads (oracle, AMM
//! reserves, ATA existence/balances). A thin wrapper over the nonblocking
//! `solana-rpc-client` so the API handlers stay small.

use anyhow::{Context, Result};
use solana_account::Account;
use solana_commitment_config::CommitmentConfig;
use solana_hash::Hash;
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;
use solana_transaction_status::TransactionStatus;

pub struct Rpc {
    client: RpcClient,
}

impl Rpc {
    pub fn new(http_url: String) -> Self {
        // `confirmed` (not the RPC default `finalized`) so freshly-landed writes
        // — the reconcile snapshot, on-demand reads, blockhash — reflect the same
        // state the client just confirmed (surfpool finalizes lazily).
        Self {
            client: RpcClient::new_with_commitment(http_url, CommitmentConfig::confirmed()),
        }
    }

    /// Every account owned by `program` (the periodic reconcile snapshot). Used by
    /// the reconcile loop to re-index the program's accounts when the live
    /// program-subscribe tail is unavailable (e.g. surfpool).
    pub async fn get_program_accounts(&self, program: &Pubkey) -> Result<Vec<(Pubkey, Account)>> {
        self.client
            .get_program_accounts(program)
            .await
            .context("get_program_accounts")
    }

    /// The node's current slot (stamped onto reconcile upserts so they always
    /// win the store's slot-gate).
    pub async fn get_slot(&self) -> Result<u64> {
        self.client.get_slot().await.context("get_slot")
    }

    /// On-demand account read; `Ok(None)` for a nonexistent account.
    pub async fn get_account(&self, pubkey: &Pubkey) -> Result<Option<Account>> {
        let resp = self
            .client
            .get_account_with_commitment(pubkey, self.client.commitment())
            .await
            .context("get_account")?;
        Ok(resp.value)
    }

    /// A recent blockhash for the client to stamp onto its transaction.
    pub async fn latest_blockhash(&self) -> Result<Hash> {
        self.client
            .get_latest_blockhash()
            .await
            .context("get_latest_blockhash")
    }

    /// Relay an already-signed, serialized transaction (legacy or versioned wire
    /// format, as produced by `@solana/web3.js` `tx.serialize()`).
    pub async fn send_raw_transaction(&self, bytes: &[u8]) -> Result<Signature> {
        let tx: VersionedTransaction =
            bincode::deserialize(bytes).context("deserialize transaction")?;
        self.client
            .send_transaction(&tx)
            .await
            .context("send_transaction")
    }

    /// Status of a submitted signature, if the network has seen it.
    ///
    /// Uses the `_with_history` variant (searchTransactionHistory=true) so a tx
    /// that already confirmed/finalized still reports its true status when polled
    /// after the ~150-slot status cache has evicted it — the bare
    /// `get_signature_statuses` returns `None` there, which the gateway would
    /// mislabel as `"pending"` for a long-since-confirmed tx.
    pub async fn signature_status(&self, sig: &Signature) -> Result<Option<TransactionStatus>> {
        let resp = self
            .client
            .get_signature_statuses_with_history(&[*sig])
            .await
            .context("get_signature_statuses_with_history")?;
        Ok(resp.value.into_iter().next().flatten())
    }
}

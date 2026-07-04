//! The Carbon processor: persist one event per decoded Kassandra instruction and
//! track the newest processed signature (for the durable-cursor promotion task).

use std::sync::Arc;

use base64::Engine as _;
use carbon_core::error::{CarbonResult, Error};
use carbon_core::instruction::InstructionProcessorInputType;
use carbon_core::processor::Processor;
use tokio_postgres::Client;

use crate::db::{self, Event};
use crate::decoder::KassandraIx;
use crate::state::SharedSession;

pub struct KassandraProcessor {
    pub client: Arc<Client>,
    pub session: SharedSession,
}

impl<'a> Processor<InstructionProcessorInputType<'a, KassandraIx>> for KassandraProcessor {
    async fn process(
        &mut self,
        data: &InstructionProcessorInputType<'a, KassandraIx>,
    ) -> CarbonResult<()> {
        let tx = &data.metadata.transaction_metadata;
        let ix = data.decoded_instruction;
        let signature = tx.signature.to_string();
        let slot = tx.slot as i64;

        let event = Event {
            signature: signature.clone(),
            ix_index: data.metadata.index as i32,
            ix_type: ix.name.to_string(),
            discriminant: ix.discriminant as i16,
            slot,
            block_time: tx.block_time,
            account0: ix.accounts.first().cloned(),
            accounts_json: serde_json::to_string(&ix.accounts).unwrap_or_else(|_| "[]".to_string()),
            data_base64: base64::engine::general_purpose::STANDARD.encode(&ix.data),
        };

        db::insert_event(&self.client, &event)
            .await
            .map_err(|e| Error::Custom(format!("insert {signature}: {e}")))?;

        // Track the run's head (newest) + frontier (oldest) slots. The promotion
        // task advances the durable cursor only once the frontier goes stable
        // (backlog drained), so a crash mid-backfill re-scans from the last
        // cursor rather than skipping the un-backfilled range.
        let mut s = self.session.lock().await;
        if s.head.as_ref().map_or(true, |(_, hs)| slot > *hs) {
            s.head = Some((signature, slot));
        }
        if slot < s.min_slot {
            s.min_slot = slot;
        }
        Ok(())
    }
}

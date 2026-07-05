//! The Carbon processor: persist one event per decoded Kassandra instruction and
//! track the newest processed signature (for the durable-cursor promotion task).

use std::str::FromStr;
use std::sync::Arc;

use base64::Engine as _;
use carbon_core::error::{CarbonResult, Error};
use carbon_core::instruction::InstructionProcessorInputType;
use carbon_core::processor::Processor;
use solana_message::VersionedMessage;
use solana_pubkey::Pubkey;
use tokio_postgres::Client;

use crate::db::{self, Event};
use crate::decoder::KassandraIx;
use crate::state::SharedSession;

/// The SPL Memo program — the CreateOracle tx carries the plaintext subject +
/// option labels as a memo (the chain stores only the prompt hash).
const MEMO_PROGRAM_ID: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";

/// Find the first SPL Memo instruction's UTF-8 payload in a transaction message.
fn extract_memo(message: &VersionedMessage, memo_program: &Pubkey) -> Option<String> {
    let keys = message.static_account_keys();
    for ci in message.instructions() {
        if keys.get(ci.program_id_index as usize) == Some(memo_program) {
            if let Ok(s) = String::from_utf8(ci.data.clone()) {
                return Some(s);
            }
        }
    }
    None
}

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
            ix_type: ix.name.clone(),
            discriminant: ix.discriminant as i16,
            slot,
            block_time: tx.block_time,
            account0: ix.accounts.first().cloned(),
            accounts: serde_json::Value::from(ix.accounts.clone()),
            data_base64: base64::engine::general_purpose::STANDARD.encode(&ix.data),
        };

        db::insert_event(&self.client, &event)
            .await
            .map_err(|e| Error::Custom(format!("insert {signature}: {e}")))?;

        // On CreateOracle, capture the plaintext SUBJECT + option labels from the
        // tx's SPL Memo (the chain stores only the prompt hash). accounts[1] is the
        // oracle PDA (see the SDK createOracle account order). Best-effort: a
        // missing/garbled memo just leaves the oracle without indexed metadata.
        if ix.name == "create_oracle" {
            if let Some(oracle) = ix.accounts.get(1) {
                let memo_program = Pubkey::from_str(MEMO_PROGRAM_ID).expect("valid memo id");
                if let Some(memo) = extract_memo(&tx.message, &memo_program) {
                    if let Ok(meta) = serde_json::from_str::<serde_json::Value>(&memo) {
                        let subject = meta.get("subject").and_then(|v| v.as_str());
                        let options = meta.get("options");
                        if let (Some(subject), Some(options)) = (subject, options) {
                            if !subject.is_empty() && options.is_array() {
                                if let Err(e) = db::insert_oracle_meta(
                                    &self.client,
                                    oracle,
                                    subject,
                                    options,
                                    slot,
                                    &signature,
                                )
                                .await
                                {
                                    log::warn!("[indexer] oracle meta insert {oracle}: {e}");
                                }
                            }
                        }
                    }
                }
            }
        }

        // Track the run's head (newest) + frontier (oldest) slots. The promotion
        // task advances the durable cursor only once the frontier goes stable
        // (backlog drained), so a crash mid-backfill re-scans from the last
        // cursor rather than skipping the un-backfilled range.
        let mut s = self.session.lock().await;
        if s.head.as_ref().is_none_or(|(_, hs)| slot > *hs) {
            s.head = Some((signature, slot));
        }
        if slot < s.min_slot {
            s.min_slot = slot;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_instruction::Instruction;
    use solana_message::{Message, VersionedMessage};

    fn memo_pid() -> Pubkey {
        Pubkey::from_str(MEMO_PROGRAM_ID).unwrap()
    }

    #[test]
    fn extract_memo_finds_the_payload() {
        let payload = r#"{"v":1,"subject":"Q?","options":["A","B"]}"#;
        let msg = Message::new(
            &[
                Instruction::new_with_bytes(Pubkey::new_unique(), &[1, 2, 3], vec![]),
                Instruction::new_with_bytes(memo_pid(), payload.as_bytes(), vec![]),
            ],
            None,
        );
        let vmsg = VersionedMessage::Legacy(msg);
        assert_eq!(extract_memo(&vmsg, &memo_pid()).as_deref(), Some(payload));
    }

    #[test]
    fn extract_memo_none_when_absent() {
        let msg = Message::new(
            &[Instruction::new_with_bytes(Pubkey::new_unique(), &[1], vec![])],
            None,
        );
        let vmsg = VersionedMessage::Legacy(msg);
        assert!(extract_memo(&vmsg, &memo_pid()).is_none());
    }
}

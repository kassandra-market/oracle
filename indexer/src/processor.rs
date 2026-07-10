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

/// Parse a `write_oracle_meta` instruction's data (discriminant @0, then the
/// length-prefixed body) → `(subject, options JSON array, uri, uri_hash hex)`.
/// `None` on any malformed input.
fn parse_write_oracle_meta(data: &[u8]) -> Option<(String, serde_json::Value, String, String)> {
    let mut off = 1usize; // skip the discriminant
    let read_u16 = |d: &[u8], off: &mut usize| -> Option<usize> {
        let b = d.get(*off..*off + 2)?;
        *off += 2;
        Some(u16::from_le_bytes(b.try_into().unwrap()) as usize)
    };
    let read_str = |d: &[u8], off: &mut usize, len: usize| -> Option<String> {
        let s = d.get(*off..*off + len)?;
        *off += len;
        String::from_utf8(s.to_vec()).ok()
    };

    let subject_len = read_u16(data, &mut off)?;
    let subject = read_str(data, &mut off, subject_len)?;
    let options_count = *data.get(off)?;
    off += 1;
    let mut options = Vec::with_capacity(options_count as usize);
    for _ in 0..options_count {
        let ol = read_u16(data, &mut off)?;
        options.push(serde_json::Value::String(read_str(data, &mut off, ol)?));
    }
    let uri_len = read_u16(data, &mut off)?;
    let uri = read_str(data, &mut off, uri_len)?;
    let uri_hash = data.get(off..off + 32)?;
    let uri_hash_hex = hex::encode(uri_hash);

    Some((
        subject,
        serde_json::Value::Array(options),
        uri,
        uri_hash_hex,
    ))
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

        // Index oracle metadata from `write_oracle_meta` (the on-chain account is
        // authoritative; this is a queryable mirror). accounts[1] is the oracle
        // (see the write_oracle_meta account order). Best-effort: a garbled ix just
        // leaves the oracle without indexed metadata.
        if ix.name == "write_oracle_meta" {
            if let Some(oracle) = ix.accounts.get(1) {
                if let Some((subject, options, uri, uri_hash)) = parse_write_oracle_meta(&ix.data) {
                    if let Err(e) = db::insert_oracle_meta(
                        &self.client,
                        oracle,
                        &subject,
                        &options,
                        &uri,
                        &uri_hash,
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

    /// Hand-encode a `write_oracle_meta` ix data (disc + length-prefixed body).
    fn encode(subject: &str, options: &[&str], uri: &str, uri_hash: [u8; 32]) -> Vec<u8> {
        let mut d = vec![kassandra_oracles_sdk::Ix::WriteOracleMeta as u8];
        d.extend_from_slice(&(subject.len() as u16).to_le_bytes());
        d.extend_from_slice(subject.as_bytes());
        d.push(options.len() as u8);
        for o in options {
            d.extend_from_slice(&(o.len() as u16).to_le_bytes());
            d.extend_from_slice(o.as_bytes());
        }
        d.extend_from_slice(&(uri.len() as u16).to_le_bytes());
        d.extend_from_slice(uri.as_bytes());
        d.extend_from_slice(&uri_hash);
        d
    }

    #[test]
    fn parse_write_oracle_meta_round_trips() {
        let data = encode("Who wins?", &["Yes", "No"], "https://x/m.json", [0xab; 32]);
        let (subject, options, uri, uri_hash) = parse_write_oracle_meta(&data).unwrap();
        assert_eq!(subject, "Who wins?");
        assert_eq!(options, serde_json::json!(["Yes", "No"]));
        assert_eq!(uri, "https://x/m.json");
        assert_eq!(uri_hash, "ab".repeat(32));
    }

    #[test]
    fn parse_write_oracle_meta_none_on_truncated() {
        let data = encode("Q?", &["A", "B"], "u", [1u8; 32]);
        assert!(parse_write_oracle_meta(&data[..10]).is_none());
    }
}

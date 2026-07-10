//! Build + sign + send + confirm the `submit_ai_claim` transaction (Task RS1).
//!
//! Turns the runner into a self-contained keeper: given the 97-byte
//! `submit_ai_claim` payload the runner ALREADY computes ([`crate::hashing`]),
//! the oracle/proposer pubkeys, and the proposer-authority keypair, this module
//!
//! 1. builds the `submit_ai_claim` [`Instruction`] (exact processor account
//!    order + `data = [Ix::SubmitAiClaim] ++ payload`),
//! 2. wraps it in a legacy [`Message`] (payer = the authority) with a fetched
//!    recent blockhash and SIGNS it (ed25519) with the loaded keypair,
//! 3. serializes it (bincode → base64) and SENDS it over the same I3
//!    [`JsonRpc`] transport used for the on-chain fetch, then
//! 4. CONFIRMS it by polling `getSignatureStatuses`, surfacing a failed tx /
//!    program error (e.g. an already-submitted claim, or a wrong-phase reject)
//!    as a clear [`SubmitError`].
//!
//! # Why the split solana-* crates
//!
//! The legacy-message compaction (compact-u16 account/instruction encoding) and
//! ed25519 signing are subtle; we use the granular `solana-message` /
//! `solana-transaction` / `solana-keypair` crates for a correct, canonical
//! serialization rather than hand-rolling it — but deliberately NOT
//! `solana-client` (send/confirm rides the existing reqwest [`JsonRpc`]) nor the
//! full `solana-sdk`. These are host-only deps; the on-chain (pinocchio) program
//! is untouched.
//!
//! # Payload provenance
//!
//! The transaction carries the SAME 97 payload bytes the runner emits in its
//! `RunOutput` (`model_id[32] ++ params_hash[32] ++ io_hash[32] ++ option[1]`);
//! it is passed in, never recomputed here, so the submitted claim and the
//! emitted metadata can never diverge.

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use serde_json::{json, Value};
use solana_hash::Hash;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::Transaction;

use crate::constants::SUBMIT_AI_CLAIM_PAYLOAD_LEN;
use crate::rpc::{JsonRpc, RpcError};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

/// The `submit_ai_claim` instruction discriminant (first `data` byte), tied to
/// the SDK's [`kassandra_oracles_sdk::Ix::SubmitAiClaim`] variant (re-exported from the
/// program) so a renumber in the program breaks this build.
pub const SUBMIT_AI_CLAIM_DISCRIMINANT: u8 = kassandra_oracles_sdk::Ix::SubmitAiClaim as u8;

/// Anything that can go wrong building/sending/confirming the claim tx.
#[derive(Debug, thiserror::Error)]
pub enum SubmitError {
    /// The `--keypair` file could not be read.
    #[error("failed to read keypair file `{path}`: {message}")]
    KeypairRead {
        /// The keypair path.
        path: String,
        /// The rendered IO error.
        message: String,
    },
    /// The `--keypair` file was not a valid Solana CLI keypair (a 64-byte JSON
    /// array of a ed25519 keypair).
    #[error("keypair file `{path}` is malformed: {message}")]
    KeypairMalformed {
        /// The keypair path.
        path: String,
        /// What was wrong.
        message: String,
    },
    /// A transport / JSON-RPC error (including a `sendTransaction` PREFLIGHT
    /// failure, which the RPC returns as a JSON-RPC error carrying the program
    /// error — e.g. an already-submitted claim or a wrong-phase reject).
    #[error(transparent)]
    Rpc(#[from] RpcError),
    /// An RPC response did not have the expected shape.
    #[error("malformed `{method}` response: {detail}")]
    Malformed {
        /// The RPC method.
        method: String,
        /// What was wrong.
        detail: String,
    },
    /// The transaction was landed but FAILED on chain (its status carried an
    /// `err`) — e.g. a program error that slipped past preflight.
    #[error("transaction `{signature}` failed on chain: {error}")]
    TxFailed {
        /// The transaction signature (base58).
        signature: String,
        /// The on-chain error (the JSON `err` object, rendered).
        error: String,
    },
    /// The transaction was sent but did not reach the target commitment within
    /// the poll budget.
    #[error("transaction `{signature}` not confirmed after {polls} polls (~{seconds}s)")]
    ConfirmTimeout {
        /// The transaction signature (base58).
        signature: String,
        /// How many status polls were made.
        polls: u32,
        /// The elapsed wall-clock budget, seconds.
        seconds: u64,
    },
}

/// The Kassandra program id as a [`Pubkey`] (from the SDK's canonical constant).
pub fn program_id() -> Pubkey {
    kassandra_oracles_sdk::PROGRAM_ID
}

/// Derive the `ai_claim` PDA (seeds `[b"claim", oracle, proposer]`) via the SDK.
pub fn derive_ai_claim_pda(oracle: &Pubkey, proposer: &Pubkey) -> Pubkey {
    kassandra_oracles_sdk::pda::ai_claim(&program_id(), oracle, proposer).0
}

/// Derive the `proposer` PDA `find_program_address([b"proposer", oracle,
/// authority], kassandra_id)` (the on-chain `propose` contract).
///
/// The keeper is run BY the proposer, so its Proposer PDA is fully determined by
/// the oracle and the signing `authority` (the `--keypair` pubkey) — no separate
/// `--proposer` argument is required.
pub fn derive_proposer_pda(oracle: &Pubkey, authority: &Pubkey) -> Pubkey {
    kassandra_oracles_sdk::pda::proposer(&program_id(), oracle, authority).0
}

/// Build the `submit_ai_claim` [`Instruction`].
///
/// Account metas are in the EXACT processor order
/// (`submit_ai_claim.rs`): `[oracle(w), proposer PDA(w), ai_claim PDA(w),
/// authority(signer,w), system(ro)]`. The `ai_claim` PDA is derived here from
/// `[b"claim", oracle, proposer]`. `data = [SubmitAiClaim disc] ++ payload`
/// (the 97-byte payload the runner already computed — asserted 97 bytes).
pub fn build_submit_ai_claim_ix(
    oracle: &Pubkey,
    proposer: &Pubkey,
    authority: &Pubkey,
    payload: &[u8; SUBMIT_AI_CLAIM_PAYLOAD_LEN],
) -> Instruction {
    // Layout is compile-time-pinned to 97; the runtime assert documents intent.
    assert_eq!(
        payload.len(),
        SUBMIT_AI_CLAIM_PAYLOAD_LEN,
        "submit_ai_claim payload must be exactly {SUBMIT_AI_CLAIM_PAYLOAD_LEN} bytes"
    );

    let ai_claim = derive_ai_claim_pda(oracle, proposer);
    kassandra_oracles_sdk::ix::submit_ai_claim_raw(
        &program_id(),
        *oracle,
        *proposer,
        ai_claim,
        *authority,
        payload,
    )
}

/// Load a Solana CLI JSON keypair file (a 64-byte JSON array: 32 secret ++ 32
/// public) into a [`Keypair`]. Clear errors on a missing file, non-array JSON,
/// wrong length, or bad key bytes.
pub fn load_keypair(path: &Path) -> Result<Keypair, SubmitError> {
    let display = path.display().to_string();
    let text = std::fs::read_to_string(path).map_err(|e| SubmitError::KeypairRead {
        path: display.clone(),
        message: e.to_string(),
    })?;
    let bytes: Vec<u8> =
        serde_json::from_str(&text).map_err(|e| SubmitError::KeypairMalformed {
            path: display.clone(),
            message: format!("expected a JSON array of 64 bytes: {e}"),
        })?;
    if bytes.len() != 64 {
        return Err(SubmitError::KeypairMalformed {
            path: display,
            message: format!("expected 64 bytes, got {}", bytes.len()),
        });
    }
    Keypair::try_from(&bytes[..]).map_err(|e| SubmitError::KeypairMalformed {
        path: display,
        message: format!("not a valid ed25519 keypair: {e}"),
    })
}

/// Fetch a recent blockhash via `getLatestBlockhash` (base58 → [`Hash`]).
pub async fn get_latest_blockhash(rpc: &dyn JsonRpc) -> Result<Hash, SubmitError> {
    let result = rpc
        .call("getLatestBlockhash", json!([{ "commitment": "confirmed" }]))
        .await?;
    let blockhash = result
        .get("value")
        .and_then(|v| v.get("blockhash"))
        .and_then(Value::as_str)
        .ok_or_else(|| SubmitError::Malformed {
            method: "getLatestBlockhash".to_string(),
            detail: "response had no `value.blockhash` string".to_string(),
        })?;
    Hash::from_str(blockhash).map_err(|e| SubmitError::Malformed {
        method: "getLatestBlockhash".to_string(),
        detail: format!("`value.blockhash` is not a valid base58 hash: {e}"),
    })
}

/// Build a legacy [`Message`] (payer = the authority) for the `submit_ai_claim`
/// instruction at `blockhash`, and SIGN it with `authority` (ed25519).
pub fn build_signed_transaction(
    oracle: &Pubkey,
    proposer: &Pubkey,
    authority: &Keypair,
    payload: &[u8; SUBMIT_AI_CLAIM_PAYLOAD_LEN],
    blockhash: Hash,
) -> Transaction {
    let authority_pubkey = authority.pubkey();
    let ix = build_submit_ai_claim_ix(oracle, proposer, &authority_pubkey, payload);
    let message = Message::new_with_blockhash(&[ix], Some(&authority_pubkey), &blockhash);
    Transaction::new(&[authority], message, blockhash)
}

/// Serialize a signed [`Transaction`] to a base64 wire string (bincode → base64)
/// for `sendTransaction` with `encoding: base64`.
pub fn encode_transaction(tx: &Transaction) -> String {
    let bytes = bincode::serialize(tx).expect("bincode-serializing a Transaction is infallible");
    BASE64.encode(bytes)
}

/// Send a base64-encoded, signed transaction via `sendTransaction`
/// (`encoding: base64`), returning the signature (base58).
///
/// A PREFLIGHT failure (e.g. the ai_claim PDA already exists = already-submitted,
/// or a wrong-phase reject) comes back from the RPC as a JSON-RPC error and is
/// surfaced as [`SubmitError::Rpc`].
pub async fn send_transaction(rpc: &dyn JsonRpc, tx_base64: &str) -> Result<String, SubmitError> {
    let result = rpc
        .call(
            "sendTransaction",
            json!([tx_base64, { "encoding": "base64" }]),
        )
        .await?;
    result
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| SubmitError::Malformed {
            method: "sendTransaction".to_string(),
            detail: "result was not a signature string".to_string(),
        })
}

/// How the send+confirm loop polls `getSignatureStatuses`.
#[derive(Clone, Copy, Debug)]
pub struct ConfirmOptions {
    /// Max number of status polls before giving up.
    pub max_polls: u32,
    /// Delay between polls.
    pub poll_interval: Duration,
    /// Whether `finalized` is required (else `confirmed` or `finalized` accepts).
    pub require_finalized: bool,
}

impl Default for ConfirmOptions {
    fn default() -> Self {
        // ~30s budget at 2s spacing — comfortably longer than a confirmed slot.
        Self {
            max_polls: 15,
            poll_interval: Duration::from_secs(2),
            require_finalized: false,
        }
    }
}

/// The outcome of a confirmed submission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirmation {
    /// The transaction signature (base58).
    pub signature: String,
    /// The reached confirmation status (`confirmed` / `finalized`).
    pub confirmation_status: String,
}

/// Poll `getSignatureStatuses` for `signature` until it reaches the target
/// commitment or the poll budget is exhausted.
///
/// A non-null `err` in the status is a FAILED tx → [`SubmitError::TxFailed`]. A
/// `null` status (not yet seen) or a below-target `processed` status keeps
/// polling; the budget exhausting → [`SubmitError::ConfirmTimeout`].
pub async fn confirm(
    rpc: &dyn JsonRpc,
    signature: &str,
    opts: ConfirmOptions,
) -> Result<Confirmation, SubmitError> {
    for poll in 0..opts.max_polls {
        if poll > 0 {
            tokio::time::sleep(opts.poll_interval).await;
        }

        let result = rpc
            .call(
                "getSignatureStatuses",
                json!([[signature], { "searchTransactionHistory": true }]),
            )
            .await?;

        let status = result
            .get("value")
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .ok_or_else(|| SubmitError::Malformed {
                method: "getSignatureStatuses".to_string(),
                detail: "response had no `value` array".to_string(),
            })?;

        // `null` → the cluster hasn't seen it yet; keep polling.
        if status.is_null() {
            continue;
        }

        // A landed tx that FAILED carries a non-null `err`.
        if let Some(err) = status.get("err") {
            if !err.is_null() {
                return Err(SubmitError::TxFailed {
                    signature: signature.to_string(),
                    error: err.to_string(),
                });
            }
        }

        let reached = status
            .get("confirmationStatus")
            .and_then(Value::as_str)
            .unwrap_or("");
        let accepted = if opts.require_finalized {
            reached == "finalized"
        } else {
            reached == "confirmed" || reached == "finalized"
        };
        if accepted {
            return Ok(Confirmation {
                signature: signature.to_string(),
                confirmation_status: reached.to_string(),
            });
        }
        // Otherwise it's `processed` (below target) — keep polling.
    }

    Err(SubmitError::ConfirmTimeout {
        signature: signature.to_string(),
        polls: opts.max_polls,
        seconds: opts.poll_interval.as_secs() * opts.max_polls as u64,
    })
}

/// The full keeper step: fetch a blockhash, build+sign the `submit_ai_claim`
/// transaction with `authority`, send it, and confirm it — returning the
/// [`Confirmation`] (signature + status) or a clear [`SubmitError`].
pub async fn submit_and_confirm(
    rpc: &dyn JsonRpc,
    oracle: &Pubkey,
    proposer: &Pubkey,
    authority: &Keypair,
    payload: &[u8; SUBMIT_AI_CLAIM_PAYLOAD_LEN],
    opts: ConfirmOptions,
) -> Result<Confirmation, SubmitError> {
    let blockhash = get_latest_blockhash(rpc).await?;
    let tx = build_signed_transaction(oracle, proposer, authority, payload, blockhash);
    let tx_base64 = encode_transaction(&tx);
    let signature = send_transaction(rpc, &tx_base64).await?;
    confirm(rpc, &signature, opts).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::MockRpc;

    /// A fresh ed25519 keypair for the sign/build tests. Each test uses one
    /// instance consistently; no fixed vector is asserted, so a random key is
    /// fine (the tests check the pubkey round-trips and the signature verifies).
    fn sample_keypair() -> Keypair {
        Keypair::new()
    }

    fn sample_keypair_json(kp: &Keypair) -> String {
        let bytes = kp.to_bytes(); // [u8; 64]
        serde_json::to_string(&bytes.to_vec()).unwrap()
    }

    fn oracle_pk() -> Pubkey {
        Pubkey::new_from_array([1u8; 32])
    }
    fn proposer_pk() -> Pubkey {
        Pubkey::new_from_array([2u8; 32])
    }

    fn sample_payload(option: u8) -> [u8; SUBMIT_AI_CLAIM_PAYLOAD_LEN] {
        // model_id = 0x11.., params_hash = 0x22.., io_hash = 0x33.., option.
        let mut p = [0u8; SUBMIT_AI_CLAIM_PAYLOAD_LEN];
        p[0..32].fill(0x11);
        p[32..64].fill(0x22);
        p[64..96].fill(0x33);
        p[96] = option;
        p
    }

    // --- discriminant + system program pins ---------------------------------

    #[test]
    fn discriminant_is_submit_ai_claim() {
        assert_eq!(SUBMIT_AI_CLAIM_DISCRIMINANT, 3);
    }

    #[test]
    fn system_program_id_is_canonical() {
        assert_eq!(
            kassandra_oracles_sdk::SYSTEM_PROGRAM_ID,
            Pubkey::from_str("11111111111111111111111111111111").unwrap()
        );
    }

    // --- instruction builder ------------------------------------------------

    #[test]
    fn ix_builder_has_exact_metas_and_data() {
        let oracle = oracle_pk();
        let proposer = proposer_pk();
        let authority = sample_keypair().pubkey();
        let payload = sample_payload(1);

        let ix = build_submit_ai_claim_ix(&oracle, &proposer, &authority, &payload);

        // Program id is the kassandra id (pinocchio [u8;32] → Pubkey).
        assert_eq!(ix.program_id, program_id());
        assert_eq!(ix.program_id, kassandra_oracles_sdk::PROGRAM_ID);

        // EXACT processor account order + roles.
        let ai_claim = derive_ai_claim_pda(&oracle, &proposer);
        let expected = [
            (oracle, false, true),                            // oracle (w)
            (proposer, false, true),                          // proposer PDA (w)
            (ai_claim, false, true),                          // ai_claim PDA (w)
            (authority, true, true),                          // authority (signer, w)
            (kassandra_oracles_sdk::SYSTEM_PROGRAM_ID, false, false), // system (ro)
        ];
        assert_eq!(ix.accounts.len(), expected.len());
        for (meta, (pk, signer, writable)) in ix.accounts.iter().zip(expected) {
            assert_eq!(meta.pubkey, pk);
            assert_eq!(meta.is_signer, signer);
            assert_eq!(meta.is_writable, writable);
        }

        // data == [disc=3] ++ 97-byte payload.
        assert_eq!(ix.data.len(), 1 + SUBMIT_AI_CLAIM_PAYLOAD_LEN);
        assert_eq!(ix.data[0], 3);
        assert_eq!(&ix.data[1..], &payload[..]);
    }

    #[test]
    fn ai_claim_pda_matches_claim_oracle_proposer_seeds() {
        let oracle = oracle_pk();
        let proposer = proposer_pk();
        let expected = Pubkey::find_program_address(
            &[b"claim", oracle.as_ref(), proposer.as_ref()],
            &program_id(),
        )
        .0;
        assert_eq!(derive_ai_claim_pda(&oracle, &proposer), expected);
    }

    #[test]
    fn proposer_pda_matches_proposer_oracle_authority_seeds() {
        let oracle = oracle_pk();
        let authority = sample_keypair().pubkey();
        let expected = Pubkey::find_program_address(
            &[b"proposer", oracle.as_ref(), authority.as_ref()],
            &program_id(),
        )
        .0;
        assert_eq!(derive_proposer_pda(&oracle, &authority), expected);
    }

    #[test]
    fn ix_carries_the_runoutput_payload_bytes() {
        // The 97-byte payload the runner emits (via ClaimMetadata::to_payload)
        // must land verbatim in the instruction data.
        use crate::hashing::ClaimMetadata;
        let meta = ClaimMetadata {
            model_id: [0xaa; 32],
            params_hash: [0xbb; 32],
            io_hash: [0xcc; 32],
        };
        let payload = meta.to_payload(2);
        let ix = build_submit_ai_claim_ix(
            &oracle_pk(),
            &proposer_pk(),
            &sample_keypair().pubkey(),
            &payload,
        );
        assert_eq!(&ix.data[1..], &payload[..]);
        assert_eq!(&ix.data[1..33], &[0xaa; 32]);
        assert_eq!(&ix.data[33..65], &[0xbb; 32]);
        assert_eq!(&ix.data[65..97], &[0xcc; 32]);
        assert_eq!(ix.data[97], 2);
    }

    // --- keypair loader -----------------------------------------------------

    #[test]
    fn load_keypair_parses_64_byte_json_array() {
        let kp = sample_keypair();
        let json = sample_keypair_json(&kp);
        let dir = std::env::temp_dir();
        let path = dir.join(format!("kass-test-kp-{}.json", std::process::id()));
        std::fs::write(&path, json).unwrap();

        let loaded = load_keypair(&path).unwrap();
        assert_eq!(loaded.pubkey(), kp.pubkey());

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_keypair_rejects_missing_file() {
        let err = load_keypair(Path::new("/no/such/kass-keypair.json")).unwrap_err();
        assert!(matches!(err, SubmitError::KeypairRead { .. }), "{err}");
    }

    #[test]
    fn load_keypair_rejects_wrong_length() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("kass-test-badkp-{}.json", std::process::id()));
        std::fs::write(&path, "[1,2,3]").unwrap();
        let err = load_keypair(&path).unwrap_err();
        assert!(matches!(err, SubmitError::KeypairMalformed { .. }), "{err}");
        std::fs::remove_file(&path).ok();
    }

    // --- message build + sign -----------------------------------------------

    #[test]
    fn signed_tx_verifies_and_carries_blockhash_and_payer() {
        let authority = sample_keypair();
        let oracle = oracle_pk();
        let proposer = proposer_pk();
        let payload = sample_payload(1);
        let blockhash = Hash::new_from_array([9u8; 32]);

        let tx = build_signed_transaction(&oracle, &proposer, &authority, &payload, blockhash);

        // The ed25519 signature verifies against the payer pubkey.
        assert!(tx.verify().is_ok(), "signature must verify");
        // Payer (fee payer / first account key) is the authority.
        assert_eq!(tx.message.account_keys[0], authority.pubkey());
        // The message carries the fetched blockhash.
        assert_eq!(tx.message.recent_blockhash, blockhash);
        // Exactly one signer (the authority); `verify()` above already proved
        // the signature is a real ed25519 sig over the message (not a default).
        assert_eq!(tx.signatures.len(), 1);
    }

    #[test]
    fn encode_transaction_is_base64_bincode() {
        let authority = sample_keypair();
        let tx = build_signed_transaction(
            &oracle_pk(),
            &proposer_pk(),
            &authority,
            &sample_payload(0),
            Hash::new_from_array([5u8; 32]),
        );
        let encoded = encode_transaction(&tx);
        let decoded = BASE64.decode(&encoded).unwrap();
        let roundtrip: Transaction = bincode::deserialize(&decoded).unwrap();
        assert_eq!(roundtrip, tx);
    }

    // --- send + confirm flow (offline via MockRpc) --------------------------

    fn fast_opts() -> ConfirmOptions {
        ConfirmOptions {
            max_polls: 3,
            poll_interval: Duration::from_millis(0),
            require_finalized: false,
        }
    }

    #[tokio::test]
    async fn get_latest_blockhash_decodes_base58() {
        let bh = Hash::new_from_array([4u8; 32]);
        let rpc = MockRpc::new().with(
            "getLatestBlockhash",
            json!({ "context": { "slot": 1 }, "value": { "blockhash": bh.to_string(), "lastValidBlockHeight": 100 } }),
        );
        let got = get_latest_blockhash(&rpc).await.unwrap();
        assert_eq!(got, bh);
    }

    #[tokio::test]
    async fn submit_and_confirm_happy_path() {
        let sig = "5".repeat(64); // a stand-in base58 signature string
        let rpc = MockRpc::new()
            .with(
                "getLatestBlockhash",
                json!({ "context": { "slot": 1 }, "value": { "blockhash": Hash::new_from_array([1u8;32]).to_string(), "lastValidBlockHeight": 100 } }),
            )
            .with("sendTransaction", json!(sig))
            .with(
                "getSignatureStatuses",
                json!({ "context": { "slot": 2 }, "value": [ { "slot": 2, "confirmations": null, "err": null, "confirmationStatus": "confirmed" } ] }),
            );

        let out = submit_and_confirm(
            &rpc,
            &oracle_pk(),
            &proposer_pk(),
            &sample_keypair(),
            &sample_payload(1),
            fast_opts(),
        )
        .await
        .unwrap();
        assert_eq!(out.signature, sig);
        assert_eq!(out.confirmation_status, "confirmed");
    }

    #[tokio::test]
    async fn confirm_surfaces_failed_tx_err() {
        let sig = "F".repeat(64);
        let rpc = MockRpc::new().with(
            "getSignatureStatuses",
            json!({ "context": { "slot": 2 }, "value": [ { "slot": 2, "confirmations": null, "err": { "InstructionError": [0, { "Custom": 7 }] }, "confirmationStatus": "processed" } ] }),
        );
        let err = confirm(&rpc, &sig, fast_opts()).await.unwrap_err();
        match err {
            SubmitError::TxFailed { signature, error } => {
                assert_eq!(signature, sig);
                assert!(error.contains("InstructionError"), "{error}");
            }
            other => panic!("expected TxFailed, got {other}"),
        }
    }

    #[tokio::test]
    async fn confirm_times_out_when_never_seen() {
        let sig = "N".repeat(64);
        // Always `null` (never seen) → poll budget exhausts.
        let rpc = MockRpc::new().with(
            "getSignatureStatuses",
            json!({ "context": { "slot": 2 }, "value": [ null ] }),
        );
        let err = confirm(&rpc, &sig, fast_opts()).await.unwrap_err();
        assert!(matches!(err, SubmitError::ConfirmTimeout { .. }), "{err}");
    }

    #[tokio::test]
    async fn send_transaction_surfaces_preflight_jsonrpc_error() {
        // A preflight failure comes back as a JSON-RPC error from the transport.
        // MockRpc only serves `result`s, so model the RPC error via an empty
        // mock (no canned `sendTransaction`) → Malformed-ish RpcError surfaced.
        let rpc = MockRpc::new();
        let err = send_transaction(&rpc, "AA==").await.unwrap_err();
        assert!(matches!(err, SubmitError::Rpc(_)), "{err}");
    }
}

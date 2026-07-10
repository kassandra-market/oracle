//! End-to-end pipeline test (Task R5) — fully offline, keyless.
//!
//! Drives the WHOLE runner pipeline as a black box, with no API key and no
//! network: a [`RunnerConfig`] goes in, the 97-byte `submit_ai_claim` payload
//! comes out. The pipeline exercised is exactly the production `run` path —
//!
//!   fetch + verify facts ([`MockFactFetcher`])
//!     → assemble the canonical prompt
//!     → complete ([`MockProvider`])
//!     → hash (`model_id` / `params_hash` / `io_hash`)
//!     → 97-byte payload
//!
//! via the same [`run_core`] / [`verify_core`] entry points `main` calls, only
//! with the trait objects pointed at the deterministic mocks. These are
//! pipeline-level assertions (payload decomposition, reproducibility,
//! tamper-rejection, a pinned payload anchor), not re-tests of the unit modules.

use kassandra_runner::cli::{
    build_model_config, run_core, verify_core, FactInput, OptionLabelInput, RunnerConfig,
    SubmittedClaim,
};
use kassandra_runner::fetch::MockFactFetcher;
use kassandra_runner::provider::MockProvider;
use sha2::{Digest, Sha256};

const FACT_URI: &str = "https://facts.example/btc-2025-12-31";
const FACT_CONTENT: &[u8] = b"On 2025-12-31 BTC closed at $98,000.";
const INTERPRETATION: &str =
    "Resolve YES if BTC closed at or above $100,000 on the date; otherwise NO.";

/// `sha256(content)` as 64 lowercase hex chars — the off-chain `content_hash`
/// convention the runner verifies against (see `runner/src/fetch.rs`).
fn content_hash_hex(content: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(content);
    hex::encode(h.finalize())
}

/// A fully-fixed config (one verified fact, two labelled options). Used for the
/// happy path, the reproducibility check, and the pinned payload anchor.
fn fixed_config() -> RunnerConfig {
    RunnerConfig {
        interpretation: INTERPRETATION.to_string(),
        options_count: 2,
        option_labels: Some(vec![
            OptionLabelInput {
                index: 0,
                label: "Yes".to_string(),
            },
            OptionLabelInput {
                index: 1,
                label: "No".to_string(),
            },
        ]),
        facts: vec![FactInput {
            content_hash: content_hash_hex(FACT_CONTENT),
            uri: FACT_URI.to_string(),
        }],
        oracle: None,
        proposer: None,
    }
}

/// A fetcher that serves the (matching) committed content for the fixed config.
fn honest_fetcher() -> MockFactFetcher {
    MockFactFetcher::new().with(FACT_URI, FACT_CONTENT.to_vec())
}

/// The pinned, deterministic model: option 1 ("No"), fixed raw response, the
/// default pinned model string so the metadata is stable across runs.
fn fixed_provider() -> MockProvider {
    MockProvider::new(1, r#"{"option_index":1}"#, "claude-opus-4-8")
}

// --- 1. full pipeline → 97-byte payload, decomposed ------------------------

#[tokio::test]
async fn pipeline_emits_decomposable_97_byte_payload() {
    let config = fixed_config();
    let fetcher = honest_fetcher();
    let provider = fixed_provider();

    let out = run_core(&config, build_model_config(None, None), &fetcher, &provider)
        .await
        .expect("run_core should succeed on a verified fact set");

    // The emitted option is the mock's, and is a valid categorical index.
    assert_eq!(out.option_index, 1, "option is the mock's choice");
    assert!(
        (out.option_index as u16) < (config.options_count as u16),
        "option must be < options_count"
    );

    // The payload decomposes EXACTLY as model_id[32] ++ params_hash[32] ++
    // io_hash[32] ++ option[1].
    let payload = hex::decode(&out.submit_ai_claim_payload_hex).unwrap();
    assert_eq!(payload.len(), 97, "payload is exactly 97 bytes");
    assert_eq!(hex::encode(&payload[0..32]), out.model_id_hex);
    assert_eq!(hex::encode(&payload[32..64]), out.params_hash_hex);
    assert_eq!(hex::encode(&payload[64..96]), out.io_hash_hex);
    assert_eq!(payload[96], out.option_index, "trailing byte is the option");
    assert_eq!(payload[96], 1);

    // The recorded model is the pinned string.
    assert_eq!(out.resolved_model_id, "claude-opus-4-8");
}

// --- 2. reproducibility (the challenger contract) --------------------------

#[tokio::test]
async fn pipeline_is_byte_reproducible() {
    let config = fixed_config();

    // Two independent runs with the same config + same mock.
    let first = run_core(
        &config,
        build_model_config(None, None),
        &honest_fetcher(),
        &fixed_provider(),
    )
    .await
    .unwrap();
    let second = run_core(
        &config,
        build_model_config(None, None),
        &honest_fetcher(),
        &fixed_provider(),
    )
    .await
    .unwrap();

    // Identical payload bytes — the determinism a challenger relies on to
    // reproduce the metadata from the same inputs.
    assert_eq!(
        first.submit_ai_claim_payload_hex, second.submit_ai_claim_payload_hex,
        "same config + same model => identical 97-byte payload"
    );
    assert_eq!(first.model_id_hex, second.model_id_hex);
    assert_eq!(first.params_hash_hex, second.params_hash_hex);
    assert_eq!(first.io_hash_hex, second.io_hash_hex);
    assert_eq!(first.option_index, second.option_index);
}

// --- 3. verify agrees with itself ------------------------------------------

#[tokio::test]
async fn verify_agrees_with_a_matching_submitted_claim() {
    let config = fixed_config();

    // First, produce the claim (as a proposer would).
    let produced = run_core(
        &config,
        build_model_config(None, None),
        &honest_fetcher(),
        &fixed_provider(),
    )
    .await
    .unwrap();

    // Then a challenger re-runs verify against that submitted option + hashes.
    let submitted = SubmittedClaim {
        option: produced.option_index,
        model_id_hex: Some(produced.model_id_hex.clone()),
        params_hash_hex: Some(produced.params_hash_hex.clone()),
        io_hash_hex: Some(produced.io_hash_hex.clone()),
    };
    let v = verify_core(
        &config,
        build_model_config(None, None),
        &honest_fetcher(),
        &fixed_provider(),
        &submitted,
    )
    .await
    .unwrap();

    assert!(
        v.option_matches,
        "re-run produces the same categorical option"
    );
    assert!(v.advice.contains("matches"), "advice: {}", v.advice);
    // The re-run reproduces every hash field, too.
    assert!(v.model_id_check.unwrap().matches);
    assert!(v.params_hash_check.unwrap().matches);
    assert!(v.io_hash_check.unwrap().matches);
    // And the produced payload equals the original.
    assert_eq!(
        v.produced.submit_ai_claim_payload_hex,
        produced.submit_ai_claim_payload_hex
    );
}

// --- 4. tamper arm: a fact whose body doesn't match its content_hash -------

#[tokio::test]
async fn pipeline_rejects_a_tampered_fact_and_produces_no_claim() {
    let config = fixed_config(); // commits to sha256(FACT_CONTENT)

    // The fetcher serves DIFFERENT bytes than the committed content_hash.
    let tampered =
        MockFactFetcher::new().with(FACT_URI, b"tampered: BTC closed at $101,000.".to_vec());

    let result = run_core(
        &config,
        build_model_config(None, None),
        &tampered,
        &fixed_provider(),
    )
    .await;

    // The fetch+verify gate REJECTS before the model is ever called: no claim.
    let err = result.expect_err("a content_hash mismatch must abort the run");
    assert!(
        err.to_string().contains("content_hash mismatch"),
        "expected a content_hash mismatch rejection, got: {err}"
    );
}

// --- 5. known-answer payload anchor ----------------------------------------
// A fully-fixed config + fixed mock response pins the EXACT 97-byte payload.
// Any cross-module drift in the assemble -> hash -> payload pipeline (a prompt
// format change, a hashing-encoding change, a width/offset change) flips this
// anchor end-to-end. If it fails intentionally, re-derive the value AND bump
// PROMPT_ASSEMBLY_VERSION when the prompt/params changed (see HASHING.md /
// PROMPT.md).

#[tokio::test]
async fn pipeline_payload_anchor() {
    let config = fixed_config();
    let out = run_core(
        &config,
        build_model_config(None, None),
        &honest_fetcher(),
        &fixed_provider(),
    )
    .await
    .unwrap();

    const EXPECTED_PAYLOAD_HEX: &str = "47a46a22f0c9fb105db3f0d8bda83ad51bd59369ab8c8c30cc32ba6356ac5a4a0bc9c6f79786e0f2810f1500426f95a5918a76e298bb6a88055f54068376025f7ef680c3e89b9c6f4c2bfa45ea21220494d7fe73d8edb6fcdf31121dd9a269a901";
    assert_eq!(
        out.submit_ai_claim_payload_hex, EXPECTED_PAYLOAD_HEX,
        "payload anchor drifted — re-derive + (if prompt/params changed) bump the version"
    );
}

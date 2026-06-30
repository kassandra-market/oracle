//! The `run` / `verify` CLI (Task R4).
//!
//! Wires the R0–R3 pieces into two commands:
//!
//! - **`run`**: load an oracle config → fetch + verify the agreed facts
//!   ([`crate::fetch`]) → assemble the prompt ([`crate::prompt`]) → call the
//!   provider ([`crate::provider`]) → compute the claim metadata
//!   ([`crate::hashing`]) → emit the `option`, the three hashes (hex), and the
//!   97-byte `submit_ai_claim` payload (hex) as JSON on stdout.
//! - **`verify`**: re-run for the same config and compare the produced option to
//!   a submitted claim's option (and, optionally, the submitted hashes) →
//!   advise "matches (no challenge)" vs "differs (consider challenging)".
//!
//! The default provider is Anthropic (Claude); `--mock` (or the
//! `KASSANDRA_RUNNER_MOCK` env var) selects the deterministic
//! [`crate::provider::MockProvider`] so the CLI runs offline with no API key.
//!
//! # Config shape
//!
//! The input config is JSON, read from `--config <path>` or stdin:
//!
//! ```json
//! {
//!   "interpretation": "Resolve YES if BTC closed above $100k on the date; otherwise NO.",
//!   "options_count": 2,
//!   "option_labels": [ { "index": 0, "label": "Yes" }, { "index": 1, "label": "No" } ],
//!   "facts": [
//!     { "content_hash": "<64-hex sha256 of the content>", "uri": "https://..." }
//!   ],
//!   "oracle": "<oracle pubkey, optional — only used to echo the AiClaim PDA seeds>",
//!   "proposer": "<proposer pubkey, optional>"
//! }
//! ```
//!
//! `option_labels`, `oracle`, and `proposer` are optional. `content_hash` is the
//! off-chain `sha256(content)` convention (see [`crate::fetch`]); the runner
//! recomputes it over the fetched bytes and rejects any mismatch.
//!
//! # Determinism caveat
//!
//! Re-running `verify` reproduces the same categorical option only as far as the
//! model is deterministic — no frontier API is bit-reproducible. The point of
//! `verify` is the categorical comparison plus confirming the inputs + metadata
//! reproduce: identical `model_id` / `params_hash` (deterministic), and an
//! `io_hash` that commits to the exact (input, raw response) seen.

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};

use crate::anthropic::{
    AnthropicProvider, DEFAULT_MAX_TOKENS, DEFAULT_MODEL, PROVIDER_ID, THINKING_MODE,
};
use crate::fetch::{fetch_and_verify_facts, FactFetcher, FactRef, HttpFactFetcher};
use crate::hashing::ClaimMetadata;
use crate::prompt::build_request;
use crate::provider::{
    AiProvider, CategoricalOption, CategoricalOptions, MockProvider, ModelConfig,
};

/// Env var that, when set non-empty, forces the MockProvider (offline).
pub const MOCK_ENV: &str = "KASSANDRA_RUNNER_MOCK";

// --- input config -----------------------------------------------------------

/// An agreed fact reference in the input config: `content_hash` (hex) + `uri`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FactInput {
    /// `sha256(content)` as 64 lowercase hex chars (a leading `0x` is allowed).
    pub content_hash: String,
    /// The http/https location the content is served from.
    pub uri: String,
}

/// An optional human-readable label for a categorical option.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OptionLabelInput {
    /// The on-chain option index.
    pub index: u8,
    /// The label text.
    pub label: String,
}

/// The oracle config the CLI consumes (JSON from `--config` or stdin).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunnerConfig {
    /// The oracle's interpretation / resolution-rule text (its on-chain
    /// `prompt_hash` commitment).
    pub interpretation: String,
    /// The number of categorical options (mirrors `Oracle.options_count`).
    pub options_count: u8,
    /// Optional per-option labels.
    #[serde(default)]
    pub option_labels: Option<Vec<OptionLabelInput>>,
    /// The agreed facts (each `content_hash` hex + `uri`).
    #[serde(default)]
    pub facts: Vec<FactInput>,
    /// Optional oracle pubkey — only echoed to describe the AiClaim PDA seeds.
    #[serde(default)]
    pub oracle: Option<String>,
    /// Optional proposer pubkey — only echoed to describe the AiClaim PDA seeds.
    #[serde(default)]
    pub proposer: Option<String>,
}

impl RunnerConfig {
    /// Parse `facts` into verifiable [`FactRef`]s.
    fn fact_refs(&self) -> anyhow::Result<Vec<FactRef>> {
        self.facts
            .iter()
            .map(|f| Ok(FactRef::new(parse_hex32(&f.content_hash)?, f.uri.clone())))
            .collect()
    }

    /// Build the categorical answer space from `options_count` + `option_labels`.
    fn categorical_options(&self) -> CategoricalOptions {
        let labels = self.option_labels.as_ref().map(|ls| {
            ls.iter()
                .map(|l| CategoricalOption {
                    index: l.index,
                    label: Some(l.label.clone()),
                })
                .collect()
        });
        CategoricalOptions {
            count: self.options_count,
            labels,
        }
    }
}

// --- output -----------------------------------------------------------------

/// The AiClaim PDA seed hint (echoed only when oracle/proposer are in the
/// config). The PDA is `find_program_address([b"claim", oracle, proposer],
/// program_id)`; we surface the seeds rather than derive the address (which
/// would also need the program id + base58 pubkey decoding).
#[derive(Clone, Debug, Serialize)]
pub struct ClaimPdaSeeds {
    /// The literal seed prefix (`b"claim"`).
    pub seed_prefix: String,
    /// The oracle pubkey seed.
    pub oracle: String,
    /// The proposer pubkey seed.
    pub proposer: String,
}

/// The `run` output (serialized to stdout as JSON).
#[derive(Clone, Debug, Serialize)]
pub struct RunOutput {
    /// The chosen categorical option index.
    pub option_index: u8,
    /// `sha256(model_id_string)` as hex.
    pub model_id_hex: String,
    /// `params_hash` as hex.
    pub params_hash_hex: String,
    /// `io_hash` as hex.
    pub io_hash_hex: String,
    /// The exact 97-byte `submit_ai_claim` payload (`model_id ++ params_hash ++
    /// io_hash ++ option`) as hex.
    pub submit_ai_claim_payload_hex: String,
    /// The resolved model identifier string actually recorded.
    pub resolved_model_id: String,
    /// The AiClaim PDA seeds, if oracle/proposer were provided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim_pda_seeds: Option<ClaimPdaSeeds>,
}

/// The result of comparing one submitted hash field.
#[derive(Clone, Debug, Serialize)]
pub struct HashCheck {
    /// What was submitted (hex).
    pub submitted: String,
    /// What we produced (hex).
    pub produced: String,
    /// Whether they match.
    pub matches: bool,
}

/// The `verify` output.
#[derive(Clone, Debug, Serialize)]
pub struct VerifyOutput {
    /// The full re-run output (produced option + hashes + payload).
    pub produced: RunOutput,
    /// The submitted claim's option.
    pub submitted_option: u8,
    /// Whether the produced option matches the submitted one.
    pub option_matches: bool,
    /// Human-readable advice.
    pub advice: String,
    /// Optional per-hash comparisons (only when submitted hashes were provided).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id_check: Option<HashCheck>,
    /// See [`Self::model_id_check`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params_hash_check: Option<HashCheck>,
    /// See [`Self::model_id_check`]. NOTE: against the live Anthropic provider an
    /// `io_hash` mismatch is EXPECTED and not grounds to challenge — the model's
    /// raw response text varies run-to-run, so `io_hash` (a commitment to the exact
    /// (input, output) the submitter used) rarely reproduces. Base the
    /// challenge/no-challenge decision on `option_matches`, not this field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub io_hash_check: Option<HashCheck>,
}

// --- core (mock-testable: takes trait objects) ------------------------------

/// Build the [`ModelConfig`] from CLI knobs (defaults pinned to Opus 4.8 +
/// adaptive thinking). Centralizes the config so `params_hash` is stable.
pub fn build_model_config(model: Option<String>, max_tokens: Option<u32>) -> ModelConfig {
    ModelConfig {
        model_id: model.unwrap_or_else(|| DEFAULT_MODEL.to_string()),
        provider: PROVIDER_ID.to_string(),
        max_tokens: max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
        thinking: Some(THINKING_MODE.to_string()),
    }
}

/// The `run` core, generic over the fetcher + provider via trait objects so it
/// is fully testable offline with [`crate::fetch::MockFactFetcher`] +
/// [`MockProvider`]. `main` passes the real [`HttpFactFetcher`] +
/// [`AnthropicProvider`].
pub async fn run_core(
    config: &RunnerConfig,
    model_config: ModelConfig,
    fetcher: &dyn FactFetcher,
    provider: &dyn AiProvider,
) -> anyhow::Result<RunOutput> {
    let fact_refs = config.fact_refs()?;
    let facts = fetch_and_verify_facts(fetcher, &fact_refs).await?;

    let options = config.categorical_options();
    let req = build_request(&config.interpretation, &facts, options, model_config);
    let resp = provider.complete(&req).await?;

    let meta = ClaimMetadata::compute(&req, &resp);
    let payload = meta.to_payload(resp.option_index);

    let claim_pda_seeds = match (&config.oracle, &config.proposer) {
        (Some(oracle), Some(proposer)) => Some(ClaimPdaSeeds {
            seed_prefix: "claim".to_string(),
            oracle: oracle.clone(),
            proposer: proposer.clone(),
        }),
        _ => None,
    };

    Ok(RunOutput {
        option_index: resp.option_index,
        model_id_hex: to_hex(&meta.model_id),
        params_hash_hex: to_hex(&meta.params_hash),
        io_hash_hex: to_hex(&meta.io_hash),
        submit_ai_claim_payload_hex: to_hex(&payload),
        resolved_model_id: resp.model_id,
        claim_pda_seeds,
    })
}

/// The submitted claim a `verify` run compares against.
#[derive(Clone, Debug, Default)]
pub struct SubmittedClaim {
    /// The submitted categorical option.
    pub option: u8,
    /// Optional submitted `model_id` (hex) to compare.
    pub model_id_hex: Option<String>,
    /// Optional submitted `params_hash` (hex) to compare.
    pub params_hash_hex: Option<String>,
    /// Optional submitted `io_hash` (hex) to compare.
    pub io_hash_hex: Option<String>,
}

/// The `verify` core: re-run, then compare the produced option (and optionally
/// the submitted hashes) to advise on challenging.
pub async fn verify_core(
    config: &RunnerConfig,
    model_config: ModelConfig,
    fetcher: &dyn FactFetcher,
    provider: &dyn AiProvider,
    submitted: &SubmittedClaim,
) -> anyhow::Result<VerifyOutput> {
    let produced = run_core(config, model_config, fetcher, provider).await?;
    let option_matches = produced.option_index == submitted.option;

    let advice = if option_matches {
        "matches (no challenge) — the re-run produced the same categorical option as the submitted claim"
            .to_string()
    } else {
        format!(
            "differs (consider challenging) — re-run produced option {}, submitted claim was option {}",
            produced.option_index, submitted.option
        )
    };

    let check = |submitted: &Option<String>, produced: &str| {
        submitted.as_ref().map(|s| {
            let s_norm = normalize_hex(s);
            HashCheck {
                submitted: s_norm.clone(),
                produced: produced.to_string(),
                matches: s_norm == produced,
            }
        })
    };

    Ok(VerifyOutput {
        submitted_option: submitted.option,
        option_matches,
        advice,
        model_id_check: check(&submitted.model_id_hex, &produced.model_id_hex),
        params_hash_check: check(&submitted.params_hash_hex, &produced.params_hash_hex),
        io_hash_check: check(&submitted.io_hash_hex, &produced.io_hash_hex),
        produced,
    })
}

// --- helpers ----------------------------------------------------------------

/// Lowercase hex of a byte slice.
fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

/// Normalize a hex string for comparison (strip `0x`, lowercase).
fn normalize_hex(s: &str) -> String {
    s.strip_prefix("0x").unwrap_or(s).to_ascii_lowercase()
}

/// Parse exactly 32 bytes from a 64-char hex string (optional `0x` prefix).
fn parse_hex32(s: &str) -> anyhow::Result<[u8; 32]> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if s.len() != 64 {
        anyhow::bail!(
            "content_hash must be 64 hex chars (32 bytes), got {} chars",
            s.len()
        );
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[2 * i..2 * i + 2], 16)
            .map_err(|e| anyhow::anyhow!("invalid hex in content_hash: {e}"))?;
    }
    Ok(out)
}

/// Load the config from `--config <path>` or, when `None`, stdin.
fn load_config(path: Option<&Path>) -> anyhow::Result<RunnerConfig> {
    let text = match path {
        Some(p) => std::fs::read_to_string(p)
            .map_err(|e| anyhow::anyhow!("failed to read config `{}`: {e}", p.display()))?,
        None => {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| anyhow::anyhow!("failed to read config from stdin: {e}"))?;
            buf
        }
    };
    serde_json::from_str(&text).map_err(|e| anyhow::anyhow!("invalid config JSON: {e}"))
}

/// Whether to use the mock provider (the `--mock` flag or `KASSANDRA_RUNNER_MOCK`
/// set non-empty).
fn use_mock(flag: bool) -> bool {
    flag || std::env::var(MOCK_ENV)
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

// --- clap -------------------------------------------------------------------

/// The Kassandra off-chain AI runner CLI.
#[derive(Debug, Parser)]
#[command(name = "kassandra-runner", version, about)]
pub struct Cli {
    /// The subcommand.
    #[command(subcommand)]
    pub command: Command,
}

/// The runner subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Resolve an oracle: fetch + verify facts, call the model, emit the claim
    /// metadata + the 97-byte submit_ai_claim payload as JSON.
    Run(RunArgs),
    /// Re-run for the same config and compare the produced option to a submitted
    /// claim's option; advise whether to challenge.
    Verify(VerifyArgs),
}

/// Shared provider/config options.
#[derive(Debug, Parser)]
pub struct CommonArgs {
    /// Path to the JSON config; if omitted, the config is read from stdin.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Use the deterministic MockProvider (offline; no API key needed). Also
    /// enabled by setting KASSANDRA_RUNNER_MOCK.
    #[arg(long)]
    pub mock: bool,
    /// Override the pinned model string (default: claude-opus-4-8).
    #[arg(long)]
    pub model: Option<String>,
    /// Override max_tokens (default: 4096).
    #[arg(long)]
    pub max_tokens: Option<u32>,
}

/// `run` arguments.
#[derive(Debug, Parser)]
pub struct RunArgs {
    /// Shared options.
    #[command(flatten)]
    pub common: CommonArgs,
}

/// `verify` arguments.
#[derive(Debug, Parser)]
pub struct VerifyArgs {
    /// Shared options.
    #[command(flatten)]
    pub common: CommonArgs,
    /// The submitted claim's categorical option to compare against.
    #[arg(long)]
    pub option: u8,
    /// Optional submitted model_id (hex) to compare.
    #[arg(long)]
    pub submitted_model_id: Option<String>,
    /// Optional submitted params_hash (hex) to compare.
    #[arg(long)]
    pub submitted_params_hash: Option<String>,
    /// Optional submitted io_hash (hex) to compare.
    #[arg(long)]
    pub submitted_io_hash: Option<String>,
}

/// Build the chosen provider (mock or real Anthropic).
fn build_provider(mock: bool) -> anyhow::Result<Box<dyn AiProvider>> {
    if use_mock(mock) {
        Ok(Box::new(MockProvider::default()))
    } else {
        Ok(Box::new(AnthropicProvider::from_env()?))
    }
}

/// Parse args and dispatch. `main` calls this inside `#[tokio::main]`.
pub async fn run_cli() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Run(args) => {
            let config = load_config(args.common.config.as_deref())?;
            let model_config = build_model_config(args.common.model, args.common.max_tokens);
            let fetcher = HttpFactFetcher::new()?;
            let provider = build_provider(args.common.mock)?;
            let out = run_core(&config, model_config, &fetcher, provider.as_ref()).await?;
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        Command::Verify(args) => {
            let config = load_config(args.common.config.as_deref())?;
            let model_config = build_model_config(args.common.model, args.common.max_tokens);
            let fetcher = HttpFactFetcher::new()?;
            let provider = build_provider(args.common.mock)?;
            let submitted = SubmittedClaim {
                option: args.option,
                model_id_hex: args.submitted_model_id,
                params_hash_hex: args.submitted_params_hash,
                io_hash_hex: args.submitted_io_hash,
            };
            let out = verify_core(
                &config,
                model_config,
                &fetcher,
                provider.as_ref(),
                &submitted,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::MockFactFetcher;
    use sha2::{Digest, Sha256};

    fn sha256_hex(content: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(content);
        to_hex(&h.finalize())
    }

    fn sample_config(uri: &str, content: &[u8]) -> RunnerConfig {
        RunnerConfig {
            interpretation: "Resolve YES if BTC closed above $100k; otherwise NO.".to_string(),
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
                content_hash: sha256_hex(content),
                uri: uri.to_string(),
            }],
            oracle: None,
            proposer: None,
        }
    }

    #[test]
    fn parse_hex32_roundtrips() {
        let bytes = [0xabu8; 32];
        let hex = to_hex(&bytes);
        assert_eq!(parse_hex32(&hex).unwrap(), bytes);
        assert_eq!(parse_hex32(&format!("0x{hex}")).unwrap(), bytes);
        assert!(parse_hex32("abcd").is_err());
        assert!(parse_hex32(&"zz".repeat(32)).is_err());
    }

    #[tokio::test]
    async fn run_core_with_mocks_emits_97_byte_payload() {
        let content = b"BTC closed at $98,000.";
        let uri = "https://facts.example/btc";
        let config = sample_config(uri, content);
        let fetcher = MockFactFetcher::new().with(uri, content.to_vec());
        let provider = MockProvider::new(1, r#"{"option_index":1}"#, "mock-claude");
        let model_config = build_model_config(None, None);

        let out = run_core(&config, model_config, &fetcher, &provider)
            .await
            .unwrap();

        // Option matches the mock.
        assert_eq!(out.option_index, 1);

        // Payload is exactly 97 bytes: model_id[32] ++ params_hash[32] ++
        // io_hash[32] ++ option[1].
        let payload = parse_payload(&out.submit_ai_claim_payload_hex);
        assert_eq!(payload.len(), 97);
        assert_eq!(to_hex(&payload[0..32]), out.model_id_hex);
        assert_eq!(to_hex(&payload[32..64]), out.params_hash_hex);
        assert_eq!(to_hex(&payload[64..96]), out.io_hash_hex);
        assert_eq!(payload[96], 1);

        // The resolved model id is the mock's.
        assert_eq!(out.resolved_model_id, "mock-claude");
    }

    #[tokio::test]
    async fn run_core_rejects_tampered_fact() {
        let committed = b"the agreed fact";
        let tampered = b"a tampered fact";
        let uri = "https://facts.example/x";
        let config = sample_config(uri, committed);
        // Serve tampered bytes that don't match the committed content_hash.
        let fetcher = MockFactFetcher::new().with(uri, tampered.to_vec());
        let provider = MockProvider::default();

        let err = run_core(&config, build_model_config(None, None), &fetcher, &provider)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("content_hash mismatch"), "{err}");
    }

    #[tokio::test]
    async fn verify_core_reports_match_and_mismatch() {
        let content = b"some fact";
        let uri = "https://facts.example/y";
        let config = sample_config(uri, content);
        let fetcher = MockFactFetcher::new().with(uri, content.to_vec());
        let provider = MockProvider::new(0, r#"{"option_index":0}"#, "mock-claude");

        // Matching submitted option.
        let matching = SubmittedClaim {
            option: 0,
            ..Default::default()
        };
        let out = verify_core(
            &config,
            build_model_config(None, None),
            &fetcher,
            &provider,
            &matching,
        )
        .await
        .unwrap();
        assert!(out.option_matches);
        assert!(out.advice.contains("matches"));

        // Differing submitted option.
        let differing = SubmittedClaim {
            option: 1,
            ..Default::default()
        };
        let out = verify_core(
            &config,
            build_model_config(None, None),
            &fetcher,
            &provider,
            &differing,
        )
        .await
        .unwrap();
        assert!(!out.option_matches);
        assert!(out.advice.contains("differs"));
    }

    #[tokio::test]
    async fn verify_core_compares_submitted_hashes() {
        let content = b"a fact";
        let uri = "https://facts.example/z";
        let config = sample_config(uri, content);
        let fetcher = MockFactFetcher::new().with(uri, content.to_vec());
        let provider = MockProvider::new(0, r#"{"option_index":0}"#, "mock-claude");

        // First produce the real hashes via run_core.
        let produced = run_core(&config, build_model_config(None, None), &fetcher, &provider)
            .await
            .unwrap();

        let submitted = SubmittedClaim {
            option: 0,
            model_id_hex: Some(produced.model_id_hex.clone()),
            params_hash_hex: Some(produced.params_hash_hex.clone()),
            io_hash_hex: Some("deadbeef".to_string()), // intentionally wrong
        };
        let out = verify_core(
            &config,
            build_model_config(None, None),
            &fetcher,
            &provider,
            &submitted,
        )
        .await
        .unwrap();

        assert!(out.model_id_check.unwrap().matches);
        assert!(out.params_hash_check.unwrap().matches);
        assert!(!out.io_hash_check.unwrap().matches);
    }

    fn parse_payload(hex: &str) -> Vec<u8> {
        (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }
}

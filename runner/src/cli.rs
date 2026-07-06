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
use std::str::FromStr;

use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

use crate::anthropic::{
    AnthropicProvider, DEFAULT_MAX_TOKENS, DEFAULT_MODEL, PROVIDER_ID, THINKING_MODE,
};
use crate::constants::SUBMIT_AI_CLAIM_PAYLOAD_LEN;
use crate::fetch::{fetch_and_verify_facts, FactFetcher, FactRef, HttpFactFetcher};
use crate::hashing::ClaimMetadata;
use crate::prompt::build_request;
use crate::provider::{
    AiProvider, CategoricalOption, CategoricalOptions, MockProvider, ModelConfig,
};
use crate::submit::{derive_proposer_pda, submit_and_confirm, ConfirmOptions, SubmitError};

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
    /// The result of the on-chain submission, present only in `--submit` keeper
    /// mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submission: Option<SubmissionOutput>,
    /// The exact 97-byte `submit_ai_claim` payload as raw bytes — the SAME bytes
    /// as [`Self::submit_ai_claim_payload_hex`]. Carried (not re-serialized) so
    /// the `--submit` path signs the runner's OWN payload verbatim rather than
    /// recomputing it.
    #[serde(skip)]
    pub submit_ai_claim_payload: [u8; SUBMIT_AI_CLAIM_PAYLOAD_LEN],
}

/// The on-chain submission result appended to a `--submit` run.
#[derive(Clone, Debug, Serialize)]
pub struct SubmissionOutput {
    /// The confirmed transaction signature (base58).
    pub signature: String,
    /// The reached confirmation status (`confirmed` / `finalized`).
    pub confirmation_status: String,
    /// The oracle the claim was submitted against (base58).
    pub oracle: String,
    /// The derived Proposer PDA (`[b"proposer", oracle, authority]`, base58).
    pub proposer: String,
    /// The signing authority = the `--keypair` pubkey (base58).
    pub authority: String,
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
        submission: None,
        submit_ai_claim_payload: payload,
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
use crate::hashing::to_hex;

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

/// Build a [`RunnerConfig`] by reading the oracle + its agreed facts over RPC
/// and pairing them with a verified off-chain interpretation.
///
/// Generic over [`crate::rpc::JsonRpc`] so it is testable offline with
/// [`crate::rpc::MockRpc`]. Fetches the `Oracle` (owner + `AccountType` tag
/// verified, then Pod-decoded via the shared struct), asserts
/// `sha256(prompt_text) == oracle.prompt_hash` (REJECTS a mismatch), enumerates
/// the AGREED facts, and assembles the config: `options_count`/`deadline`-backed
/// facts from chain, the interpretation from the prompt text.
/// The subset of the oracle-metadata JSON the runner consumes. Fetched from the
/// on-chain `oracle_meta.uri` and verified against `uri_hash`. `promptTemplate`
/// is the AI-runner interpretation (defaulted at creation); `interpretation` is
/// the optional human rules — the runner prefers the former.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OracleMetaJson {
    #[serde(default)]
    prompt_template: Option<String>,
    #[serde(default)]
    interpretation: Option<String>,
}

pub async fn build_config_from_chain(
    rpc: &dyn crate::rpc::JsonRpc,
    fetcher: &dyn crate::fetch::FactFetcher,
    oracle_pubkey: &str,
) -> anyhow::Result<RunnerConfig> {
    // 1. Read the on-chain metadata (subject/options/uri/uri_hash).
    let meta = crate::rpc::fetch_oracle_meta(rpc, oracle_pubkey).await?;
    if meta.uri.is_empty() {
        anyhow::bail!(
            "oracle `{oracle_pubkey}` has no metadata uri on chain — cannot read the interpretation"
        );
    }

    // 2. Fetch the metadata JSON and VERIFY it against the on-chain uri_hash
    //    (exactly the fact content-hash contract: fetch by uri, check the hash).
    let json_bytes = fetcher.fetch(&meta.uri).await.map_err(|e| {
        anyhow::anyhow!(
            "failed to fetch oracle metadata JSON at `{}`: {e}",
            meta.uri
        )
    })?;
    let actual: [u8; 32] = Sha256::digest(&json_bytes).into();
    if actual != meta.uri_hash {
        anyhow::bail!(
            "oracle metadata JSON at `{}` hashes to sha256 {} but the on-chain uri_hash is {} \
             (the hosted JSON does not match what this oracle committed to)",
            meta.uri,
            to_hex(&actual),
            to_hex(&meta.uri_hash),
        );
    }

    // 3. Parse + take the interpretation (promptTemplate preferred).
    let meta_json: OracleMetaJson = serde_json::from_slice(&json_bytes)
        .map_err(|e| anyhow::anyhow!("oracle metadata JSON is malformed: {e}"))?;
    let interpretation = meta_json
        .prompt_template
        .filter(|s| !s.trim().is_empty())
        .or_else(|| meta_json.interpretation.filter(|s| !s.trim().is_empty()))
        .ok_or_else(|| {
            anyhow::anyhow!("oracle metadata JSON has no promptTemplate/interpretation text")
        })?;

    // Option labels come straight from the on-chain (program-readable) labels.
    let option_labels = (!meta.options.is_empty()).then(|| {
        meta.options
            .iter()
            .enumerate()
            .map(|(i, label)| OptionLabelInput {
                index: i as u8,
                label: label.clone(),
            })
            .collect()
    });

    // 4. Agreed facts (unchanged).
    let facts = crate::rpc::fetch_agreed_facts(rpc, oracle_pubkey).await?;
    let facts = facts
        .into_iter()
        .map(|f| FactInput {
            content_hash: to_hex(&f.content_hash),
            uri: f.uri,
        })
        .collect();

    Ok(RunnerConfig {
        interpretation,
        options_count: meta.options.len() as u8,
        option_labels,
        facts,
        oracle: Some(oracle_pubkey.to_string()),
        proposer: None,
    })
}

/// Resolve the [`RunnerConfig`] for a command from its [`CommonArgs`]: either
/// the explicit JSON config (`--config`/stdin) or the on-chain fetch
/// (`--oracle` + `--rpc-url` + `--prompt-file`). The two modes are mutually
/// exclusive.
async fn resolve_config(common: &CommonArgs) -> anyhow::Result<RunnerConfig> {
    match &common.oracle {
        Some(oracle) => {
            if common.config.is_some() {
                anyhow::bail!("--oracle and --config are mutually exclusive");
            }
            let rpc_url = common
                .rpc_url
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("--oracle requires --rpc-url <url>"))?;
            // The interpretation is read from chain (oracle_meta.uri → JSON,
            // verified against uri_hash) — no --prompt-file needed.
            let rpc = crate::rpc::HttpJsonRpc::new(rpc_url.clone())?;
            let fetcher = HttpFactFetcher::new()?;
            build_config_from_chain(&rpc, &fetcher, oracle).await
        }
        None => load_config(common.config.as_deref()),
    }
}

// --- --submit keeper mode ----------------------------------------------------

/// The validated `--submit` target: the RPC url + keypair path to load + the
/// oracle to submit against. `None` when `--submit` is not set (emit-only, the
/// default).
#[derive(Debug)]
struct SubmitTarget {
    rpc_url: String,
    keypair_path: PathBuf,
    oracle: Pubkey,
}

/// Resolve the oracle pubkey for submission: the explicit `--oracle` (on-chain
/// mode) or the config's `oracle` field (explicit-config mode). Errors clearly
/// if neither is present or the value is not a valid base58 pubkey.
fn resolve_submit_oracle(common: &CommonArgs, config: &RunnerConfig) -> anyhow::Result<Pubkey> {
    let raw = common
        .oracle
        .as_deref()
        .or(config.oracle.as_deref())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "--submit needs an oracle: pass --oracle <pubkey> or set `oracle` in the config"
            )
        })?;
    Pubkey::from_str(raw).map_err(|e| anyhow::anyhow!("invalid oracle pubkey `{raw}`: {e}"))
}

/// Validate the `--submit` args and resolve the submission target.
///
/// `--submit` requires BOTH `--keypair <path>` (the proposer's authority) and
/// `--rpc-url <url>` (the network to submit to — already required in on-chain
/// `--oracle` mode, but ALSO required for submission in explicit-config mode).
/// The two required-arg checks are ordered keypair-then-rpc-url so each surfaces
/// a distinct, clear error. Returns `None` when `--submit` is off.
fn resolve_submit_target(
    common: &CommonArgs,
    submit: bool,
    keypair: Option<&Path>,
    config: &RunnerConfig,
) -> anyhow::Result<Option<SubmitTarget>> {
    if !submit {
        return Ok(None);
    }
    let keypair_path = keypair
        .ok_or_else(|| anyhow::anyhow!("--submit requires --keypair <path>"))?
        .to_path_buf();
    let rpc_url = common.rpc_url.clone().ok_or_else(|| {
        anyhow::anyhow!("--submit requires --rpc-url <url> (the network to submit the claim to)")
    })?;
    let oracle = resolve_submit_oracle(common, config)?;
    Ok(Some(SubmitTarget {
        rpc_url,
        keypair_path,
        oracle,
    }))
}

/// Sign + submit + confirm the run's claim over `rpc` — the testable seam
/// (takes a `&dyn JsonRpc` so the keeper flow runs OFFLINE against
/// [`crate::rpc::MockRpc`], mirroring [`build_config_from_chain`]).
///
/// The submitted transaction carries the RunOutput's OWN 97-byte `payload`
/// verbatim (REUSE — never recomputed), signed by `authority`; the Proposer PDA
/// is DERIVED from `[b"proposer", oracle, authority]`.
pub async fn submit_claim(
    rpc: &dyn crate::rpc::JsonRpc,
    oracle: &Pubkey,
    authority: &Keypair,
    payload: &[u8; SUBMIT_AI_CLAIM_PAYLOAD_LEN],
    opts: ConfirmOptions,
) -> Result<SubmissionOutput, SubmitError> {
    let authority_pubkey = authority.pubkey();
    let proposer = derive_proposer_pda(oracle, &authority_pubkey);
    let confirmation = submit_and_confirm(rpc, oracle, &proposer, authority, payload, opts).await?;
    Ok(SubmissionOutput {
        signature: confirmation.signature,
        confirmation_status: confirmation.confirmation_status,
        oracle: oracle.to_string(),
        proposer: proposer.to_string(),
        authority: authority_pubkey.to_string(),
    })
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
    /// Path to the JSON config; if omitted (and no `--oracle`), read from stdin.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Build the config from an on-chain oracle (base58 pubkey) instead of a
    /// JSON `--config`: the oracle's `options_count`/`deadline`/agreed facts are
    /// read over RPC, and the interpretation comes from `--prompt-file` (whose
    /// sha256 must equal the on-chain `prompt_hash`). Requires `--rpc-url` +
    /// `--prompt-file`; mutually exclusive with `--config`.
    #[arg(long)]
    pub oracle: Option<String>,
    /// Solana JSON-RPC url used with `--oracle`.
    #[arg(long)]
    pub rpc_url: Option<String>,
    /// Path to the interpretation prompt-text file used with `--oracle`; its
    /// sha256 must equal the on-chain `oracle.prompt_hash` (else the run is
    /// rejected).
    #[arg(long)]
    pub prompt_file: Option<PathBuf>,
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
    /// Keeper mode: after producing the claim, SIGN + SEND + CONFIRM the
    /// `submit_ai_claim` transaction on chain (default: emit-only, no network
    /// write). Requires `--keypair` and `--rpc-url`; the signer MUST be the
    /// proposer's authority. The oracle comes from `--oracle` or the config's
    /// `oracle` field; the Proposer PDA is derived from it + the keypair pubkey.
    #[arg(long)]
    pub submit: bool,
    /// Path to the Solana CLI keypair JSON (a 64-byte array) that signs the
    /// `submit_ai_claim` transaction in `--submit` mode. This keypair MUST be
    /// the proposer's registered `authority`.
    #[arg(long)]
    pub keypair: Option<PathBuf>,
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
            let config = resolve_config(&args.common).await?;
            // Validate `--submit` args BEFORE the (paid) model call so a missing
            // --keypair / --rpc-url / oracle fails fast.
            let submit_target =
                resolve_submit_target(&args.common, args.submit, args.keypair.as_deref(), &config)?;
            let model_config = build_model_config(args.common.model, args.common.max_tokens);
            let fetcher = HttpFactFetcher::new()?;
            let provider = build_provider(args.common.mock)?;
            let mut out = run_core(&config, model_config, &fetcher, provider.as_ref()).await?;

            if let Some(target) = submit_target {
                let authority = crate::submit::load_keypair(&target.keypair_path)?;
                let rpc = crate::rpc::HttpJsonRpc::new(target.rpc_url)?;
                // Reuse the run's OWN payload bytes (never recomputed) so the
                // submitted claim can never diverge from the emitted metadata.
                let submission = submit_claim(
                    &rpc,
                    &target.oracle,
                    &authority,
                    &out.submit_ai_claim_payload,
                    ConfirmOptions::default(),
                )
                .await?;
                out.submission = Some(submission);
            }

            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        Command::Verify(args) => {
            let config = resolve_config(&args.common).await?;
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

    // --- on-chain config build (offline via MockRpc) -----------------------

    /// Hand-encode an `oracle_meta` account body (mirrors `write_oracle_meta`).
    fn oracle_meta_account_bytes(
        oracle: [u8; 32],
        subject: &str,
        options: &[&str],
        uri: &str,
        uri_hash: [u8; 32],
    ) -> Vec<u8> {
        use kassandra_sdk::accounts::AccountType;
        let mut d = vec![AccountType::OracleMeta.as_u8(), 255];
        d.extend_from_slice(&oracle);
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

    #[tokio::test]
    async fn build_config_from_chain_reads_meta_and_runs() {
        use crate::fetch::MockFactFetcher;
        use crate::rpc::MockRpc;
        use bytemuck::Zeroable;
        use kassandra_sdk::accounts::{AccountType, Fact};
        use serde_json::json;

        let oracle_pk = "So11111111111111111111111111111111111111112";
        let oracle_bytes: [u8; 32] = bs58::decode(oracle_pk)
            .into_vec()
            .unwrap()
            .try_into()
            .unwrap();

        // The off-chain metadata JSON that `oracle_meta.uri` points at.
        let meta_uri = "https://meta.example/oracle.json";
        let meta_json = r#"{"version":1,"subject":"Did BTC close >= $100k?","options":["Yes","No"],"promptTemplate":"Resolve YES if BTC closed at or above $100,000; otherwise NO."}"#;
        let uri_hash: [u8; 32] = Sha256::digest(meta_json.as_bytes()).into();
        let meta_bytes = oracle_meta_account_bytes(
            oracle_bytes,
            "Did BTC close >= $100k?",
            &["Yes", "No"],
            meta_uri,
            uri_hash,
        );

        // One agreed fact whose off-chain content the mock fetcher serves.
        let fact_content = b"BTC closed at $98,000.";
        let content_hash: [u8; 32] = Sha256::digest(fact_content).into();
        let fact_uri = "https://facts.example/btc";
        let mut fact = Fact::zeroed();
        fact.account_type = AccountType::Fact.as_u8();
        fact.oracle = oracle_bytes.into();
        fact.content_hash = content_hash;
        fact.uri_len = fact_uri.len() as u16;
        fact.uri[..fact_uri.len()].copy_from_slice(fact_uri.as_bytes());
        fact.agreed = 1;

        let owner = MockRpc::program_owner();
        let rpc = MockRpc::new()
            .with(
                "getAccountInfo",
                json!({
                    "context": { "slot": 1 },
                    "value": {
                        "data": [MockRpc::base64(&meta_bytes), "base64"],
                        "owner": owner,
                        "lamports": 1u64, "executable": false, "rentEpoch": 0u64,
                        "space": meta_bytes.len(),
                    }
                }),
            )
            .with(
                "getProgramAccounts",
                json!([{
                    "pubkey": "Fact111111111111111111111111111111111111111",
                    "account": {
                        "data": [MockRpc::base64(bytemuck::bytes_of(&fact)), "base64"],
                        "owner": owner,
                        "lamports": 1u64, "executable": false, "rentEpoch": 0u64,
                        "space": Fact::LEN,
                    }
                }]),
            );

        // The fetcher serves BOTH the metadata JSON (verified vs uri_hash) and the
        // fact content.
        let fetcher = MockFactFetcher::new()
            .with(meta_uri, meta_json.as_bytes().to_vec())
            .with(fact_uri, fact_content.to_vec());

        let config = build_config_from_chain(&rpc, &fetcher, oracle_pk)
            .await
            .unwrap();
        assert!(config.interpretation.contains("Resolve YES"));
        assert_eq!(config.options_count, 2);
        assert_eq!(config.option_labels.as_ref().unwrap().len(), 2);
        assert_eq!(config.facts.len(), 1);
        assert_eq!(config.facts[0].content_hash, sha256_hex(fact_content));
        assert_eq!(config.oracle.as_deref(), Some(oracle_pk));

        // And it drives the existing pipeline (fact fetch + mock provider).
        let provider = MockProvider::new(0, r#"{"option_index":0}"#, "mock-claude");
        let out = run_core(&config, build_model_config(None, None), &fetcher, &provider)
            .await
            .unwrap();
        assert_eq!(out.option_index, 0);
    }

    #[tokio::test]
    async fn build_config_from_chain_rejects_json_hash_mismatch() {
        use crate::fetch::MockFactFetcher;
        use crate::rpc::MockRpc;
        use serde_json::json;

        let oracle_pk = "So11111111111111111111111111111111111111112";
        let oracle_bytes: [u8; 32] = bs58::decode(oracle_pk)
            .into_vec()
            .unwrap()
            .try_into()
            .unwrap();
        let meta_uri = "https://meta.example/oracle.json";
        // uri_hash commits to THIS json...
        let committed = r#"{"promptTemplate":"the real rules"}"#;
        let uri_hash: [u8; 32] = Sha256::digest(committed.as_bytes()).into();
        let meta_bytes =
            oracle_meta_account_bytes(oracle_bytes, "Q?", &["Yes", "No"], meta_uri, uri_hash);

        let rpc = MockRpc::new().with(
            "getAccountInfo",
            json!({
                "context": { "slot": 1 },
                "value": {
                    "data": [MockRpc::base64(&meta_bytes), "base64"],
                    "owner": MockRpc::program_owner(),
                    "lamports": 1u64, "executable": false, "rentEpoch": 0u64,
                    "space": meta_bytes.len(),
                }
            }),
        );
        // ...but the host serves DIFFERENT json → rejected.
        let fetcher = MockFactFetcher::new()
            .with(meta_uri, br#"{"promptTemplate":"tampered rules"}"#.to_vec());

        let err = build_config_from_chain(&rpc, &fetcher, oracle_pk)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("uri_hash"), "{err}");
    }

    fn parse_payload(hex: &str) -> Vec<u8> {
        (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    // --- --submit keeper mode (offline) -------------------------------------

    use crate::rpc::{JsonRpc, RpcError};
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    use serde_json::json;
    use solana_hash::Hash;
    use solana_transaction::Transaction;
    use std::time::Duration;

    fn common_args_empty() -> CommonArgs {
        CommonArgs {
            config: None,
            oracle: None,
            rpc_url: None,
            prompt_file: None,
            mock: true,
            model: None,
            max_tokens: None,
        }
    }

    fn fast_confirm() -> ConfirmOptions {
        ConfirmOptions {
            max_polls: 2,
            poll_interval: Duration::from_millis(0),
            require_finalized: false,
        }
    }

    /// A [`JsonRpc`] that CAPTURES the base64 tx handed to `sendTransaction` so
    /// the test can decode it and prove the submitted instruction data carries
    /// the runner's OWN payload verbatim. Serves a canned blockhash + a
    /// `confirmed` status.
    struct CapturingRpc {
        sent_tx: std::sync::Mutex<Option<String>>,
        signature: String,
    }

    #[async_trait::async_trait]
    impl JsonRpc for CapturingRpc {
        async fn call(
            &self,
            method: &str,
            params: serde_json::Value,
        ) -> Result<serde_json::Value, RpcError> {
            match method {
                "getLatestBlockhash" => Ok(json!({
                    "context": { "slot": 1 },
                    "value": {
                        "blockhash": Hash::new_from_array([7u8; 32]).to_string(),
                        "lastValidBlockHeight": 100
                    }
                })),
                "sendTransaction" => {
                    let tx = params
                        .get(0)
                        .and_then(|v| v.as_str())
                        .expect("sendTransaction param[0] is the base64 tx")
                        .to_string();
                    *self.sent_tx.lock().unwrap() = Some(tx);
                    Ok(json!(self.signature))
                }
                "getSignatureStatuses" => Ok(json!({
                    "context": { "slot": 2 },
                    "value": [ { "slot": 2, "confirmations": null, "err": null, "confirmationStatus": "confirmed" } ]
                })),
                other => Err(RpcError::Malformed {
                    method: other.to_string(),
                    detail: "unexpected method".to_string(),
                }),
            }
        }
    }

    /// Full keeper flow (fetch/config → claim → submit) against a capturing
    /// MockRpc: the SUBMITTED tx must carry the RunOutput's own payload, the
    /// proposer PDA must be derived from `[b"proposer", oracle, authority]`, and
    /// the confirmed signature is reported.
    #[tokio::test]
    async fn keeper_submit_carries_runoutput_payload_and_reports_signature() {
        // Produce a real RunOutput (and its payload) via the offline pipeline.
        let content = b"BTC closed at $98,000.";
        let uri = "https://facts.example/btc";
        let config = sample_config(uri, content);
        let fetcher = MockFactFetcher::new().with(uri, content.to_vec());
        let provider = MockProvider::new(1, r#"{"option_index":1}"#, "mock-claude");
        let out = run_core(&config, build_model_config(None, None), &fetcher, &provider)
            .await
            .unwrap();

        let oracle = Pubkey::new_from_array([3u8; 32]);
        let authority = Keypair::new();
        let sig = "S".repeat(64);
        let rpc = CapturingRpc {
            sent_tx: std::sync::Mutex::new(None),
            signature: sig.clone(),
        };

        let submission = submit_claim(
            &rpc,
            &oracle,
            &authority,
            &out.submit_ai_claim_payload,
            fast_confirm(),
        )
        .await
        .unwrap();

        assert_eq!(submission.signature, sig);
        assert_eq!(submission.confirmation_status, "confirmed");
        assert_eq!(submission.oracle, oracle.to_string());
        // Proposer PDA is DERIVED from [b"proposer", oracle, authority].
        assert_eq!(
            submission.proposer,
            derive_proposer_pda(&oracle, &authority.pubkey()).to_string()
        );
        assert_eq!(submission.authority, authority.pubkey().to_string());

        // Decode the ACTUAL submitted tx: instruction data == [disc=3] ++ the
        // RunOutput payload (the reuse guarantee, end-to-end).
        let b64 = rpc.sent_tx.lock().unwrap().clone().unwrap();
        let bytes = BASE64.decode(&b64).unwrap();
        let tx: Transaction = bincode::deserialize(&bytes).unwrap();
        let ix = &tx.message.instructions[0];
        assert_eq!(ix.data[0], 3);
        assert_eq!(&ix.data[1..], &out.submit_ai_claim_payload[..]);
    }

    #[test]
    fn submit_off_yields_no_target() {
        let common = common_args_empty();
        let config = sample_config("https://x/y", b"z");
        assert!(resolve_submit_target(&common, false, None, &config)
            .unwrap()
            .is_none());
    }

    #[test]
    fn submit_requires_keypair() {
        let common = common_args_empty();
        let config = sample_config("https://x/y", b"z");
        let err = resolve_submit_target(&common, true, None, &config).unwrap_err();
        assert!(err.to_string().contains("--keypair"), "{err}");
    }

    #[test]
    fn submit_explicit_mode_requires_rpc_url() {
        // Explicit-config mode (no --oracle / --rpc-url), keypair provided → the
        // missing --rpc-url must be surfaced.
        let common = common_args_empty();
        let config = sample_config("https://x/y", b"z");
        let err = resolve_submit_target(&common, true, Some(Path::new("/tmp/kp.json")), &config)
            .unwrap_err();
        assert!(err.to_string().contains("--rpc-url"), "{err}");
    }

    #[test]
    fn submit_needs_an_oracle() {
        let mut common = common_args_empty();
        common.rpc_url = Some("http://localhost:8899".to_string());
        let config = sample_config("https://x/y", b"z"); // config.oracle == None
        let err = resolve_submit_target(&common, true, Some(Path::new("/tmp/kp.json")), &config)
            .unwrap_err();
        assert!(err.to_string().contains("oracle"), "{err}");
    }

    #[test]
    fn submit_oracle_resolved_from_config() {
        let mut common = common_args_empty();
        common.rpc_url = Some("http://localhost:8899".to_string());
        let mut config = sample_config("https://x/y", b"z");
        let oracle_pk = "So11111111111111111111111111111111111111112";
        config.oracle = Some(oracle_pk.to_string());
        let target = resolve_submit_target(&common, true, Some(Path::new("/tmp/kp.json")), &config)
            .unwrap()
            .unwrap();
        assert_eq!(target.oracle.to_string(), oracle_pk);
        assert_eq!(target.rpc_url, "http://localhost:8899");
    }
}

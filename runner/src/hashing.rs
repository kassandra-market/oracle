//! Canonical claim-metadata hashing — THE off-chain protocol contract.
//!
//! The on-chain program stores three opaque 32-byte commitments in `AiClaim`
//! (`model_id`, `params_hash`, `io_hash`) plus a 1-byte categorical `option`.
//! It does NOT compute them. This module defines the canonical, byte-exact
//! scheme that a proposer's runner uses to produce them AND that a challenger's
//! independent re-run must follow to reproduce them. The full byte layout is
//! documented in `runner/HASHING.md` (and mirrored in the rustdoc below); that
//! document is the protocol spec.
//!
//! # Determinism is the whole point
//!
//! Every preimage byte is defined deterministically: there is no map iteration,
//! no float formatting, no locale, no timestamps, no platform-dependent integer
//! widths. All integers are fixed-width **big-endian**; all strings are their
//! verbatim UTF-8 bytes with an explicit **4-byte big-endian length prefix** so
//! adjacent fields can never collide (e.g. `"a"+"bc"` cannot alias `"ab"+"c"`).
//! A third party with only this spec + the same inputs reproduces identical
//! 32-byte hashes in any language.
//!
//! # The three hashes
//!
//! 1. **`model_id`** = `sha256(model_id_string_utf8)`. The model string is the
//!    resolved pinned model identifier ([`crate::provider::ModelConfig::model_id`],
//!    as echoed back in [`crate::provider::CompletionResponse::model_id`]) —
//!    e.g. `"claude-opus-4-8"`. See [`hash_model_id`].
//!
//! 2. **`params_hash`** = `sha256(canonical_params_bytes)`, a fixed-field-order
//!    length-prefixed serialization of every config input that affects the
//!    answer: the prompt-assembly version, provider id, model string, thinking
//!    mode, output-schema id + version, and `max_tokens`. See [`hash_params`]
//!    and [`CanonicalParams`].
//!
//! 3. **`io_hash`** = `sha256(len(system)‖system ‖ len(user)‖user ‖ raw_response)`
//!    — a COMMITMENT to the exact (assembled input, verbatim raw response) the
//!    submitter used. See [`hash_io`].
//!
//! Note `option` is NOT hashed: it is a separate plaintext byte in the payload.

use sha2::{Digest, Sha256};

use crate::constants::{
    IO_HASH_LEN, MODEL_ID_LEN, OPTION_LEN, PARAMS_HASH_LEN, SUBMIT_AI_CLAIM_PAYLOAD_LEN,
};
use crate::provider::{CompletionRequest, CompletionResponse, ModelConfig};

/// Version of the runner's prompt-assembly contract (Task R2). It is folded
/// into `params_hash` so that if the assembly of `system`/`user` ever changes,
/// claims produced by different assembly versions hash differently. **Bump this
/// whenever R2's prompt assembly changes in a way that affects the model
/// input.**
pub const PROMPT_ASSEMBLY_VERSION: u32 = 1;

/// Stable identifier of the structured-output schema the runner forces the
/// model to answer in (the categorical `{ "option_index": <int> }` shape). Part
/// of `params_hash` so a different answer schema hashes differently.
pub const OUTPUT_SCHEMA_ID: &str = "kassandra.categorical_option_index";

/// Version of [`OUTPUT_SCHEMA_ID`]. Bump when the schema's shape changes.
pub const OUTPUT_SCHEMA_VERSION: u32 = 1;

/// Append a string as `u32be(len) ++ utf8_bytes`.
///
/// The 4-byte big-endian length prefix makes every string field
/// self-delimiting, so two adjacent strings can never be confused with a
/// different split of the same concatenation. (Strings longer than `u32::MAX`
/// bytes — ~4 GiB — are out of scope; the cast would wrap, which the protocol
/// does not support.)
fn put_str(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(&(s.len() as u32).to_be_bytes());
    buf.extend_from_slice(s.as_bytes());
}

/// Append an `Option<&str>` as a 1-byte presence tag (`0x00` = none, `0x01` =
/// some) followed, when present, by the length-prefixed string.
fn put_opt_str(buf: &mut Vec<u8>, s: Option<&str>) {
    match s {
        None => buf.push(0u8),
        Some(s) => {
            buf.push(1u8);
            put_str(buf, s);
        }
    }
}

/// `sha256` over `bytes`. The return type is tied to the R0 payload width
/// constant: this only compiles while `MODEL_ID_LEN == 32`, so a drift in the
/// pinned width breaks the build rather than silently producing a wrong hash.
fn sha256(bytes: &[u8]) -> [u8; MODEL_ID_LEN] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

/// The deterministic preimage for `params_hash`: every config input that
/// affects the categorical answer, in a fixed field order.
///
/// Use [`CanonicalParams::from_config`] to build one from a resolved
/// [`ModelConfig`] with the runner's own schema/assembly version constants
/// filled in. The struct is exposed (and the versions are public fields) so a
/// challenger — or a sensitivity test — can construct any variant explicitly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CanonicalParams<'a> {
    /// The runner's prompt-assembly version ([`PROMPT_ASSEMBLY_VERSION`]).
    pub prompt_assembly_version: u32,
    /// Provider identifier (e.g. `"anthropic"`).
    pub provider: &'a str,
    /// Resolved model identifier string (e.g. `"claude-opus-4-8"`).
    pub model_id: &'a str,
    /// Thinking mode declared to the provider (e.g. `Some("adaptive")`).
    pub thinking: Option<&'a str>,
    /// Output-schema identifier ([`OUTPUT_SCHEMA_ID`]).
    pub output_schema_id: &'a str,
    /// Output-schema version ([`OUTPUT_SCHEMA_VERSION`]).
    pub output_schema_version: u32,
    /// Upper bound on generated tokens.
    pub max_tokens: u32,
}

impl<'a> CanonicalParams<'a> {
    /// Build the canonical params from a resolved [`ModelConfig`], filling the
    /// output-schema id/version and prompt-assembly version from the runner's
    /// own constants. This is the production path; both the proposer and a
    /// challenger running the same runner version produce identical bytes.
    pub fn from_config(config: &'a ModelConfig) -> Self {
        Self {
            prompt_assembly_version: PROMPT_ASSEMBLY_VERSION,
            provider: &config.provider,
            model_id: &config.model_id,
            thinking: config.thinking.as_deref(),
            output_schema_id: OUTPUT_SCHEMA_ID,
            output_schema_version: OUTPUT_SCHEMA_VERSION,
            max_tokens: config.max_tokens,
        }
    }

    /// The exact canonical preimage bytes hashed into `params_hash`.
    ///
    /// Fixed field order (THIS ordering IS the spec — never reorder):
    /// 1. `prompt_assembly_version` — `u32` big-endian
    /// 2. `provider`               — `u32be(len) ++ utf8`
    /// 3. `model_id`               — `u32be(len) ++ utf8`
    /// 4. `thinking`               — `0x00` | `0x01 ++ u32be(len) ++ utf8`
    /// 5. `output_schema_id`       — `u32be(len) ++ utf8`
    /// 6. `output_schema_version`  — `u32` big-endian
    /// 7. `max_tokens`             — `u32` big-endian
    pub fn to_canonical_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.prompt_assembly_version.to_be_bytes());
        put_str(&mut buf, self.provider);
        put_str(&mut buf, self.model_id);
        put_opt_str(&mut buf, self.thinking);
        put_str(&mut buf, self.output_schema_id);
        buf.extend_from_slice(&self.output_schema_version.to_be_bytes());
        buf.extend_from_slice(&self.max_tokens.to_be_bytes());
        buf
    }
}

/// `model_id = sha256(model_id_string_utf8)`.
///
/// The preimage is the verbatim UTF-8 bytes of the resolved model identifier
/// string — nothing else (no length prefix, no separators).
pub fn hash_model_id(model_id_string: &str) -> [u8; MODEL_ID_LEN] {
    sha256(model_id_string.as_bytes())
}

/// `params_hash = sha256(params.to_canonical_bytes())`.
///
/// See [`CanonicalParams::to_canonical_bytes`] for the exact preimage layout.
pub fn hash_params(params: &CanonicalParams) -> [u8; PARAMS_HASH_LEN] {
    sha256(&params.to_canonical_bytes())
}

/// `io_hash = sha256( u32be(len(system)) ++ system ++ u32be(len(user)) ++ user
/// ++ raw_response )`.
///
/// `system` and `user` are the exact assembled model input strings the
/// submitter sent; `raw_response` is the model's verbatim response text (the
/// structured-output JSON string, byte-for-byte as returned). `system` and
/// `user` are length-prefixed so the boundary between them is unambiguous;
/// `raw_response` is appended verbatim and consumes the remainder, so no
/// trailing length prefix is needed. The result commits to the EXACT
/// (input, output) pair the submitter used.
pub fn hash_io(system: &str, user: &str, raw_response: &str) -> [u8; IO_HASH_LEN] {
    let mut buf = Vec::new();
    put_str(&mut buf, system);
    put_str(&mut buf, user);
    buf.extend_from_slice(raw_response.as_bytes());
    sha256(&buf)
}

/// The three canonical claim hashes. `option` is deliberately NOT a field here
/// — it is a separate plaintext byte supplied to [`ClaimMetadata::to_payload`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClaimMetadata {
    /// `sha256(model_id_string_utf8)`.
    pub model_id: [u8; MODEL_ID_LEN],
    /// `sha256(canonical_params_bytes)`.
    pub params_hash: [u8; PARAMS_HASH_LEN],
    /// `sha256(canonical_input ++ raw_response)`.
    pub io_hash: [u8; IO_HASH_LEN],
}

impl ClaimMetadata {
    /// Compute all three hashes from the request the submitter sent and the
    /// response they received. The model string + params come from the
    /// RESOLVED values in the response (what actually answered); the I/O
    /// commitment covers the request's `system`/`user` + the verbatim
    /// `raw_response`.
    pub fn compute(req: &CompletionRequest, resp: &CompletionResponse) -> Self {
        Self {
            model_id: hash_model_id(&resp.model_id),
            params_hash: hash_params(&CanonicalParams::from_config(&resp.params)),
            io_hash: hash_io(&req.system, &req.user, &resp.raw_response),
        }
    }

    /// Assemble the exact `submit_ai_claim` instruction payload (after the
    /// 1-byte discriminant): `model_id[32] ++ params_hash[32] ++ io_hash[32] ++
    /// option[1]` = 97 bytes. Offsets and widths are tied to the R0 payload
    /// constants, not loose literals.
    pub fn to_payload(&self, option: u8) -> [u8; SUBMIT_AI_CLAIM_PAYLOAD_LEN] {
        let mut out = [0u8; SUBMIT_AI_CLAIM_PAYLOAD_LEN];
        let model_end = MODEL_ID_LEN;
        let params_end = model_end + PARAMS_HASH_LEN;
        let io_end = params_end + IO_HASH_LEN;
        out[..model_end].copy_from_slice(&self.model_id);
        out[model_end..params_end].copy_from_slice(&self.params_hash);
        out[params_end..io_end].copy_from_slice(&self.io_hash);
        // OPTION_LEN == 1: the option is the single trailing byte.
        debug_assert_eq!(OPTION_LEN, 1);
        out[io_end] = option;
        out
    }
}

/// Lowercase hex-encode bytes (no `0x` prefix) — the canonical string form for
/// the runner's hashes and payloads. Shared by cli / fetch / prompt / rpc.
pub fn to_hex(bytes: &[u8]) -> String {
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{CategoricalOptions, ModelConfig};


    fn sample_config() -> ModelConfig {
        ModelConfig {
            model_id: "claude-opus-4-8".to_string(),
            provider: "anthropic".to_string(),
            max_tokens: 1024,
            thinking: Some("adaptive".to_string()),
        }
    }

    fn sample_request() -> CompletionRequest {
        CompletionRequest {
            system: "Decide the outcome per the interpretation.".to_string(),
            user: "Facts: ...\nOptions:\n0) yes\n1) no\nChoose exactly one.".to_string(),
            options: CategoricalOptions {
                count: 2,
                labels: None,
            },
            config: sample_config(),
        }
    }

    fn sample_response() -> CompletionResponse {
        CompletionResponse {
            option_index: 1,
            raw_response: r#"{"option_index":1}"#.to_string(),
            model_id: "claude-opus-4-8".to_string(),
            params: sample_config(),
        }
    }

    // --- known-answer / regression anchors ---------------------------------
    // Computed once from the fixed inputs above and pinned. A change to ANY
    // input (or to the encoding) flips the corresponding anchor, which is the
    // signal a challenger would use to detect a divergent runner.

    #[test]
    fn model_id_known_answer() {
        // sha256("claude-opus-4-8"), cross-checked with `shasum -a 256`.
        assert_eq!(
            hex::encode(hash_model_id("claude-opus-4-8")),
            "47a46a22f0c9fb105db3f0d8bda83ad51bd59369ab8c8c30cc32ba6356ac5a4a"
        );
    }

    #[test]
    fn params_hash_known_answer() {
        let config = sample_config();
        let params = CanonicalParams::from_config(&config);
        assert_eq!(
            hex::encode(hash_params(&params)),
            "a08e048d8f780ebcc8122268ee6f2e796e8176632b817f3874d8dc4fc405f9c4"
        );
    }

    #[test]
    fn io_hash_known_answer() {
        let io = hash_io(
            "Decide the outcome per the interpretation.",
            "Facts: ...\nOptions:\n0) yes\n1) no\nChoose exactly one.",
            r#"{"option_index":1}"#,
        );
        assert_eq!(
            hex::encode(io),
            "e24990bd43a9d570ea938da194cb7323cb9b1df388211a48f7abaf37479d87c7"
        );
    }

    // --- determinism --------------------------------------------------------

    #[test]
    fn hashes_are_deterministic_across_runs() {
        let req = sample_request();
        let resp = sample_response();
        let a = ClaimMetadata::compute(&req, &resp);
        let b = ClaimMetadata::compute(&req, &resp);
        assert_eq!(a, b);
    }

    // --- sensitivity --------------------------------------------------------

    #[test]
    fn changing_model_string_flips_model_id_and_params() {
        let base = sample_config();
        let mut other = base.clone();
        other.model_id = "claude-opus-4-9".to_string();

        assert_ne!(
            hash_model_id(&base.model_id),
            hash_model_id(&other.model_id),
            "model string must flip model_id"
        );
        assert_ne!(
            hash_params(&CanonicalParams::from_config(&base)),
            hash_params(&CanonicalParams::from_config(&other)),
            "model string must flip params_hash"
        );
    }

    #[test]
    fn changing_max_tokens_flips_params() {
        let base = sample_config();
        let mut other = base.clone();
        other.max_tokens = 2048;
        assert_ne!(
            hash_params(&CanonicalParams::from_config(&base)),
            hash_params(&CanonicalParams::from_config(&other)),
        );
    }

    #[test]
    fn changing_thinking_flips_params() {
        let base = sample_config();
        let mut none = base.clone();
        none.thinking = None;
        let mut other = base.clone();
        other.thinking = Some("extended".to_string());
        let h_base = hash_params(&CanonicalParams::from_config(&base));
        let h_none = hash_params(&CanonicalParams::from_config(&none));
        let h_other = hash_params(&CanonicalParams::from_config(&other));
        assert_ne!(h_base, h_none);
        assert_ne!(h_base, h_other);
        assert_ne!(h_none, h_other);
    }

    #[test]
    fn changing_provider_flips_params() {
        let base = sample_config();
        let mut other = base.clone();
        other.provider = "mock".to_string();
        assert_ne!(
            hash_params(&CanonicalParams::from_config(&base)),
            hash_params(&CanonicalParams::from_config(&other)),
        );
    }

    #[test]
    fn changing_schema_version_flips_params() {
        let config = sample_config();
        let base = CanonicalParams::from_config(&config);
        let mut other = base;
        other.output_schema_version = base.output_schema_version + 1;
        assert_ne!(hash_params(&base), hash_params(&other));
    }

    #[test]
    fn changing_schema_id_flips_params() {
        let config = sample_config();
        let base = CanonicalParams::from_config(&config);
        let mut other = base;
        other.output_schema_id = "kassandra.something_else";
        assert_ne!(hash_params(&base), hash_params(&other));
    }

    #[test]
    fn changing_assembly_version_flips_params() {
        let config = sample_config();
        let base = CanonicalParams::from_config(&config);
        let mut other = base;
        other.prompt_assembly_version = base.prompt_assembly_version + 1;
        assert_ne!(hash_params(&base), hash_params(&other));
    }

    #[test]
    fn changing_input_or_response_flips_io() {
        let base = hash_io("sys", "usr", "resp");
        assert_ne!(base, hash_io("SYS", "usr", "resp"), "system flips io_hash");
        assert_ne!(base, hash_io("sys", "USR", "resp"), "user flips io_hash");
        assert_ne!(
            base,
            hash_io("sys", "usr", "RESP"),
            "raw_response flips io_hash"
        );
    }

    #[test]
    fn changing_only_option_does_not_change_any_hash() {
        let req = sample_request();
        let resp = sample_response();
        let meta = ClaimMetadata::compute(&req, &resp);
        // The option is a plaintext payload byte; the three hashes are
        // independent of it.
        let p0 = meta.to_payload(0);
        let p1 = meta.to_payload(7);
        assert_eq!(
            &p0[..96],
            &p1[..96],
            "only the trailing option byte differs"
        );
        assert_eq!(p0[96], 0);
        assert_eq!(p1[96], 7);
    }

    // --- collision resistance of the joins ----------------------------------

    #[test]
    fn io_join_is_unambiguous() {
        // "a"+"bc" must not collide with "ab"+"c".
        assert_ne!(
            hash_io("a", "bc", "x"),
            hash_io("ab", "c", "x"),
            "length-prefixed system/user join must be collision-free"
        );
        // The system/user boundary must also be distinct from the response
        // boundary: moving bytes from user into raw_response changes the hash.
        assert_ne!(hash_io("s", "uv", ""), hash_io("s", "u", "v"));
    }

    #[test]
    fn params_join_is_unambiguous() {
        // Moving a character across the provider/model boundary must flip the
        // hash even though the naive concatenation "anthropic"+"claude" is the
        // same total string.
        let config = sample_config();
        let a = CanonicalParams {
            provider: "anthropicX",
            model_id: "claude-opus-4-8",
            ..CanonicalParams::from_config(&config)
        };
        let b = CanonicalParams {
            provider: "anthropic",
            model_id: "Xclaude-opus-4-8",
            ..CanonicalParams::from_config(&config)
        };
        assert_ne!(hash_params(&a), hash_params(&b));
    }

    // --- width / payload layout (tied to R0 constants) ----------------------

    #[test]
    fn hashes_are_32_bytes() {
        let meta = ClaimMetadata::compute(&sample_request(), &sample_response());
        assert_eq!(meta.model_id.len(), MODEL_ID_LEN);
        assert_eq!(meta.params_hash.len(), PARAMS_HASH_LEN);
        assert_eq!(meta.io_hash.len(), IO_HASH_LEN);
        assert_eq!(MODEL_ID_LEN, 32);
    }

    #[test]
    fn payload_has_hashes_at_expected_offsets() {
        let meta = ClaimMetadata::compute(&sample_request(), &sample_response());
        let payload = meta.to_payload(3);
        assert_eq!(payload.len(), SUBMIT_AI_CLAIM_PAYLOAD_LEN);
        assert_eq!(payload.len(), 97);
        assert_eq!(&payload[0..32], &meta.model_id);
        assert_eq!(&payload[32..64], &meta.params_hash);
        assert_eq!(&payload[64..96], &meta.io_hash);
        assert_eq!(payload[96], 3);
    }
}

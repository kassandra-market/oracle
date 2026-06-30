//! Deterministic prompt assembly + categorical answer parsing (Task R2).
//!
//! This module turns an oracle's resolution rules + the agreed fact set + the
//! categorical options into the **canonical** `system` / `user` strings of a
//! [`CompletionRequest`], defines the **structured-output JSON schema** the real
//! provider (Task R4) forces the model to answer in, and **parses** the model's
//! structured response back into a validated `option_index`.
//!
//! # Why byte-determinism matters here
//!
//! The assembled `system` / `user` strings are committed on-chain via `io_hash`
//! (see [`crate::hashing`] / `runner/HASHING.md`): `io_hash` hashes the EXACT
//! `system` / `user` bytes this module produces. A challenger who assembles from
//! the same inputs MUST produce byte-identical strings, or their `io_hash` will
//! not match and the protocol breaks. Therefore the assembly is fully
//! deterministic:
//!
//! - **Fact ordering is canonical**: facts are sorted by their 32-byte
//!   `content_hash` ascending (lexicographic), so the rendered order is
//!   independent of the caller's input order.
//! - **Fixed separators**: blocks are joined by exactly `"\n\n"`, option lines
//!   by exactly `"\n"`; there is no trailing whitespace and no trailing newline.
//! - **No nondeterminism sources**: no map iteration, no floats, no locale, no
//!   timestamps. Integers are rendered in base-10 with `{}` (locale-independent
//!   for integers). Verified fact content is rendered **verbatim** (it is what
//!   `content_hash` commits to — trimming would diverge from the on-chain hash).
//!
//! # Versioning
//!
//! This file defines **prompt-assembly version 1**, pinned by
//! [`crate::hashing::PROMPT_ASSEMBLY_VERSION`] (re-exported here as
//! [`PROMPT_ASSEMBLY_VERSION`]), which is folded into `params_hash`. **Any change
//! to the assembled bytes — the preamble text, section headers, separators, fact
//! rendering, option enumeration, or the answer instruction — MUST bump that
//! constant**, so claims produced by different assembly versions never collide.
//! The [`assembly_regression_anchor`](tests) test pins the exact assembled output
//! of a fixed input so an accidental format change fails the build instead of
//! silently shipping with the wrong version.
//!
//! # The structured-output schema
//!
//! [`output_schema`] returns the JSON Schema that forces the model to answer
//! `{ "option_index": <integer in [0, count)> }`. Its stable identity is pinned
//! by [`crate::hashing::OUTPUT_SCHEMA_ID`] / [`crate::hashing::OUTPUT_SCHEMA_VERSION`]
//! (also folded into `params_hash`); the only input-dependent part is the
//! `maximum` bound, derived from `options_count` (itself committed on-chain).
//!
//! # Parsing policy
//!
//! [`parse_option_index`] is **lenient about extra fields** (it reads only
//! `option_index` and ignores any others — though the schema's
//! `additionalProperties: false` prevents extras from a compliant provider) but
//! **strict about the value**: it rejects missing / non-object / non-integer /
//! negative / out-of-range values with a clear [`ParseError`].

use serde_json::Value;

use crate::provider::{CategoricalOptions, CompletionRequest, ModelConfig};

/// Re-export of the assembly version constant (lives in [`crate::hashing`] so it
/// can feed `params_hash`). This module's format IS that version's contract;
/// changing the format requires bumping the constant there.
pub use crate::hashing::PROMPT_ASSEMBLY_VERSION;

/// The fixed system preamble that frames the task. Part of the canonical
/// assembly (version [`PROMPT_ASSEMBLY_VERSION`]) — changing it changes the
/// hashed bytes and requires a version bump.
pub const SYSTEM_PREAMBLE: &str = "You are an impartial oracle resolver for a categorical prediction market. \
Your task is to determine the single correct outcome by applying the resolution rules to the provided facts. \
Decide based ONLY on the resolution rules and the facts given to you; do not use outside knowledge, assumptions, or information not present below. \
You must choose exactly one option by its integer index.";

/// An agreed fact whose `content` has already been fetched and verified against
/// its on-chain `content_hash` (Task R3 does the fetch + verification; this
/// module accepts the already-verified pair).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fact {
    /// The on-chain `content_hash` (sha256 of `content`), 32 bytes. Doubles as
    /// the canonical sort key for deterministic ordering.
    pub content_hash: [u8; 32],
    /// The verified fact content, rendered verbatim into the prompt.
    pub content: String,
}

impl Fact {
    /// Convenience constructor.
    pub fn new(content_hash: [u8; 32], content: impl Into<String>) -> Self {
        Self {
            content_hash,
            content: content.into(),
        }
    }
}

/// The canonical assembled model input: the exact `system` / `user` strings that
/// feed a [`CompletionRequest`] and (via [`crate::hashing::hash_io`]) `io_hash`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssembledPrompt {
    /// System text: the fixed preamble + the oracle's resolution rules.
    pub system: String,
    /// User text: the canonically-ordered facts + enumerated options + the
    /// answer instruction.
    pub user: String,
}

/// Lowercase hex of a byte slice (used to render a fact's `content_hash`).
/// Deterministic, allocation-light, no locale.
fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

/// Look up the label for option index `i`, if any. Labels are matched by their
/// explicit `index` field, independent of their position in the vec.
fn label_for(options: &CategoricalOptions, i: u8) -> Option<&str> {
    options
        .labels
        .as_ref()
        .and_then(|ls| ls.iter().find(|o| o.index == i))
        .and_then(|o| o.label.as_deref())
}

/// Assemble the canonical `system` / `user` strings.
///
/// `interpretation` is the oracle's resolution-rule text (committed on-chain via
/// `prompt_hash`). `facts` are the already-verified `(content_hash, content)`
/// pairs in ANY order — they are sorted canonically here. `options` is the
/// categorical answer space.
///
/// ## `system` layout
///
/// ```text
/// {SYSTEM_PREAMBLE}
///
/// # Resolution rules
///
/// {interpretation}
/// ```
///
/// ## `user` layout
///
/// Three blocks joined by `"\n\n"`, no trailing newline:
///
/// ```text
/// # Facts
///
/// ## Fact 1 (sha256: {hex64})
/// {content}
///
/// ## Fact 2 (sha256: {hex64})
/// {content}
///
/// # Options
///
/// You must choose exactly one of the following options by its integer index:
///
/// [0] {label or "(no label)"}
/// [1] {label or "(no label)"}
///
/// # Answer
///
/// Respond with the structured JSON output { "option_index": <index> }, ...
/// ```
///
/// Facts are sorted by `content_hash` ascending and numbered 1..=N in that
/// order; each is tagged with its `content_hash` hex so the fact set is
/// unambiguous (two distinct fact sets cannot render to the same bytes). If
/// there are no facts, the body is the literal `(no facts provided)`.
pub fn assemble(
    interpretation: &str,
    facts: &[Fact],
    options: &CategoricalOptions,
) -> AssembledPrompt {
    let system = format!("{SYSTEM_PREAMBLE}\n\n# Resolution rules\n\n{interpretation}");

    // Canonical fact order: sort references by content_hash bytes ascending.
    let mut ordered: Vec<&Fact> = facts.iter().collect();
    ordered.sort_by(|a, b| a.content_hash.cmp(&b.content_hash));

    let facts_block = if ordered.is_empty() {
        "# Facts\n\n(no facts provided)".to_string()
    } else {
        let entries: Vec<String> = ordered
            .iter()
            .enumerate()
            .map(|(i, f)| {
                format!(
                    "## Fact {} (sha256: {})\n{}",
                    i + 1,
                    to_hex(&f.content_hash),
                    f.content
                )
            })
            .collect();
        format!("# Facts\n\n{}", entries.join("\n\n"))
    };

    let count = options.count;
    let option_lines: Vec<String> = (0..count)
        .map(|i| match label_for(options, i) {
            Some(label) => format!("[{i}] {label}"),
            None => format!("[{i}] (no label)"),
        })
        .collect();
    let options_block = format!(
        "# Options\n\nYou must choose exactly one of the following options by its integer index:\n\n{}",
        option_lines.join("\n")
    );

    // count is >= 2 on-chain; saturating_sub guards a degenerate count == 0.
    let max_index = count.saturating_sub(1);
    let answer_block = format!(
        "# Answer\n\nRespond with the structured JSON output {{ \"option_index\": <index> }}, \
where <index> is the integer index (0 to {max_index} inclusive) of the single correct option. \
Base your choice ONLY on the resolution rules and the facts above."
    );

    let user = [facts_block, options_block, answer_block].join("\n\n");

    AssembledPrompt { system, user }
}

/// Assemble + wrap into a ready-to-send [`CompletionRequest`] (convenience for
/// Tasks R4/R5). The `config`'s `provider`/`model_id`/`thinking`/`max_tokens`
/// flow through to `params_hash`; the assembled `system`/`user` flow through to
/// `io_hash`.
pub fn build_request(
    interpretation: &str,
    facts: &[Fact],
    options: CategoricalOptions,
    config: ModelConfig,
) -> CompletionRequest {
    let AssembledPrompt { system, user } = assemble(interpretation, facts, &options);
    CompletionRequest {
        system,
        user,
        options,
        config,
    }
}

/// The structured-output JSON Schema forcing `{ "option_index": <integer> }`.
///
/// Stable identity is pinned by [`crate::hashing::OUTPUT_SCHEMA_ID`] /
/// [`crate::hashing::OUTPUT_SCHEMA_VERSION`]. The schema constrains
/// `option_index` to an integer in `[0, options_count)` via `minimum`/`maximum`,
/// requires the field, and forbids any other field (`additionalProperties:
/// false`) so the provider returns a clean, parse-robust answer. The only
/// input-dependent value is `maximum` (= `options_count - 1`), derived from the
/// on-chain `options_count`; the schema's SHAPE is what the version pins.
pub fn output_schema(options_count: u8) -> Value {
    let max_index = options_count.saturating_sub(1);
    serde_json::json!({
        "type": "object",
        "properties": {
            "option_index": {
                "type": "integer",
                "minimum": 0,
                "maximum": max_index
            }
        },
        "required": ["option_index"],
        "additionalProperties": false
    })
}

/// Error parsing a model's structured-output response into an `option_index`.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    /// The raw response was not valid JSON.
    #[error("response is not valid JSON: {0}")]
    InvalidJson(String),
    /// The JSON was valid but not a JSON object.
    #[error("response JSON is not an object")]
    NotAnObject,
    /// The required `option_index` field was absent.
    #[error("response is missing the required `option_index` field")]
    MissingField,
    /// `option_index` was present but not a non-negative integer (it was a
    /// float, string, negative, boolean, etc.).
    #[error("`option_index` must be a non-negative integer, got `{0}`")]
    NotAnUnsignedInteger(String),
    /// `option_index` is a valid integer but outside `[0, options_count)`.
    #[error("`option_index` {got} is out of range: must be 0..{count} ({count} options)")]
    OutOfRange {
        /// The parsed (in-range-for-u64) index that failed the bound check.
        got: u64,
        /// The number of options (`option_index` must be `< count`).
        count: u8,
    },
}

/// Parse a model's raw structured-output JSON string into a validated
/// `option_index`.
///
/// Accepts the verbatim `raw_response` (the JSON the provider returned) and the
/// oracle's `options_count`. Returns the chosen index on success, or a
/// [`ParseError`] if the response is malformed, missing the field, the wrong
/// type, negative, or out of range (`index >= options_count`).
///
/// **Extra-field policy: lenient.** Only `option_index` is read; any additional
/// fields are ignored (a schema-compliant provider sends none thanks to
/// `additionalProperties: false`, but a stray field never breaks parsing).
pub fn parse_option_index(raw_response: &str, options_count: u8) -> Result<u8, ParseError> {
    let value: Value =
        serde_json::from_str(raw_response).map_err(|e| ParseError::InvalidJson(e.to_string()))?;

    let obj = value.as_object().ok_or(ParseError::NotAnObject)?;

    let field = obj.get("option_index").ok_or(ParseError::MissingField)?;

    // `as_u64` is true ONLY for a JSON integer >= 0; it rejects floats (incl.
    // `1.0`), negatives, strings, booleans, and null — exactly what we want.
    let raw = field
        .as_u64()
        .ok_or_else(|| ParseError::NotAnUnsignedInteger(field.to_string()))?;

    if raw >= options_count as u64 {
        return Err(ParseError::OutOfRange {
            got: raw,
            count: options_count,
        });
    }

    // raw < options_count <= u8::MAX, so this never truncates.
    Ok(raw as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::AiProvider;
    use crate::provider::{CategoricalOption, CategoricalOptions, MockProvider};

    fn h(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    fn opts_no_labels(count: u8) -> CategoricalOptions {
        CategoricalOptions {
            count,
            labels: None,
        }
    }

    fn opts_with_labels() -> CategoricalOptions {
        CategoricalOptions {
            count: 2,
            labels: Some(vec![
                CategoricalOption {
                    index: 0,
                    label: Some("Yes".to_string()),
                },
                CategoricalOption {
                    index: 1,
                    label: Some("No".to_string()),
                },
            ]),
        }
    }

    // --- assembly determinism ----------------------------------------------

    #[test]
    fn assembly_is_independent_of_fact_input_order() {
        let interp = "Resolve YES iff the event occurred before the deadline.";
        let opts = opts_with_labels();
        let f_a = Fact::new(h(0x01), "alpha fact");
        let f_b = Fact::new(h(0x02), "beta fact");
        let f_c = Fact::new(h(0x03), "gamma fact");

        let forward = assemble(interp, &[f_a.clone(), f_b.clone(), f_c.clone()], &opts);
        let shuffled = assemble(interp, &[f_c, f_a, f_b], &opts);

        // Different input order -> byte-identical output (proves content_hash sort).
        assert_eq!(forward.system, shuffled.system);
        assert_eq!(forward.user, shuffled.user);
    }

    #[test]
    fn facts_render_in_content_hash_order() {
        let interp = "rules";
        let opts = opts_no_labels(2);
        // Insert out of order; expect rendered order 0x01, 0x05, 0xaa.
        let facts = vec![
            Fact::new(h(0xaa), "third"),
            Fact::new(h(0x01), "first"),
            Fact::new(h(0x05), "second"),
        ];
        let user = assemble(interp, &facts, &opts).user;
        let p_first = user.find("first").unwrap();
        let p_second = user.find("second").unwrap();
        let p_third = user.find("third").unwrap();
        assert!(p_first < p_second && p_second < p_third);
        // Numbered 1..=N in canonical order.
        assert!(user.contains("## Fact 1 (sha256: 0101010101010101010101010101010101010101010101010101010101010101)\nfirst"));
        assert!(user.contains("## Fact 3 (sha256: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa)\nthird"));
    }

    #[test]
    fn options_enumerated_with_labels() {
        let user = assemble("r", &[], &opts_with_labels()).user;
        assert!(user.contains("[0] Yes\n[1] No"));
    }

    #[test]
    fn options_enumerated_without_labels() {
        let user = assemble("r", &[], &opts_no_labels(3)).user;
        assert!(user.contains("[0] (no label)\n[1] (no label)\n[2] (no label)"));
    }

    #[test]
    fn no_facts_renders_placeholder() {
        let user = assemble("r", &[], &opts_no_labels(2)).user;
        assert!(user.contains("# Facts\n\n(no facts provided)"));
    }

    #[test]
    fn system_contains_preamble_then_rules() {
        let a = assemble("MY RULES", &[], &opts_no_labels(2));
        assert_eq!(
            a.system,
            format!("{SYSTEM_PREAMBLE}\n\n# Resolution rules\n\nMY RULES")
        );
    }

    #[test]
    fn no_trailing_newline_or_whitespace() {
        let a = assemble("rules", &[Fact::new(h(1), "c")], &opts_with_labels());
        assert_eq!(
            a.system.trim_end(),
            a.system,
            "system has trailing whitespace"
        );
        assert_eq!(a.user.trim_end(), a.user, "user has trailing whitespace");
    }

    // --- regression anchor: pin the EXACT assembled bytes ------------------
    // A change to ANY part of the format flips these strings. If this test
    // fails, the assembly changed: update the strings AND bump
    // PROMPT_ASSEMBLY_VERSION in hashing.rs (claims would otherwise silently
    // hash differently under the same version).

    #[test]
    fn assembly_regression_anchor() {
        let interp = "Resolve YES if BTC closed above $100k on the date; otherwise NO.";
        let facts = vec![
            // Deliberately out of content_hash order.
            Fact::new(h(0x22), "BTC closed at $98,000."),
            Fact::new(h(0x11), "The date in question is 2025-12-31."),
        ];
        let opts = opts_with_labels();
        let a = assemble(interp, &facts, &opts);

        let expected_system = "You are an impartial oracle resolver for a categorical prediction market. \
Your task is to determine the single correct outcome by applying the resolution rules to the provided facts. \
Decide based ONLY on the resolution rules and the facts given to you; do not use outside knowledge, assumptions, or information not present below. \
You must choose exactly one option by its integer index.\n\n\
# Resolution rules\n\n\
Resolve YES if BTC closed above $100k on the date; otherwise NO.";
        assert_eq!(a.system, expected_system);

        let expected_user = "# Facts\n\n\
## Fact 1 (sha256: 1111111111111111111111111111111111111111111111111111111111111111)\n\
The date in question is 2025-12-31.\n\n\
## Fact 2 (sha256: 2222222222222222222222222222222222222222222222222222222222222222)\n\
BTC closed at $98,000.\n\n\
# Options\n\n\
You must choose exactly one of the following options by its integer index:\n\n\
[0] Yes\n[1] No\n\n\
# Answer\n\n\
Respond with the structured JSON output { \"option_index\": <index> }, \
where <index> is the integer index (0 to 1 inclusive) of the single correct option. \
Base your choice ONLY on the resolution rules and the facts above.";
        assert_eq!(a.user, expected_user);
    }

    // --- output schema ------------------------------------------------------

    #[test]
    fn output_schema_shape() {
        let schema = output_schema(3);
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"]["option_index"]["type"], "integer");
        assert_eq!(schema["properties"]["option_index"]["minimum"], 0);
        assert_eq!(schema["properties"]["option_index"]["maximum"], 2);
        assert_eq!(schema["required"], serde_json::json!(["option_index"]));
        assert_eq!(schema["additionalProperties"], false);
    }

    // --- parsing ------------------------------------------------------------

    #[test]
    fn parse_valid_index() {
        assert_eq!(parse_option_index(r#"{"option_index":1}"#, 2).unwrap(), 1);
        assert_eq!(parse_option_index(r#"{"option_index":0}"#, 2).unwrap(), 0);
    }

    #[test]
    fn parse_extra_fields_are_ignored() {
        // Lenient policy: stray fields don't break parsing.
        let raw = r#"{"option_index":2,"reasoning":"because","confidence":0.9}"#;
        assert_eq!(parse_option_index(raw, 3).unwrap(), 2);
    }

    #[test]
    fn parse_rejects_out_of_range() {
        assert_eq!(
            parse_option_index(r#"{"option_index":2}"#, 2),
            Err(ParseError::OutOfRange { got: 2, count: 2 })
        );
        assert_eq!(
            parse_option_index(r#"{"option_index":255}"#, 3),
            Err(ParseError::OutOfRange { got: 255, count: 3 })
        );
    }

    #[test]
    fn parse_rejects_negative() {
        assert!(matches!(
            parse_option_index(r#"{"option_index":-1}"#, 2),
            Err(ParseError::NotAnUnsignedInteger(_))
        ));
    }

    #[test]
    fn parse_rejects_float() {
        assert!(matches!(
            parse_option_index(r#"{"option_index":1.5}"#, 3),
            Err(ParseError::NotAnUnsignedInteger(_))
        ));
        // Even an integer-valued float is rejected (wrong JSON type).
        assert!(matches!(
            parse_option_index(r#"{"option_index":1.0}"#, 3),
            Err(ParseError::NotAnUnsignedInteger(_))
        ));
    }

    #[test]
    fn parse_rejects_wrong_type() {
        assert!(matches!(
            parse_option_index(r#"{"option_index":"1"}"#, 3),
            Err(ParseError::NotAnUnsignedInteger(_))
        ));
        assert!(matches!(
            parse_option_index(r#"{"option_index":true}"#, 3),
            Err(ParseError::NotAnUnsignedInteger(_))
        ));
        assert!(matches!(
            parse_option_index(r#"{"option_index":null}"#, 3),
            Err(ParseError::NotAnUnsignedInteger(_))
        ));
    }

    #[test]
    fn parse_rejects_missing_field() {
        assert_eq!(
            parse_option_index(r#"{"other":1}"#, 3),
            Err(ParseError::MissingField)
        );
    }

    #[test]
    fn parse_rejects_non_object() {
        assert_eq!(parse_option_index("1", 3), Err(ParseError::NotAnObject));
        assert_eq!(parse_option_index("[1,2]", 3), Err(ParseError::NotAnObject));
    }

    #[test]
    fn parse_rejects_malformed_json() {
        assert!(matches!(
            parse_option_index(r#"{"option_index":}"#, 3),
            Err(ParseError::InvalidJson(_))
        ));
        assert!(matches!(
            parse_option_index("not json", 3),
            Err(ParseError::InvalidJson(_))
        ));
    }

    // --- pipeline composition (assemble -> request -> mock -> parse) --------

    #[tokio::test]
    async fn pipeline_composes_through_mock_provider() {
        let opts = opts_with_labels();
        let req = build_request(
            "Resolve per the rules.",
            &[Fact::new(h(0x01), "fact one")],
            opts,
            ModelConfig {
                model_id: "claude-opus-4-8".to_string(),
                provider: "mock".to_string(),
                max_tokens: 1024,
                thinking: Some("adaptive".to_string()),
            },
        );

        let provider = MockProvider::new(1, r#"{"option_index":1}"#, "mock-claude");
        let resp = provider.complete(&req).await.unwrap();

        // The raw response parses back to the same option index, validated
        // against the request's option count.
        let parsed = parse_option_index(&resp.raw_response, req.options.count).unwrap();
        assert_eq!(parsed, 1);
        assert_eq!(parsed, resp.option_index);
    }
}

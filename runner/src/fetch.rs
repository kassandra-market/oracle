//! Agreed-fact fetching + `content_hash` verification (Task R3).
//!
//! An oracle's agreed facts live on-chain as a `content_hash: [u8; 32]` plus a
//! `uri` (≤200 bytes); the fact *content* itself is off-chain at that `uri`.
//! This module fetches each `uri`, verifies the fetched bytes against the
//! on-chain `content_hash`, and — only on a match — produces the verified
//! [`Fact`] pairs that [`crate::prompt::assemble`] feeds to the model.
//!
//! # The security contract
//!
//! `content_hash` is the on-chain commitment to the exact fact bytes the oracle
//! agreed on. The model's answer (and therefore the on-chain claim) is only
//! sound if the content it reasons over is *exactly* that committed content.
//! So tampered, swapped, or unavailable content is **REJECTED with a clear
//! error — never silently fed to the model**. A mismatch is treated as a hard
//! failure, identical to a fetch failure.
//!
//! # `content_hash` derivation (mirrored from the program)
//!
//! The on-chain program (`programs/oracles/src/processor/submit_fact.rs`)
//! treats `content_hash` as an **opaque, caller-supplied 32-byte value**: it
//! uses it only as a `Fact` PDA seed (`[b"fact", oracle, content_hash]`) and
//! stores it verbatim — it never hashes the content or applies any framing
//! (the program tests submit arbitrary values such as `[0x42; 32]`). The
//! derivation is therefore a purely *off-chain* convention, defined here as the
//! reference for both proposer and challenger:
//!
//! > **`content_hash = sha256(raw fact content bytes)`** — plain SHA-256 over
//! > the exact bytes served at the `uri`, with no length prefix, no domain tag,
//! > and no other framing.
//!
//! Verification recomputes `sha256(body)` over the **raw response bytes** (not
//! the decoded string) and compares to the on-chain `content_hash`.
//!
//! # Encoding policy (non-UTF-8)
//!
//! The verified content is rendered into the prompt as text and is what
//! `content_hash` committed to, so it MUST be valid UTF-8. A body that hashes
//! correctly but is not valid UTF-8 is **rejected** ([`VerifyError::NonUtf8`])
//! rather than lossily decoded — a lossy decode would diverge from the bytes
//! the hash committed to.
//!
//! # Fetcher abstraction (offline tests)
//!
//! Fetching is behind the [`FactFetcher`] trait so the verification logic runs
//! offline in tests. [`HttpFactFetcher`] is the real `reqwest`-based default
//! (http/https only, with a timeout and a body-size cap); [`MockFactFetcher`]
//! is a deterministic, no-network map used by the tests.
//!
//! # Resource limits + SSRF (documented limitations)
//!
//! [`HttpFactFetcher`] caps the response body at [`DEFAULT_MAX_BODY_BYTES`]
//! (overridable via [`HttpFactFetcher::with_max_body_bytes`]): it rejects a
//! declared `Content-Length` over the cap up front, and streams the body
//! chunk-by-chunk, aborting with [`FetchError::TooLarge`] the moment the
//! accumulated size would exceed the cap — so an unbounded or hostile body can't
//! exhaust memory.
//!
//! **SSRF is NOT mitigated here.** The scheme allowlist (http/https) stops
//! `file:`/`data:`/etc., but a fact `uri` may still resolve to an internal /
//! link-local / loopback address (e.g. `http://169.254.169.254/...` or
//! `http://10.0.0.5/...`), and redirects are followed within `reqwest`'s default
//! cap. Treat fact URIs as untrusted: run the runner where it has no privileged
//! network position, or add egress filtering / DNS-pinning at the deployment
//! layer. This is a deliberate, documented limitation for v1.
//!
//! # Batch policy (fail-fast)
//!
//! [`fetch_and_verify_facts`] is **fail-fast**: it returns the first
//! [`VerifyError`] (fetch failure, hash mismatch, or non-UTF-8), naming the
//! offending `uri`. One bad fact invalidates the whole agreed set for a run —
//! there is no partial/best-effort fact set, so collecting further errors would
//! add no value.

use std::time::Duration;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::prompt::Fact;

/// Default HTTP request timeout for [`HttpFactFetcher`].
pub const DEFAULT_FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Default maximum fact-content body size for [`HttpFactFetcher`] (8 MiB). A
/// body exceeding this is rejected with [`FetchError::TooLarge`] so a hostile or
/// unbounded response can't exhaust memory. Fact content is small text, so this
/// is generous.
pub const DEFAULT_MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// An agreed fact as committed on-chain: the 32-byte `content_hash` plus the
/// off-chain `uri` its content is served from. (This mirrors the on-chain
/// `Fact`'s `content_hash` + `uri`; the runner takes it as explicit input.)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FactRef {
    /// The on-chain commitment: `sha256(content_bytes)`.
    pub content_hash: [u8; 32],
    /// The location the fact content is fetched from (http/https).
    pub uri: String,
}

impl FactRef {
    /// Convenience constructor.
    pub fn new(content_hash: [u8; 32], uri: impl Into<String>) -> Self {
        Self {
            content_hash,
            uri: uri.into(),
        }
    }
}

/// A transport-level failure fetching a `uri`. Distinct from a *verification*
/// failure ([`VerifyError`]) — this is "couldn't get the bytes", not "the bytes
/// don't match".
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    /// The `uri`'s scheme is not `http`/`https`. Kept deliberately narrow:
    /// `file:`, `data:`, `ftp:`, etc. are rejected so a fact `uri` can never
    /// pull from the local filesystem or other exotic sources.
    #[error("unsupported URI scheme `{scheme}` for `{uri}` (only http/https are allowed)")]
    UnsupportedScheme {
        /// The offending uri.
        uri: String,
        /// The scheme that was rejected.
        scheme: String,
    },
    /// A transport/network error (DNS, connection, TLS, timeout, malformed
    /// URL). Carries the rendered cause so callers see why.
    #[error("fetch of `{uri}` failed: {message}")]
    Transport {
        /// The uri being fetched.
        uri: String,
        /// The rendered underlying error.
        message: String,
    },
    /// The server responded with a non-success (non-2xx) HTTP status.
    #[error("fetch of `{uri}` returned non-success HTTP status {status}")]
    Status {
        /// The uri being fetched.
        uri: String,
        /// The HTTP status code.
        status: u16,
    },
    /// The response body exceeded the configured size cap (declared
    /// `Content-Length` or streamed bytes). Rejected to bound memory.
    #[error("body of `{uri}` exceeds the {limit}-byte size cap")]
    TooLarge {
        /// The uri whose body was too large.
        uri: String,
        /// The configured cap in bytes.
        limit: usize,
    },
    /// The fetcher has no content for this `uri` (used by the mock; analogous to
    /// a 404 / DNS failure for the real fetcher).
    #[error("no content available for `{uri}`")]
    NotFound {
        /// The uri that was not found.
        uri: String,
    },
}

/// A fact failed verification against its on-chain `content_hash`. Either the
/// bytes couldn't be fetched ([`FetchError`]), they hashed to the wrong value
/// (tampered/wrong content), or they weren't valid UTF-8.
#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    /// The content couldn't be fetched at all.
    #[error(transparent)]
    Fetch(#[from] FetchError),
    /// The fetched bytes did not hash to the on-chain `content_hash`: the
    /// content is tampered, swapped, or otherwise not what the oracle agreed
    /// on. The fact is REJECTED — it is never passed to the model.
    #[error(
        "content_hash mismatch for `{uri}`: expected sha256 {expected}, \
         computed {actual} (content is tampered or does not match the on-chain commitment)"
    )]
    ContentHashMismatch {
        /// The uri whose content failed verification.
        uri: String,
        /// The expected on-chain `content_hash`, hex.
        expected: String,
        /// The sha256 actually computed over the fetched body, hex.
        actual: String,
    },
    /// The bytes hashed correctly but are not valid UTF-8, so they cannot be
    /// used as prompt text. Rejected rather than lossily decoded.
    #[error("content of `{uri}` is not valid UTF-8: {message}")]
    NonUtf8 {
        /// The uri whose content was not valid UTF-8.
        uri: String,
        /// The rendered decode error.
        message: String,
    },
}

/// Fetches raw fact-content bytes for a `uri`. Behind a trait so the
/// verification logic is testable offline (see [`MockFactFetcher`]).
#[async_trait]
pub trait FactFetcher {
    /// Fetch the raw bytes served at `uri`, or a [`FetchError`] naming it.
    async fn fetch(&self, uri: &str) -> Result<Vec<u8>, FetchError>;
}

/// The default `reqwest`-based fetcher: an HTTP(S) `GET` returning the raw
/// response body bytes.
///
/// **Scheme policy:** only `http` and `https` are accepted; any other scheme is
/// rejected up front with [`FetchError::UnsupportedScheme`] (so a fact `uri`
/// can never reach the local filesystem, `data:` blobs, etc.).
///
/// **Status policy:** non-2xx responses are errors ([`FetchError::Status`]).
///
/// **Timeout:** a per-request timeout (default [`DEFAULT_FETCH_TIMEOUT`]) bounds
/// each fetch. Redirects follow `reqwest`'s default policy (capped); nothing
/// exotic is enabled.
///
/// **Body-size cap:** the response body is capped at `max_body_bytes` (default
/// [`DEFAULT_MAX_BODY_BYTES`]) — a declared `Content-Length` over the cap is
/// rejected up front, and the body is streamed chunk-by-chunk and aborted with
/// [`FetchError::TooLarge`] the moment it would exceed the cap.
#[derive(Clone, Debug)]
pub struct HttpFactFetcher {
    client: reqwest::Client,
    max_body_bytes: usize,
}

impl HttpFactFetcher {
    /// Build a fetcher with the [default timeout](DEFAULT_FETCH_TIMEOUT) and
    /// [default body cap](DEFAULT_MAX_BODY_BYTES).
    pub fn new() -> Result<Self, FetchError> {
        Self::with_timeout(DEFAULT_FETCH_TIMEOUT)
    }

    /// Build a fetcher with a custom request timeout (default body cap).
    pub fn with_timeout(timeout: Duration) -> Result<Self, FetchError> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| FetchError::Transport {
                uri: "<client init>".to_string(),
                message: e.to_string(),
            })?;
        Ok(Self {
            client,
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
        })
    }

    /// Override the maximum response body size (builder-style).
    pub fn with_max_body_bytes(mut self, max_body_bytes: usize) -> Self {
        self.max_body_bytes = max_body_bytes;
        self
    }
}

/// The scheme of `uri` (lowercased), i.e. the text before the first `:`.
fn scheme_of(uri: &str) -> Option<String> {
    uri.split_once(':').map(|(s, _)| s.to_ascii_lowercase())
}

#[async_trait]
impl FactFetcher for HttpFactFetcher {
    async fn fetch(&self, uri: &str) -> Result<Vec<u8>, FetchError> {
        // Scheme allowlist FIRST — never hand a non-http(s) uri to reqwest.
        match scheme_of(uri).as_deref() {
            Some("http") | Some("https") => {}
            other => {
                return Err(FetchError::UnsupportedScheme {
                    uri: uri.to_string(),
                    scheme: other.unwrap_or("").to_string(),
                });
            }
        }

        let resp = self
            .client
            .get(uri)
            .send()
            .await
            .map_err(|e| FetchError::Transport {
                uri: uri.to_string(),
                message: e.to_string(),
            })?;

        let status = resp.status();
        if !status.is_success() {
            return Err(FetchError::Status {
                uri: uri.to_string(),
                status: status.as_u16(),
            });
        }

        // Reject an over-cap declared Content-Length up front (cheap, before
        // reading the body).
        if let Some(len) = resp.content_length() {
            if len > self.max_body_bytes as u64 {
                return Err(FetchError::TooLarge {
                    uri: uri.to_string(),
                    limit: self.max_body_bytes,
                });
            }
        }

        // Stream chunk-by-chunk so a body with no/lying Content-Length still
        // can't exceed the cap or exhaust memory.
        let mut resp = resp;
        let mut buf = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| FetchError::Transport {
            uri: uri.to_string(),
            message: e.to_string(),
        })? {
            if buf.len() + chunk.len() > self.max_body_bytes {
                return Err(FetchError::TooLarge {
                    uri: uri.to_string(),
                    limit: self.max_body_bytes,
                });
            }
            buf.extend_from_slice(&chunk);
        }
        Ok(buf)
    }
}

/// A deterministic, no-network fetcher backed by a `uri -> bytes` map. Used by
/// tests: a registered `uri` returns its bytes; an unregistered one returns
/// [`FetchError::NotFound`].
#[derive(Clone, Debug, Default)]
pub struct MockFactFetcher {
    responses: std::collections::HashMap<String, Vec<u8>>,
}

impl MockFactFetcher {
    /// An empty fetcher (every `uri` is `NotFound`).
    pub fn new() -> Self {
        Self::default()
    }

    /// Register the bytes returned for `uri` (builder-style).
    pub fn with(mut self, uri: impl Into<String>, body: impl Into<Vec<u8>>) -> Self {
        self.responses.insert(uri.into(), body.into());
        self
    }

    /// Register the bytes returned for `uri` (mutating).
    pub fn insert(&mut self, uri: impl Into<String>, body: impl Into<Vec<u8>>) {
        self.responses.insert(uri.into(), body.into());
    }
}

#[async_trait]
impl FactFetcher for MockFactFetcher {
    async fn fetch(&self, uri: &str) -> Result<Vec<u8>, FetchError> {
        self.responses
            .get(uri)
            .cloned()
            .ok_or_else(|| FetchError::NotFound {
                uri: uri.to_string(),
            })
    }
}

/// `sha256(bytes)` as 32 bytes — the `content_hash` derivation (plain SHA-256,
/// no framing), matching the off-chain convention the program stores opaquely.
fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

/// Fetch and verify a single agreed fact.
///
/// Fetches `fact_ref.uri`, recomputes `sha256(body)`, and:
/// - on a hash **match**, decodes the body as UTF-8 and returns the verified
///   [`Fact`];
/// - on a **mismatch**, returns [`VerifyError::ContentHashMismatch`] (the
///   content is REJECTED, never returned);
/// - on a **fetch failure**, surfaces the [`FetchError`] (via
///   [`VerifyError::Fetch`]) naming the `uri`;
/// - on **non-UTF-8** content, returns [`VerifyError::NonUtf8`].
pub async fn fetch_and_verify_fact<F>(fetcher: &F, fact_ref: &FactRef) -> Result<Fact, VerifyError>
where
    F: FactFetcher + ?Sized,
{
    let body = fetcher.fetch(&fact_ref.uri).await?;

    let actual = sha256(&body);
    if actual != fact_ref.content_hash {
        return Err(VerifyError::ContentHashMismatch {
            uri: fact_ref.uri.clone(),
            expected: hex::encode(&fact_ref.content_hash),
            actual: hex::encode(&actual),
        });
    }

    // Hash matches; decode as UTF-8 (reject non-UTF-8 rather than lossily
    // decode — the decoded text must correspond to the committed bytes).
    let content = String::from_utf8(body).map_err(|e| VerifyError::NonUtf8 {
        uri: fact_ref.uri.clone(),
        message: e.to_string(),
    })?;

    Ok(Fact {
        content_hash: fact_ref.content_hash,
        content,
    })
}

/// Fetch and verify a whole agreed-fact set, **fail-fast**.
///
/// Returns the verified [`Fact`]s (ready for [`crate::prompt::assemble`]) in the
/// SAME order as `fact_refs`, or the first [`VerifyError`] encountered. Any
/// single bad fact (unfetchable, tampered, or non-UTF-8) fails the whole run —
/// the agreed fact set is all-or-nothing.
pub async fn fetch_and_verify_facts<F>(
    fetcher: &F,
    fact_refs: &[FactRef],
) -> Result<Vec<Fact>, VerifyError>
where
    F: FactFetcher + ?Sized,
{
    let mut verified = Vec::with_capacity(fact_refs.len());
    for fact_ref in fact_refs {
        verified.push(fetch_and_verify_fact(fetcher, fact_ref).await?);
    }
    Ok(verified)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{AiProvider, CategoricalOptions, MockProvider};

    /// Real `content_hash` for some content: `sha256(content)`.
    fn ch(content: &[u8]) -> [u8; 32] {
        sha256(content)
    }

    // --- match --------------------------------------------------------------

    #[tokio::test]
    async fn verifies_matching_content() {
        let content = b"BTC closed at $98,000 on 2025-12-31.";
        let uri = "https://facts.example/btc";
        let fetcher = MockFactFetcher::new().with(uri, content.to_vec());
        let fact_ref = FactRef::new(ch(content), uri);

        let fact = fetch_and_verify_fact(&fetcher, &fact_ref).await.unwrap();

        // The verified Fact carries the content_hash and the UTF-8 of the body.
        assert_eq!(fact.content_hash, ch(content));
        assert_eq!(fact.content, "BTC closed at $98,000 on 2025-12-31.");
    }

    // --- mismatch (tampered content) ----------------------------------------

    #[tokio::test]
    async fn rejects_content_hash_mismatch() {
        let committed = b"the agreed fact";
        let tampered = b"a DIFFERENT, tampered fact";
        let uri = "https://facts.example/x";
        // The fetcher serves tampered bytes, but the ref commits to the
        // original content's hash.
        let fetcher = MockFactFetcher::new().with(uri, tampered.to_vec());
        let fact_ref = FactRef::new(ch(committed), uri);

        let err = fetch_and_verify_fact(&fetcher, &fact_ref)
            .await
            .unwrap_err();

        match err {
            VerifyError::ContentHashMismatch {
                uri: u,
                expected,
                actual,
            } => {
                assert_eq!(u, uri);
                assert_eq!(expected, hex::encode(&ch(committed)));
                assert_eq!(actual, hex::encode(&ch(tampered)));
                assert_ne!(expected, actual);
            }
            other => panic!("expected ContentHashMismatch, got {other:?}"),
        }
    }

    // --- fetch failure ------------------------------------------------------

    #[tokio::test]
    async fn surfaces_fetch_failure_with_uri() {
        let uri = "https://facts.example/missing";
        let fetcher = MockFactFetcher::new(); // nothing registered
        let fact_ref = FactRef::new([0u8; 32], uri);

        let err = fetch_and_verify_fact(&fetcher, &fact_ref)
            .await
            .unwrap_err();

        // The uri is named in the rendered error.
        assert!(format!("{err}").contains(uri));
        match err {
            VerifyError::Fetch(FetchError::NotFound { uri: u }) => assert_eq!(u, uri),
            other => panic!("expected Fetch(NotFound), got {other:?}"),
        }
    }

    // --- non-UTF-8 ----------------------------------------------------------

    #[tokio::test]
    async fn rejects_non_utf8_body_that_hashes_correctly() {
        // Invalid UTF-8 bytes, but the ref commits to THEIR hash, so the hash
        // check passes and the UTF-8 check is what must reject it.
        let bytes: Vec<u8> = vec![0xff, 0xfe, 0x00, 0x80];
        let uri = "https://facts.example/binary";
        let fetcher = MockFactFetcher::new().with(uri, bytes.clone());
        let fact_ref = FactRef::new(ch(&bytes), uri);

        let err = fetch_and_verify_fact(&fetcher, &fact_ref)
            .await
            .unwrap_err();

        match err {
            VerifyError::NonUtf8 { uri: u, .. } => assert_eq!(u, uri),
            other => panic!("expected NonUtf8, got {other:?}"),
        }
    }

    // --- unsupported scheme (HTTP fetcher, offline — fails before any I/O) ---

    #[tokio::test]
    async fn http_fetcher_rejects_non_http_scheme() {
        let fetcher = HttpFactFetcher::new().unwrap();
        let err = fetcher.fetch("file:///etc/passwd").await.unwrap_err();
        match err {
            FetchError::UnsupportedScheme { uri, scheme } => {
                assert_eq!(uri, "file:///etc/passwd");
                assert_eq!(scheme, "file");
            }
            other => panic!("expected UnsupportedScheme, got {other:?}"),
        }
    }

    // --- body-size cap (local server, offline) ------------------------------

    #[tokio::test]
    async fn http_fetcher_rejects_oversize_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // A one-shot server that returns a body larger than the cap we set.
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            // Drain the request headers so reqwest's write completes.
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await;
            let body = vec![b'x'; 1000];
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(head.as_bytes()).await;
            let _ = sock.write_all(&body).await;
            let _ = sock.flush().await;
        });

        let fetcher = HttpFactFetcher::new().unwrap().with_max_body_bytes(100);
        let uri = format!("http://{addr}/big");
        let err = fetcher.fetch(&uri).await.unwrap_err();
        match err {
            FetchError::TooLarge { uri: u, limit } => {
                assert_eq!(u, uri);
                assert_eq!(limit, 100);
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    // --- multiple facts, fail-fast ------------------------------------------

    #[tokio::test]
    async fn batch_all_good_returns_verified_in_order() {
        let c0 = b"fact zero";
        let c1 = b"fact one";
        let c2 = b"fact two";
        let fetcher = MockFactFetcher::new()
            .with("https://f/0", c0.to_vec())
            .with("https://f/1", c1.to_vec())
            .with("https://f/2", c2.to_vec());
        let refs = vec![
            FactRef::new(ch(c0), "https://f/0"),
            FactRef::new(ch(c1), "https://f/1"),
            FactRef::new(ch(c2), "https://f/2"),
        ];

        let facts = fetch_and_verify_facts(&fetcher, &refs).await.unwrap();

        assert_eq!(facts.len(), 3);
        // Same order as the input refs.
        assert_eq!(facts[0].content, "fact zero");
        assert_eq!(facts[1].content, "fact one");
        assert_eq!(facts[2].content, "fact two");
    }

    #[tokio::test]
    async fn batch_fails_fast_on_one_bad_fact() {
        let good = b"good fact";
        let committed_for_bad = b"committed content";
        let tampered = b"tampered content";
        let fetcher = MockFactFetcher::new()
            .with("https://f/good", good.to_vec())
            .with("https://f/bad", tampered.to_vec());
        let refs = vec![
            FactRef::new(ch(good), "https://f/good"),
            // This one is tampered: serves bytes that don't match its hash.
            FactRef::new(ch(committed_for_bad), "https://f/bad"),
        ];

        let err = fetch_and_verify_facts(&fetcher, &refs).await.unwrap_err();
        match err {
            VerifyError::ContentHashMismatch { uri, .. } => assert_eq!(uri, "https://f/bad"),
            other => panic!("expected ContentHashMismatch, got {other:?}"),
        }
    }

    // --- composition: verified facts feed R2's assemble ---------------------

    #[tokio::test]
    async fn verified_facts_feed_prompt_assembly_and_mock_provider() {
        let c0 = b"The date in question is 2025-12-31.";
        let c1 = b"BTC closed at $98,000.";
        let fetcher = MockFactFetcher::new()
            .with("https://f/date", c0.to_vec())
            .with("https://f/price", c1.to_vec());
        let refs = vec![
            FactRef::new(ch(c0), "https://f/date"),
            FactRef::new(ch(c1), "https://f/price"),
        ];

        let facts = fetch_and_verify_facts(&fetcher, &refs).await.unwrap();

        // Feed the verified Facts straight into R2's assemble.
        let opts = CategoricalOptions {
            count: 2,
            labels: None,
        };
        let assembled = crate::prompt::assemble("Resolve YES iff BTC > $100k.", &facts, &opts);
        // Both verified contents made it into the prompt.
        assert!(assembled
            .user
            .contains("The date in question is 2025-12-31."));
        assert!(assembled.user.contains("BTC closed at $98,000."));

        // And the assembled request runs through the mock provider.
        let req = crate::prompt::build_request(
            "Resolve YES iff BTC > $100k.",
            &facts,
            opts,
            crate::provider::ModelConfig {
                model_id: "claude-opus-4-8".to_string(),
                provider: "mock".to_string(),
                max_tokens: 1024,
                thinking: Some("adaptive".to_string()),
            },
        );
        let provider = MockProvider::new(1, r#"{"option_index":1}"#, "mock-claude");
        let resp = provider.complete(&req).await.unwrap();
        assert_eq!(resp.option_index, 1);
    }
}

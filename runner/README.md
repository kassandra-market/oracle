# kassandra-runner

The open-source, reproducible **off-chain AI runner** for Kassandra oracle
resolution.

Given an oracle's fixed interpretation (its on-chain `prompt_hash` commitment),
the agreed fact set (each `content_hash` + `uri`), and the categorical options,
the runner:

1. **fetches + verifies** each fact's content against its on-chain `content_hash`
   (tampered/unavailable facts are rejected, never fed to the model),
2. **assembles** a byte-deterministic prompt (system + user),
3. **completes** it with a pinned model behind a generic provider trait
   (Claude/Anthropic by default),
4. **hashes** the result into the on-chain claim metadata
   (`model_id` / `params_hash` / `io_hash`), and
5. **emits** the chosen categorical `option` plus the exact 97-byte
   `submit_ai_claim` payload — the bytes the on-chain program records and that a
   **challenger can independently reproduce**.

By default it only **emits** the payload. It can also run as a self-contained
**keeper** (`run --submit`) that signs + sends + confirms the `submit_ai_claim`
transaction itself (see [Keeper mode](#keeper-mode-run---submit)). It can read
the oracle/fact accounts read-only from chain (the [on-chain config
mode](#on-chain-config-mode), which pairs an RPC fetch with a `--prompt-file`
supplying the interpretation text that hashes to the on-chain `prompt_hash`), or
take an explicit config as input; either way it prints the claim metadata.

## Determinism caveat (read this first)

**No frontier model API is bit-reproducible**, even with a pinned model and fixed
parameters. The runner does **not** pretend otherwise. The Kassandra protocol's
resolution mode + decision market are the ultimate arbiter of the categorical
answer; the runner's job is to give:

- **best-effort determinism** — a pinned model string, no sampling knobs
  (`temperature` / `top_p` / `top_k` are never sent), and a structured-output
  JSON schema that forces a clean `{ "option_index": <int> }` answer instead of
  free-text scraping; and
- **reproducible metadata** — `model_id` and `params_hash` are pure functions of
  the declared config and reproduce byte-for-byte; the assembled prompt is fully
  deterministic; and `io_hash` is a **commitment to the exact (input, raw
  response)** the submitter actually used.

In particular, **`io_hash` is a commitment, not a reproducibility oracle.** A
challenger is not expected to reproduce bit-identical model text; they reproduce
the *categorical* option (and confirm `model_id` / `params_hash` match), and
`io_hash` proves what the proposer actually ran.

## Architecture

- **`AiProvider` trait** (`src/provider.rs`) — one async `complete(&req)` call
  from an assembled request to a categorical answer + the metadata the hashing
  needs. Everything downstream is model-agnostic.
- **`AnthropicProvider`** (`src/anthropic.rs`) — the default. Rust has no
  official Anthropic SDK, so this is a thin `reqwest` client over
  `POST https://api.anthropic.com/v1/messages` (headers `x-api-key`,
  `anthropic-version: 2023-06-01`). Model `claude-opus-4-8`, adaptive thinking,
  the categorical answer forced via `output_config.format` json_schema. It never
  sends `temperature` / `top_p` / `top_k` / `budget_tokens` (Opus 4.8 rejects
  them with a 400).
- **`MockProvider`** (`src/provider.rs`) — a deterministic, no-network provider
  (fixed option + canned response) that makes the whole pipeline testable and
  runnable offline with no API key.
- **Prompt assembly** (`src/prompt.rs`, spec: [`PROMPT.md`](PROMPT.md)) — the
  canonical, versioned `system` / `user` strings + the structured-output schema
  + the categorical parser.
- **Fact fetch + verify** (`src/fetch.rs`) — `FactFetcher` trait,
  `HttpFactFetcher` (http/https only, request timeout, body-size cap), and a
  `MockFactFetcher` for offline tests. Verifies `sha256(body) == content_hash`.
- **Canonical hashing** (`src/hashing.rs`, spec: [`HASHING.md`](HASHING.md)) —
  the single source of truth for `model_id` / `params_hash` / `io_hash` and the
  97-byte payload assembly.
- **CLI** (`src/cli.rs`, entry `src/main.rs`) — the `run` / `verify` commands.

The runner reuses the on-chain program crate (`kassandra-oracles-program`) for
`CLAIM_OPTION_NONE` and the `submit_ai_claim` payload widths, pinned to the
actual `AiClaim` field layout via compile-time assertions — so the runner can
never drift from the on-chain encoding (see `src/constants.rs`,
[`NOTES.md`](NOTES.md)).

## Build & test

```sh
cargo build -p kassandra-runner
cargo test  -p kassandra-runner   # fully offline + keyless (mock provider/fetcher)
```

The default test suite uses the mock provider and a mock fetcher: **no API key,
no network**. The single live Anthropic test is `#[ignore]`d and runs only on
demand:

```sh
ANTHROPIC_API_KEY=sk-... cargo test -p kassandra-runner --lib \
  -- --ignored live_anthropic_completion --nocapture
```

The end-to-end pipeline test lives in [`tests/e2e.rs`](tests/e2e.rs): it drives
the production `run_core` / `verify_core` with the mocks and asserts payload
decomposition, byte-reproducibility, tamper-rejection, and a pinned 97-byte
payload anchor.

## How to run

The runner builds its config in one of two mutually-exclusive ways:

1. an explicit **JSON config** from `--config <path>` or stdin (below), or
2. an **on-chain fetch** from `--oracle <pubkey> --rpc-url <url> --prompt-file
   <path>` (see [On-chain config mode](#on-chain-config-mode)).

### Config JSON shape

```json
{
  "interpretation": "Resolve YES if BTC closed at or above $100,000 on the date; otherwise NO.",
  "options_count": 2,
  "option_labels": [
    { "index": 0, "label": "Yes" },
    { "index": 1, "label": "No" }
  ],
  "facts": [
    { "content_hash": "<64-hex sha256 of the fact content>", "uri": "https://..." }
  ],
  "oracle": "<oracle pubkey, optional — only echoed as the AiClaim PDA seeds>",
  "proposer": "<proposer pubkey, optional>"
}
```

- `interpretation` — the oracle's resolution rules (its on-chain `prompt_hash`).
- `options_count` — the categorical option count (mirrors `Oracle.options_count`).
- `option_labels` — optional; matched to indices by their explicit `index`.
- `facts` — each `content_hash` is `sha256(content)` as 64 hex chars (a leading
  `0x` is allowed); the runner fetches `uri`, recomputes the hash, and **rejects
  any mismatch**.
- `oracle` / `proposer` — optional; only echoed to describe the AiClaim PDA seeds
  (`[b"claim", oracle, proposer]`).

### On-chain config mode

Instead of an explicit `--config`, the runner can build the config directly from
an oracle account on chain:

```sh
kassandra-runner run \
  --oracle <ORACLE_PUBKEY_BASE58> \
  --rpc-url https://api.mainnet-beta.solana.com \
  --prompt-file interpretation.txt \
  --mock
```

What it does:

- **`getAccountInfo`** (base64) for the oracle account → verifies it is owned by
  the Kassandra program and carries the `Oracle` account-type tag → `bytemuck`
  decodes it through the **shared `kassandra_oracles_program::state::Oracle`** struct,
  reading `options_count`, `deadline`, and the `prompt_hash` commitment.
- **`getProgramAccounts`** with a `dataSize == Fact::LEN` filter and a `memcmp`
  on the `Fact.oracle` field enumerates this oracle's `Fact` accounts, decodes
  each through the shared `Fact` struct, and keeps the ones whose `agreed` flag
  is set → their `content_hash` + `uri` become the fact set (fetched and
  hash-verified exactly as in the explicit-config path).
- The interpretation **TEXT** is **not** on chain — only its `prompt_hash` is.
  So it is read from `--prompt-file`, and the runner asserts
  **`sha256(prompt_file_bytes) == oracle.prompt_hash`**, mirroring the fact
  `content_hash` check. **A mismatch is rejected** — a wrong or tampered prompt
  file can never be fed to the model as this oracle's interpretation.

Transport is plain JSON-RPC over the same `reqwest` stack the fact fetcher uses —
there is **no `solana-client`/`solana-sdk` dependency**. `--oracle` requires both
`--rpc-url` and `--prompt-file` and is mutually exclusive with `--config`.

> **Prompt-hash requirement.** The prompt file must contain the oracle's
> interpretation text verbatim, byte-for-byte, such that its SHA-256 equals the
> on-chain `prompt_hash` the oracle was created with (the same plain-SHA-256,
> no-framing convention the fact `content_hash` uses). Keep the exact text that
> was hashed at `create_oracle` time.

`verify` accepts the same `--oracle/--rpc-url/--prompt-file` flags.

### `run` — resolve and emit the claim

```sh
# Offline, no key (deterministic mock provider):
kassandra-runner run --config oracle.json --mock

# Real Claude (reads ANTHROPIC_API_KEY from the environment):
export ANTHROPIC_API_KEY=sk-...
kassandra-runner run --config oracle.json
```

`run` prints JSON: the chosen `option_index`, the three hashes (hex), the exact
97-byte `submit_ai_claim_payload_hex` (`model_id ++ params_hash ++ io_hash ++
option`), the `resolved_model_id`, and (when `oracle`/`proposer` are present) the
AiClaim PDA seeds. The payload hex is what you hand to the SDK/CLI that actually
submits the transaction — unless you use `--submit` (below), where the runner
submits it for you.

### Keeper mode (`run --submit`)

By default `run` only emits the payload (no network write). With `--submit`, the
runner becomes a **self-contained keeper**: after producing the claim it BUILDS,
SIGNS, SENDS, and CONFIRMS the `submit_ai_claim` transaction itself, then reports
the confirmed signature (or a clear program error). No SDK bridge needed.

```sh
export ANTHROPIC_API_KEY=sk-...
kassandra-runner run --submit \
  --oracle <ORACLE_PUBKEY_BASE58> \
  --rpc-url https://api.mainnet-beta.solana.com \
  --keypair ~/.config/solana/id.json \
  --prompt-file interpretation.txt
```

The full keeper path is: **fetch the oracle/facts from chain (or read an explicit
`--config`) → run the model → sign + submit → confirmed signature.**

- **`--keypair <path>`** is a standard Solana CLI keypair JSON (a 64-byte array).
  **The signer MUST be the proposer's registered `authority`** — the on-chain
  `submit_ai_claim` asserts `authority == proposer.authority`, and the Proposer
  PDA the transaction targets is DERIVED as `[b"proposer", oracle, authority]`
  from the oracle and this keypair's pubkey. There is no `--proposer` flag: the
  keeper is run *by* that proposer, so its Proposer PDA is fully determined.
- **The submitted transaction carries the runner's OWN 97-byte payload verbatim**
  — the exact bytes shown in `submit_ai_claim_payload_hex`, never recomputed — so
  the submitted claim can never diverge from the emitted metadata.
- **`--oracle`** (or the config's `oracle` field) and **`--rpc-url`** are
  required with `--submit`. In explicit-`--config` mode `--rpc-url` is normally
  optional, but it is **required for submission** (the network to submit to); a
  missing `--keypair` / `--rpc-url` / oracle fails fast with a clear error before
  the model is ever called.
- The result is appended to the JSON output under `submission` (the confirmed
  `signature`, `confirmation_status`, `oracle`, derived `proposer`, and
  `authority`). Send + confirm rides the same reqwest JSON-RPC transport as the
  on-chain fetch (no `solana-client`); confirmation polls `getSignatureStatuses`
  to `confirmed` (~30s budget).

> **Idempotency / phase notes.** `submit_ai_claim` creates the AiClaim PDA
> `[b"claim", oracle, proposer]`, so a **second submit for the same (oracle,
> proposer) FAILS** — the PDA already exists (`DuplicateClaim`). Submission also
> requires the oracle to be in its `AiClaim` phase within the claim window, the
> proposer to be registered + not disqualified, and the option to index a real
> categorical option; a wrong-phase / closed-window / already-submitted attempt
> surfaces the program error (via the tx preflight or the confirmed status),
> which the runner reports rather than retrying.

### `verify` — should I challenge?

```sh
kassandra-runner verify --config oracle.json --option 1 \
  [--submitted-model-id <hex>] [--submitted-params-hash <hex>] [--submitted-io-hash <hex>] \
  [--mock]
```

`verify` re-runs the pipeline and compares the produced option (and, when
provided, the submitted hashes) to a submitted claim, advising
`"matches (no challenge)"` vs `"differs (consider challenging)"`.

### Flags

| Flag | Meaning |
|------|---------|
| `--config <path>` | Config JSON path; omit to read from stdin. Mutually exclusive with `--oracle`. |
| `--oracle <pubkey>` | Build the config from this on-chain oracle instead of `--config`. Requires `--rpc-url` + `--prompt-file`. |
| `--rpc-url <url>` | Solana JSON-RPC url used with `--oracle`. |
| `--prompt-file <path>` | Interpretation text file used with `--oracle`; its `sha256` must equal the on-chain `prompt_hash`. |
| `--submit` | (`run`) Keeper mode: sign + send + confirm the `submit_ai_claim` tx. Requires `--keypair` + `--rpc-url` + an oracle. Default is emit-only. |
| `--keypair <path>` | (`run --submit`) Solana CLI keypair JSON (64-byte array) that signs the tx. MUST be the proposer's `authority`. |
| `--mock` | Use the deterministic `MockProvider` (offline, no key). Also enabled by `KASSANDRA_RUNNER_MOCK=1`. |
| `--model <str>` | Override the pinned model string (default `claude-opus-4-8`). |
| `--max-tokens <n>` | Override `max_tokens` (default `4096`). |
| `--option <n>` | (`verify`) the submitted claim's option to compare against. |
| `--submitted-{model-id,params-hash,io-hash} <hex>` | (`verify`) optional submitted hashes to compare. |

`ANTHROPIC_API_KEY` is read from the environment for the real provider; it is
never hardcoded and never logged. Pass `--mock` (or set `KASSANDRA_RUNNER_MOCK`)
to run without it.

## Claim-metadata hashing & how a challenger reproduces

The on-chain program stores three opaque 32-byte commitments
(`model_id` / `params_hash` / `io_hash`) plus a 1-byte categorical `option`, and
the `submit_ai_claim` payload is:

```
model_id[32] ++ params_hash[32] ++ io_hash[32] ++ option[1]   = 97 bytes
```

The program does **not** compute the hashes — the runner defines the canonical,
byte-exact off-chain scheme that both the proposer and a challenger follow. The
two specs are normative and language-independent (the `HASHING.md` doc even
includes a Python reference):

- **[`HASHING.md`](HASHING.md)** — the exact byte layout of `model_id`,
  `params_hash`, and `io_hash` (fixed field order, big-endian integers,
  length-prefixed strings; SHA-256).
- **[`PROMPT.md`](PROMPT.md)** — the exact byte layout of the assembled
  `system` / `user` strings (canonical fact ordering, fixed separators,
  versioned format).

### Challenger reproduction steps

To independently reproduce an `AiClaim` from the same inputs:

1. **Pin the same model.** Use the claim's `model_id` string (default
   `claude-opus-4-8`) and the same declared params (provider, thinking mode,
   `max_tokens`, the runner's `OUTPUT_SCHEMA_ID`/`OUTPUT_SCHEMA_VERSION` and
   `PROMPT_ASSEMBLY_VERSION`).
2. **Re-verify the facts.** Fetch each agreed fact's `uri` and confirm
   `sha256(body) == content_hash`; reject any mismatch.
3. **Assemble** the `system` / `user` strings from the same interpretation +
   verified facts (in canonical `content_hash` order) + enumerated options, per
   [`PROMPT.md`](PROMPT.md). This must be byte-identical.
4. **Hash** per [`HASHING.md`](HASHING.md):
   - `model_id = sha256(model_id_string)` — must equal the submitted `model_id`.
   - `params_hash = sha256(canonical_params_bytes)` — must equal the submitted
     `params_hash`. (`model_id` + `params_hash` are pure functions of the
     declared config and reproduce exactly.)
   - `io_hash = sha256(str(system) ++ str(user) ++ raw_response)` — a commitment
     to the submitter's exact input + raw model output.
5. **Compare the categorical option.** Re-run the model and compare your
   `option_index` to the claim's `option`. If it differs, consider challenging.
   This categorical comparison — *not* bit-identical model text — is the
   reproducibility contract, because `model_id` / `params_hash` reproduce while
   `io_hash` only commits to what the proposer ran.

`kassandra-runner verify` automates steps 2–5.

## v1 limitations

Deliberate, documented scope boundaries for v1:

- **On-chain fetch is read-only + prompt-file-paired.** The runner reads the
  oracle/fact accounts over JSON-RPC (see [On-chain config
  mode](#on-chain-config-mode)), but the interpretation TEXT is not on chain, so
  the on-chain fetch MUST be paired with a `--prompt-file` whose `sha256` matches
  the on-chain `prompt_hash`. The RPC client has no retry/failover and reads at
  `confirmed` commitment; `getProgramAccounts` enumeration requires an RPC that
  serves it (some providers gate or paginate it for large programs).
- **SSRF to internal IPs is not blocked.** The fetcher's scheme allowlist stops
  `file:` / `data:` / etc., but a fact `uri` may still resolve to a
  link-local / loopback / internal address, and redirects follow `reqwest`'s
  default policy. Treat fact URIs as untrusted and add egress filtering /
  DNS-pinning at the deployment layer. (A response body-size cap *is* enforced.)
- **One bundled provider + a mock.** Only Claude/Anthropic (default) and the
  deterministic `MockProvider` ship; additional providers are future work.
- **No TEE / zkTLS attestation.** Explicitly rejected in the design; the protocol
  relies on the decision market, not hardware/proof attestation of the run.

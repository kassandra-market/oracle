# Kassandra Off-chain AI Runner — Design + Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans / subagent-driven-development to implement this plan task-by-task.

**Goal:** A `runner/` Rust crate — an open-source, reproducible AI runner that takes an oracle's fixed prompt + interpretation + the agreed fact set + the categorical options, calls a pinned model behind a **generic provider trait (Claude/Anthropic as the default)**, and emits a **categorical answer + the on-chain claim metadata** (`model_id` / `params_hash` / `io_hash`) that `submit_ai_claim` records and that a challenger can independently reproduce/verify.

**Architecture:** A new Cargo workspace member `runner/` (crate `kassandra-runner`), a CLI + library. The runner **shares the program crate** (`kassandra-program`) for the option encoding, `CLAIM_OPTION_NONE`, the `AiClaim` field meaning, and (ideally) a canonical claim-hash module — so there is **no cross-language hash-parity risk**. A generic `AiProvider` trait abstracts the model; the default `AnthropicProvider` calls the real Anthropic API; a `MockProvider` makes the whole pipeline deterministically testable without an API key.

**Tech Stack:** Rust (workspace member), `tokio` (async), `reqwest` (HTTP — for both the AI call and fact fetching), `sha2`, `serde`/`serde_json`, `clap` (CLI). The on-chain program is the source of truth and is READ-ONLY (do NOT modify it; if a genuine mismatch/bug is found, STOP and report).

## AI provider decision (from the claude-api skill + the user)
- **Generic `AiProvider` trait + Claude/Anthropic as the default**, swappable (the on-chain `model_id` identifies which model+params a challenger must match).
- **Rust has NO official Anthropic SDK** → the `AnthropicProvider` uses **raw HTTP via `reqwest`** against `POST https://api.anthropic.com/v1/messages` (headers: `x-api-key: $ANTHROPIC_API_KEY`, `anthropic-version: 2023-06-01`, `content-type: application/json`).
- **Model: `claude-opus-4-8`** (the default; pinned). **Adaptive thinking** (`thinking: {type: "adaptive"}`). **NO `temperature`/`top_p`/`budget_tokens`** — those are rejected (400) on Opus 4.8; do not send them.
- **Categorical extraction via structured outputs** (`output_config: {format: {type: "json_schema", schema: {...}}}`) forcing a clean `{ "option_index": <int in [0,options_count)> }` (or equivalent) so parsing is robust + deterministic — not free-text "ANSWER:" scraping. Guard the index against `options_count`.
- **Determinism reality (design-acknowledged):** no frontier API is bit-reproducible even with fixed params; the protocol's mode + decision market are the arbiter. The runner targets BEST-EFFORT determinism (pinned model, structured output) + metadata that lets a challenger re-run and compare the **categorical** answer (not bit-identical text). `io_hash` is a COMMITMENT to the exact (input, raw response) the submitter used, not a reproducibility oracle.

## Claim-metadata hashing (the protocol contract — define precisely + document)
The program stores three 32-byte hashes in `AiClaim` (`model_id[32]`, `params_hash[32]`, `io_hash[32]`) + `option u8` (submit_ai_claim payload `model_id[32] ++ params_hash[32] ++ io_hash[32] ++ option u8`). The program does NOT compute them — the runner defines the canonical off-chain scheme that BOTH the proposer's runner and a challenger's re-run must follow. Define (and document as THE spec):
- `model_id = sha256(model_id_string_utf8)` (e.g. `sha256("claude-opus-4-8")`). Document the exact string.
- `params_hash = sha256(canonical_params_bytes)` where canonical params = a deterministic serialization of the request config that affects the answer: model string, provider id, thinking mode, the output-schema/version, max_tokens, and the runner's prompt-assembly version. Define a stable, sorted, versioned encoding (e.g. a canonical JSON with sorted keys, or a fixed field order) — document it byte-for-byte so a challenger reproduces it.
- `io_hash = sha256(canonical_input_bytes ++ raw_response_bytes)` where canonical_input = the exact assembled model input (system + user text in the canonical prompt-assembly order, including the sorted agreed facts + enumerated options) and raw_response = the model's raw response text (the structured-output JSON string, verbatim). Document the concatenation + encoding.
- Put the canonical hashing + prompt-assembly in a single module so it's the one source of truth; expose it for a `verify` path. If sharing it INTO the program crate is clean (host-only), do so; otherwise keep it in the runner crate as the reference + document. Decide in R0/R1.

## Conventions
- The runner is a NEW crate; do NOT modify the on-chain program. TDD-ish. `cargo build` + `cargo test -p kassandra-runner` green + `cargo clippy` clean + `cargo fmt` before each commit.
- Commit trailer `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`, git author `Kassandra <hexadecifish@gmail.com>`.
- The real Anthropic call must be behind the `AiProvider` trait and gated in tests (env-gated / `#[ignore]`) so the suite runs without an API key. The MockProvider is the deterministic default for tests.
- Do NOT hardcode an API key; read `ANTHROPIC_API_KEY` from env.

## Live-state the runner must mirror (verify against the program)
- `submit_ai_claim.rs` payload: `model_id[32] ++ params_hash[32] ++ io_hash[32] ++ option u8` (verify the exact order/length); the AiClaim PDA seeds `[b"claim", oracle, proposer]`; `option` is a categorical index `< options_count`; `CLAIM_OPTION_NONE = 0xFF`.
- The oracle's `prompt_hash[32]` is set at creation (the interpretation/prompt commitment) + `options_count`. Facts are `content_hash[32]` + `uri` (≤200 bytes) — the runner fetches + verifies content against `content_hash`.

## Tasks

### R0 — Scaffold + provider trait + recon (DO FIRST; stop-and-report if sharing the program crate is unworkable)
- Create `runner/` workspace member (`Cargo.toml` crate `kassandra-runner`; add to the workspace members). Deps: tokio, reqwest (rustls), sha2, serde, serde_json, clap, anyhow/thiserror.
- **Recon (write `runner/NOTES.md`):** can the runner depend on `kassandra-program` as a `lib` to reuse constants (`CLAIM_OPTION_NONE`, option encoding, the submit_ai_claim payload layout)? Check the program crate's `crate-type`/targets and whether the Solana deps make a host dependency heavy. If clean, depend on it; if not, MIRROR the handful of constants the runner needs + add a parity assertion (test) referencing the documented values, and note it. Decide where the canonical claim-hash module lives (shared into the program crate host-side, or in the runner as the reference) — document the choice + rationale.
- Define `AiProvider` trait (`async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse>`), the `CompletionRequest`/`CompletionResponse` types (request carries the assembled input + the categorical options + model config; response carries the chosen option index + the raw response text + the resolved model_id string + params used), and a deterministic `MockProvider` (configurable fixed option + canned response). 
- A smoke test: build a CompletionRequest, run it through MockProvider, assert the deterministic option/response. Commit `chore(runner): scaffold crate + AiProvider trait + mock + recon`.

### R1 — Canonical claim-metadata hashing (the contract)
- Implement `model_id`/`params_hash`/`io_hash` per the spec above in a single canonical module. Document the exact byte layout of each (a `runner/HASHING.md` or rustdoc) so a third-party challenger can reproduce it.
- Reproducibility tests: known inputs → known (stable) 32-byte hashes; same inputs → identical hashes across runs; a changed model/param/input flips the relevant hash; the params canonical encoding is deterministic (sorted/fixed-order). Verify the produced 32-byte arrays are the right width for the submit_ai_claim payload. Commit `feat(runner): canonical claim-metadata hashing (model_id/params_hash/io_hash)`.

### R2 — Prompt assembly + categorical parsing
- Deterministic prompt assembly: system = the oracle's interpretation/prompt text; user = the agreed facts (in a CANONICAL order — e.g. sorted by content_hash) + the enumerated categorical options (0..options_count) + an instruction to choose exactly one option index. Stable, versioned (the assembly version feeds params_hash).
- Categorical parsing: from the structured-output JSON → option index; validate `0 <= index < options_count` (reject/error otherwise). Provide the json_schema the AnthropicProvider will use.
- Tests via MockProvider + unit tests on assembly (canonical fact ordering is deterministic; options enumerated correctly) + parsing (valid index, out-of-range rejected, malformed rejected). Commit `feat(runner): deterministic prompt assembly + categorical parsing`.

### R3 — Fact fetching + content_hash verification
- Fetch each agreed fact's content from its `uri` (reqwest GET; support http(s); document any scheme limits), compute `sha256(content)`, and assert it equals the on-chain `content_hash` — REJECT on mismatch (tampered/unavailable fact) with a clear error. Feed verified content into prompt assembly.
- Tests: a mock/local fetcher (a `FactFetcher` trait or injectable client) so tests don't hit the network — content_hash match passes, mismatch rejected, fetch failure surfaced. Commit `feat(runner): fact fetch + content_hash verification`.

### R4 — Anthropic provider (Claude default) + CLI
- `AnthropicProvider` implementing `AiProvider`: reqwest `POST /v1/messages`, model `claude-opus-4-8`, `thinking:{type:"adaptive"}`, `output_config.format` json_schema for the categorical answer, `x-api-key`/`anthropic-version` headers, `max_tokens` set, NO temperature/top_p/budget_tokens. Parse `stop_reason` (handle `refusal` → error), extract the structured JSON from the response content, parse the option index. Env: `ANTHROPIC_API_KEY`.
- CLI (clap): `run` (read an oracle-config input — prompt/interpretation + options_count + the agreed facts' content_hash+uri, from a JSON file or flags — assemble, fetch+verify facts, call the provider, emit the claim metadata JSON + the `submit_ai_claim` payload bytes/hex) and `verify` (re-run for a given oracle + compare the produced option to a submitted claim's option → advise challenge-or-not). Default provider = Anthropic; a `--mock` flag (or env) selects the MockProvider for offline use.
- Tests: the Anthropic provider behind an env-gated/`#[ignore]` integration test (skipped without a key); the CLI `run` path tested end-to-end with `--mock` (deterministic). Commit `feat(runner): Anthropic provider + run/verify CLI`.

### R5 — End-to-end (mock) + docs
- An end-to-end test using MockProvider + a mock fact fetcher: oracle-config input → assemble → fetch+verify facts → mock-complete → produce option + the three hashes + the submit_ai_claim payload; assert the payload is exactly `model_id[32] ++ params_hash[32] ++ io_hash[32] ++ option u8` (correct widths/order) and that `verify` agrees with itself. `runner/README.md`: what it is, the determinism caveat, the hashing spec (link HASHING.md), how to run (`run`/`verify`, the `--mock` flag, `ANTHROPIC_API_KEY`), and how a challenger reproduces. Append a "covered vs deferred" note. Commit `docs(runner): readme + end-to-end mock test + hashing spec`.

## Out of scope (later)
- Submitting the transaction on-chain (the runner emits the payload; submission is the SDK/CLI's job or a thin later layer); fetching the oracle/fact ACCOUNTS from chain via RPC (the runner takes an explicit config input for v1 — on-chain fetch can be a thin later layer); a TEE/zkTLS attestation of the run (explicitly rejected in the design); multiple bundled providers beyond Claude + mock.

## Execution note
After each task: build + `cargo test -p kassandra-runner` + clippy + fmt green, commit. R0 (program-crate sharing) + R1 (the hashing contract) are the crux — get the hashing byte-exact + documented so a challenger reproduces it. The program is read-only truth. Append an R0–R5 delta log here.

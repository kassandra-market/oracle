# Kassandra surfpool E2E (runner-in-the-loop, mock AI) — Design + Plan

> **For Claude:** REQUIRED SUB-SKILL: subagent-driven-development (per-task implement + review).

**Goal:** A surfpool-based end-to-end test suite that drives the full Kassandra oracle lifecycle against a real RPC validator, with the **off-chain runner in the loop** producing AI claims from **controllable mock AI responses** (a local mock Anthropic server + the runner's real `AnthropicProvider`). Core lifecycle solid; the forked-MetaDAO challenge-market path pushed as far as tractable, deferrals documented.

**Architecture:** surfpool (`surfpool start`, standard Solana RPC on :8899; forks mainnet on demand via `--network mainnet`/`--rpc-url`) is driven by a TypeScript harness that REUSES the merged SDK (`sdk/` — web3.js v3 instruction builders + decoders + the `submitAiClaimFromRunner` bridge) over RPC, and invokes the **runner binary** as a subprocess at the AI-claim step. A local **mock Anthropic server** returns canned `/v1/messages` responses; the runner's `AnthropicProvider` is pointed at it via a new **base-URL override** (env/flag) so the REAL provider HTTP+parse path is exercised. The suite is **GATED** (opt-in; skips if surfpool RPC is unreachable) so the existing fast offline suite (72 tests) is untouched.

**Tech Stack:** surfpool 1.0.0 (installed at `~/.local/bin/surfpool`); `solana`/`solana-test-validator` CLI available; the SDK (TS, `@solana/web3.js@3.0.0-rc.2`, vitest); the Rust runner (`runner/`); the on-chain program (`programs/kassandra/`, built to `target/deploy/kassandra_program.so` via `just build`). The program is the source of truth; the runner gets a SMALL additive change (base-URL override) — anything else in the program/runner is read-only (STOP+report on a genuine bug).

## Recon already done (master)
- `surfpool start`: RPC :8899, WS :8900, `--no-tui` (headless log stream), `--block-production-mode transaction` (deterministic block-per-tx), `-n/--network mainnet|devnet` OR `-u/--rpc-url <datasource>` to fork (lazily pulls accounts from the datasource — this is how MetaDAO's deployed programs become available). Also `surfpool mcp`, `run`/`ls` (txtx runbooks).
- Runner: `MESSAGES_URL` is a hardcoded const in `runner/src/anthropic.rs:77` — needs an override to point at the mock server. The `--mock` CLI path uses `MockProvider` (fixed output) — NOT what we want here; we want the real `AnthropicProvider` against the mock server.
- The SDK speaks standard RPC, so it drives surfpool directly; the litesvm interop bridge is litesvm-only — the surfpool harness sends real RPC transactions (web3.js v3 sign+send to localhost:8899).

## Open questions for the harness to resolve in T1 (don't guess)
- How to load the LOCAL Kassandra `.so` into surfpool: `solana program deploy --url http://localhost:8899 target/deploy/kassandra_program.so` against the running simnet, vs a surfpool cheatcode/runbook, vs a genesis/account-load flag. Determine empirically + document.
- How to set up accounts (SPL mints, funded payers, token accounts, and — for the core path — an Oracle/Proposer where seeding is needed): real instructions over RPC where possible; surfpool cheatcode RPC methods (e.g. `surfnet_setAccount`/clone-program, if they exist) where seeding is needed. Determine the available cheatcodes + document.
- Whether the CORE path can run on a STANDALONE simnet (no fork, hermetic, fast) and only the challenge-market path needs `--network mainnet`. Prefer standalone for the core.
- surfpool process lifecycle from a test: spawn (`--no-tui`), wait-for-RPC-ready (poll getHealth), teardown; or assume an externally-running surfpool + skip-if-unreachable. Gate the suite either way.

## Tasks

### T1 — Recon + harness scaffold + runner base-URL override + smoke (DO FIRST; stop-and-report if surfpool can't be driven headless)
- **Runner change (small, additive):** make the Anthropic endpoint overridable — add a base-URL override read from env (`ANTHROPIC_BASE_URL`, falling back to the existing const) and/or an `AnthropicProvider` constructor/CLI flag; keep the default `https://api.anthropic.com/v1/messages` unchanged. Add a unit test that the override is honored. Keep the runner's existing 71-test suite green. (This is the only runner edit — do not change provider semantics, hashing, or the request body.)
- **Harness scaffold** (gated; reuse the SDK package — e.g. `sdk/test/surfpool/` helpers + a `surfpool-e2e.test.ts`, or a sibling `e2e/` that imports the built SDK — pick + document): a `SurfpoolHarness` helper that spawns `surfpool start --no-tui --block-production-mode transaction` (standalone simnet for the core path), polls RPC `getHealth`/`getVersion` until ready (timeout), deploys `target/deploy/kassandra_program.so` (the determined method) at the program ID `KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY`, and tears down on completion. The suite must be GATED — skip (not fail) if surfpool isn't installed/reachable, behind an env flag (e.g. `KASSANDRA_E2E=1`) or `pnpm test:e2e` — so the default `pnpm test` stays fast + offline + green.
- **Smoke test:** surfpool up → program deployed → build `initProtocol` via the SDK → sign+send over RPC → assert success → fetch + `decodeProtocol` the Protocol account → assert admin/mints. Proves the whole rig (surfpool + deploy + SDK-over-RPC + decode).
- Write `NOTES-surfpool.md`: how to deploy the .so, the account-setup mechanism (cheatcodes available?), standalone-vs-fork, process lifecycle, how to run the gated suite. Commit `feat(e2e): runner base-URL override + surfpool harness scaffold + smoke`.

### T2 — Mock Anthropic server + runner-against-mock (real provider path)
- A local **mock Anthropic HTTP server** (TS, in the harness) serving `POST /v1/messages` that returns a valid Anthropic Messages response whose content is the structured-output JSON the runner expects (`{"option_index": N}`), with `stop_reason: "end_turn"`, the `model` field echoed, configurable per request (the test sets which option N + can simulate a `refusal`). Match the response shape the runner's `parse_messages_response` consumes (read `runner/src/anthropic.rs`).
- An integration test: start the mock server → invoke the runner CLI `run` (NOT `--mock`) with the base-URL pointed at the mock server + a config (interpretation + options + zero or mock-served facts) → assert it emits the expected `option_index` + the claim metadata (the three hashes + 97-byte payload), proving the REAL `AnthropicProvider` HTTP+parse path works against controllable responses. Also a refusal arm (mock returns refusal → runner errors clearly).
- Commit `feat(e2e): mock Anthropic server + runner real-provider integration`.

### T3 — Core lifecycle E2E on surfpool (runner-in-the-loop, mock AI)
- The headline test(s): drive the full lifecycle on surfpool via SDK instructions over RPC, with the runner (real provider → mock server) producing the AI claim:
  - **Uncontested resolve arm:** `initProtocol` → `createOracle` → `propose`×N (same option) → warp/advance → `finalizeProposals` → assert Oracle `Resolved` + resolved_option (decode over RPC).
  - **Dispute → AI-claim arm:** create → propose conflicting options → finalizeProposals (→ FactProposal) → `submitFact` → advance → `voteFact` → `finalizeFacts` → **run the offchain runner** (mock AI returns a chosen option for the disputed question) → `submitAiClaimFromRunner` (the bridge) → `finalizeAiClaims` → `finalizeOracle` → assert Resolved with the AI-claim's option, and decode the on-chain AiClaim to match the runner's metadata.
  - Use real instructions where the phase gating allows; where driving a precondition fully is impractical on a live validator, seed via surfpool cheatcodes (if available) + document. Handle surfpool's clock/time advance for the phase windows (determine the mechanism — block-production-mode transaction + a clock cheatcode, or real time; document).
- Assert on-chain state by fetching + decoding accounts over RPC (the SDK decoders). Commit `test(e2e): surfpool core lifecycle with runner-in-the-loop (mock AI)`.

### T4 — Challenge-market push (forked MetaDAO) + docs + covered-vs-deferred
- Switch the harness to a **forked** surfpool (`--network mainnet`) so MetaDAO's conditional-vault + AMM (+ futarchy/Meteora/Squads) programs are fetchable. Attempt the challenge path: `openChallenge`/`settleChallenge` against the forked MetaDAO programs — composing the market accounts the SDK leaves caller-supplied. Push as far as tractable: at minimum confirm the forked programs load + a challenge can be opened; ideally a settle. **Honestly document** whatever proves intractable offline (market composition complexity, account fetching, non-determinism) as a deferral — do NOT fake a pass.
- `docs` / a `README` for the surfpool E2E: prerequisites (surfpool installed, `just build`, network access for the fork), how to run (`KASSANDRA_E2E=1 pnpm test:e2e` or the script), what's covered (core lifecycle + runner + mock AI) vs deferred (the challenge-market/futarchy extent reached). Append the final covered-vs-deferred note to this plan.
- Commit `test(e2e): challenge-market push on forked MetaDAO + docs + covered-vs-deferred`.

## Out of scope / deferred
- Making the surfpool suite part of the default `pnpm test` (it's gated/opt-in — heavier + network for the fork).
- Live devnet/mainnet submission with real funds; a real (non-mock) Anthropic call in E2E (the mock server is the point — deterministic + free; the runner's live Anthropic test already exists, `#[ignore]`).
- Full futarchy-governance E2E (config/dead-end via the DAO) unless T4 reaches it; full challenge-market settlement if intractable on a forked validator (document).

## Execution note
After each task: the relevant build/test green; the GATED suite must not break the default offline `pnpm test` (72) or the runner's `cargo test` (71). T1 (can we drive surfpool headless + deploy the .so + SDK-over-RPC) is the make-or-break — stop-and-report if not. The runner edit is additive (base-URL override) only. Prefer a standalone simnet for the core path (hermetic) and fork only for T4. Append a T1–T4 delta log here.

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

## Delta log

### T1 — DONE (2026-06-30)
- **Make-or-break PROVEN.** surfpool can be driven headless from a test, the
  local `.so` deploys at the FIXED program id, and an SDK-built tx is accepted
  over RPC + the Protocol decodes. No blockers.
- **Runner base-URL override (additive, the only runner edit).**
  `runner/src/anthropic.rs`: reads `ANTHROPIC_BASE_URL` (env); set non-empty →
  treated as the API base, `/v1/messages` appended (mirrors the official SDK);
  unset → the pinned `https://api.anthropic.com/v1/messages` default, UNCHANGED.
  Added `AnthropicProvider::with_base_url(key, base)` + `messages_url()`
  accessor + pure `resolve_messages_url()`. 3 new unit tests (default unchanged,
  override appends path + trims slash/whitespace). Provider semantics / request
  body / hashing untouched. `cargo test -p kassandra-runner` green (68 + 5 + 1,
  1 `#[ignore]` live); clippy + fmt clean. No CLI flag added (env is the natural
  channel for the T2 subprocess; kept minimal).
- **Deploy method (the key recon).** `surfnet_setAccount` writes the ELF as a
  non-upgradeable BPFLoader2 program account at `KassVxv...` (data is **hex**,
  not base64); surfpool JIT-executes it. `solana program deploy` can't — the
  build keypair is random, not the fixed vanity id. Cheatcodes catalogued for
  T3 (`surfnet_timeTravel` for phase windows, `setTokenAccount`, `setSupply`,
  `cloneProgramAccount` for T4, etc.). standalone simnet still hits a default
  mainnet datasource at boot (network needed to boot) but the core path is
  fully local. All in `NOTES-surfpool.md`.
- **Harness + smoke (gated).** Reused the SDK package: `sdk/test/surfpool/`
  (`harness.ts` `SurfpoolHarness` spawn→wait-health→deploy→teardown, +
  `surfpool-smoke.test.ts`). Gated by `sdk/vitest.config.ts` (excludes
  `test/surfpool/**` unless `KASSANDRA_E2E=1`) + a `pnpm test:e2e` script + a
  `skipIf` for missing surfpool/`.so`. Default `pnpm test` = **72**, offline, no
  surfpool spawn (verified). `pnpm test:e2e` = 73 incl. the real surfpool smoke;
  teardown leaves port 8899 free.

### T2 — DONE (2026-06-30)
- **Mock Anthropic server** (`sdk/test/surfpool/mock-anthropic.ts`,
  `MockAnthropic`): a `node:http` server on an ephemeral localhost port serving
  `POST /v1/messages` (the runner appends `/v1/messages` to `ANTHROPIC_BASE_URL`
  per T1, so the base is `http://127.0.0.1:<port>`). Returns the EXACT shape
  `parse_messages_response` consumes:
  - success → `{ id, type:"message", role:"assistant", model:<echoed request
    model>, content:[{ type:"text", text:"{\"option_index\":N}" }],
    stop_reason:"end_turn", usage }` — the single `text` block IS the
    structured-output JSON the runner captures verbatim → `parse_option_index`.
  - refusal → `{ ..., content:[], stop_reason:"refusal",
    stop_details:{ type:"refusal", category, explanation } }` — the runner
    checks `stop_reason=="refusal"` first and bails.
  - Configurable per scenario: `setOption(N, model?)` / `setRefusal(cat, expl)`;
    captures request bodies for assertions. Clean `stop()`.
- **Integration test** (`sdk/test/surfpool/runner-mock-anthropic.test.ts`,
  gated by `KASSANDRA_E2E=1`; skips if the debug binary is missing — does NOT
  need surfpool): spawns `target/debug/kassandra-runner run --config <tmp>`
  (NOT `--mock`) with `ANTHROPIC_BASE_URL=<mock>` + dummy `ANTHROPIC_API_KEY` +
  a 2-option ZERO-fact config (no real fact-fetch HTTP). Asserts the mock's
  chosen `option_index` flowed through the REAL provider, the three hashes are
  32-byte hex, and `submit_ai_claim_payload_hex` is 97 bytes (= model_id ++
  params_hash ++ io_hash ++ option). Refusal arm: mock refuses → runner exits
  non-zero, emits no claim, stderr contains "refusal".
- No runner/program edits (T1's env override was sufficient). Default
  `pnpm test` = **72** offline (verified, never imports the mock/test); typecheck
  clean; `cargo test -p kassandra-runner` green. `pnpm test:e2e` now also runs
  the runner↔mock integration (2 tests, no surfpool spawn needed for them).

### T3 — DONE (2026-06-30)
- **Time mechanism PROVEN.** `surfnet_timeTravel({absoluteSlot})` advances the
  Clock sysvar's `unix_timestamp` (the value the program's `now()` reads) at
  ~0.4 s/slot, so jumping the absolute slot forward crosses `phase_ends_at`.
  Only `absoluteSlot` works (`absoluteTimestamp` → Internal error, no
  `surfnet_setClock`, `absoluteEpoch` is destructive). The harness gained
  `clockUnixTimestamp()` / `currentSlot()` / `advanceToUnix(target)` (read the
  Clock sysvar + re-jump until `unix_timestamp >= target`) + `confirmSignature()`
  (poll `getSignatureStatuses`). Wall-clock time never moves the chain clock
  (block-per-tx, no live clock), so the long runner-subprocess step inside the
  AiClaim window is safe. Documented in `NOTES-surfpool.md` (T3 RESOLVED).
- **Runner helper factored** (T2 reviewer flag). `sdk/test/surfpool/run-runner.ts`:
  `RUNNER_BIN` / `runnerAvailable()` / `runRunner(configPath, baseUrl)` (the
  `ANTHROPIC_BASE_URL` + non-empty key + `KASSANDRA_RUNNER_MOCK=""` real-provider
  env trick) + `RunOutput` + `writeRunnerConfig()`. `runner-mock-anthropic.test.ts`
  now imports it (no behavior change; still green).
- **Lifecycle E2E** (`sdk/test/surfpool/lifecycle-e2e.test.ts`, gated by
  `KASSANDRA_E2E=1`; skips if surfpool/`.so`/runner absent; its own port 8901):
  - **Uncontested arm:** init → create_oracle(3 opts) → propose×3 (same option,
    fresh funded authorities) → advance past the proposal window → finalize_proposals
    → `decodeOracle` over RPC asserts `Resolved` + `resolvedOption == agreed` +
    the stake vault holds Σ bonds. ALL real ix.
  - **Dispute → AI-claim arm (headline, runner in the loop):** create_oracle(2) →
    propose×2 CONFLICTING (opts 0/1) → finalize_proposals (→ FactProposal) →
    submit_fact → advance → advance_phase (→ FactVoting) → vote_fact (approve 2000,
    clears 2/3 quorum) → advance → finalize_facts (→ AiClaim) → **for each
    proposer: invoke the REAL runner** (genuine `AnthropicProvider` → the T2 mock,
    `setOption(0)`, config carrying the oracle+proposer so the bridge's
    `claim_pda_seeds` cross-check is exercised) → `submitAiClaimFromRunner` →
    submit over RPC → finalize_ai_claims (→ Challenge) → finalize_oracle (→ Resolved).
    Asserts `Oracle.phase == Resolved` + `resolvedOption == 0` (the AI's option),
    AND `decodeAiClaim` of the on-chain claim matches the runner's exact
    model_id/params_hash/io_hash/option + oracle/proposer/authority. Proves
    runner(mock AI) → SDK bridge → real program on surfpool → resolved oracle.
  - **Real vs seeded:** the ENTIRE phase chain (propose…finalize_oracle) is driven
    by REAL instructions over RPC — NO `setAccount` seeding of any Kassandra
    program account or phase. The only fabricated state is SPL plumbing (KASS/USDC
    mints + funded creator/proposer/submitter/voter KASS token accounts), packed
    as canonical SPL bytes token-program-owned, exactly as the litesvm `e2e.test.ts`
    / Rust `common/mod.rs` fund them; the program's own SPL CPIs run against the
    real Token program (auto-available on surfpool). The KASS mint is given supply
    `1e18` and the creator a large balance so create_oracle's dynamic EMA fee
    `Burn` (0 on the genesis oracle, positive on the 2nd in the shared protocol —
    cf. Rust `e2e_second_oracle_fee_is_burned`) succeeds.
- No program/runner edits (T1's env override sufficed). Default `pnpm test` =
  **72** offline (verified). `KASSANDRA_E2E=1 pnpm test:e2e` = **77** (72 + smoke +
  2 runner↔mock + 2 lifecycle); both lifecycle arms green. typecheck clean.

### T4 — DONE (2026-06-30)
- **Mainnet fork PROVEN.** `SurfpoolHarness.start({ fork: "mainnet" })` (additive
  `--network mainnet`) boots a forked simnet on this machine; all five MetaDAO
  program ids fetch over RPC as `executable` BPF-upgradeable programs:
  conditional-vault `VLTX1ish…`, AMM v0.4 `AMMyu265…`, futarchy v0.6 `FUTARELBf…`,
  Meteora DAMM v2 `cpamdpZC…`, Squads v4 `SQDS4ep6…`. They don't just LOAD — the
  conditional-vault **EXECUTES**: a real `initialize_question` CPI over RPC
  creates the on-chain `Question` (decoded oracle/num_outcomes match).
- **T3 foot-gun re-verified on the fork.** `surfnet_timeTravel({absoluteSlot})`
  still advances the Clock `unix_timestamp` at ~0.4 s/slot on the fork, and
  `getSlot` returns the (monotonic) absolute slot — exactly the two values
  `advanceToUnix` uses (it never reads the Clock's `slot` field, which surfpool
  rewrites to a within-epoch index after a jump). So the phase-window mechanism
  works unchanged in fork mode.
- **A challenge market is OPENED on the fork.** `challenge-market-e2e.test.ts`
  drives the FULL dispute core to `Challenge` over RPC (real instructions, clock
  via timeTravel), COMPOSES the MetaDAO market with real CPIs (`initialize_
  question` + KASS/USDC `initialize_conditional_vault`), and calls the Kassandra
  `openChallenge`. Its **program-signed `split_tokens` CPI executes against the
  forked conditional-vault**, physically splitting the proposer's 1-KASS bond into
  pass/fail conditional KASS (each == bond, underlying in the vault). Asserted:
  `Market` PDA created + bound, `ai_claim.challenged` flipped, USDC escrow funded
  with the on-chain-computed amount (`bond×twap/scale`), `open_challenge_count==1`.
  `kass_price` is fed a fabricated futarchy-owned `Dao` blob (mirrors the Rust
  `bless_kass_price`); the pass/fail AMMs are placeholder AMM-owned accounts
  (`open_challenge` checks only AMM ownership).
- **Stopping point (deferred, documented).** `settle_challenge` is NOT driven on
  the fork: it reads a **swap-driven AMM TWAP**, which needs two live MetaDAO AMM
  pools built + cranked over RPC (delayed-twap, ≥150-slot windows) — substantial
  and non-deterministic on a forked validator. The full real-AMM settle (redeem +
  directional fees + KASS/USDC conservation) is already covered in the LiteSVM
  Rust suite (`tests/challenge_e2e.rs`). Live-cluster submission + full futarchy
  governance also deferred. See `sdk/test/surfpool/README.md` for the full
  covered-vs-deferred.
- No program/runner edits (T1's env override sufficed; the harness gained only an
  additive `fork` option). Default `pnpm test` = **72** offline (verified).
  `KASSANDRA_E2E=1 pnpm test:e2e` = **80** (72 + smoke + 2 runner↔mock + 2
  lifecycle + 3 challenge-market), all green. typecheck clean.

### Final — surfpool E2E: covered vs deferred
- **Covered (real, over RPC):** the full core oracle lifecycle on a standalone
  simnet — uncontested resolve AND dispute→AI-claim with the REAL off-chain runner
  (genuine `AnthropicProvider`) in the loop against a controllable mock Anthropic
  server, every phase driven by real instructions (only SPL mints/token-accounts
  fabricated), asserted by decoding on-chain state; the runner's real
  HTTP+parse path (success + refusal); and on a MAINNET FORK, the MetaDAO programs
  loading + the conditional-vault executing + a challenge market being OPENED via
  the Kassandra `openChallenge` (real program-signed `split_tokens` CPI into the
  forked vault).
- **Deferred (documented, not faked):** `settle_challenge` on the fork (real
  swap-driven AMM TWAP — covered in the LiteSVM Rust suite instead); `kass_price`
  from a genuine live futarchy DAO (a fabricated DAO blob is used); live
  devnet/mainnet submission with real funds; a real (non-mock) Anthropic call;
  full futarchy-governance E2E.

## Execution note
After each task: the relevant build/test green; the GATED suite must not break the default offline `pnpm test` (72) or the runner's `cargo test` (71). T1 (can we drive surfpool headless + deploy the .so + SDK-over-RPC) is the make-or-break — stop-and-report if not. The runner edit is additive (base-URL override) only. Prefer a standalone simnet for the core path (hermetic) and fork only for T4. Append a T1–T4 delta log here.

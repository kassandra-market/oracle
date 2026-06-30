# Kassandra surfpool E2E (gated)

End-to-end tests that drive the **real** Kassandra oracle lifecycle against a
**real RPC validator** ([surfpool](https://github.com/txtx/surfpool)), built by
the merged SDK's web3.js v3 instruction builders and sent as real transactions,
with the **off-chain runner in the loop** producing AI claims from a controllable
**mock Anthropic server**. The forked-MetaDAO challenge-market path is pushed as
far as tractable.

This suite is **GATED / opt-in**: the default `pnpm test` (72 tests) stays fast,
offline, and never spawns surfpool. The E2E suite only runs under
`KASSANDRA_E2E=1` (see `sdk/vitest.config.ts`, which excludes
`test/surfpool/**` otherwise) and **skips cleanly** (does not fail) when surfpool
/ the program `.so` / the runner binary are absent.

## Prerequisites

1. **surfpool** on `PATH` (or set `SURFPOOL_BIN`). Tested against surfpool
   `1.0.0` (`~/.local/bin/surfpool`).
2. **The program artifact:** `just build` → `target/deploy/kassandra_program.so`
   (deployed at the fixed program id via the `surfnet_setAccount` cheatcode).
3. **The runner binary** (for the lifecycle / runner-against-mock arms):
   `cargo build -p kassandra-runner` → `target/debug/kassandra-runner`.
4. **Network access.** surfpool 1.0.0 always boots against a datasource
   (mainnet by default), so even the standalone core path needs network at boot.
   The **challenge-market arm forks mainnet** (`--network mainnet`) and lazily
   fetches the deployed MetaDAO programs over RPC — so it needs network and is
   **slower** than the local core path.

## How to run

```sh
cd sdk
pnpm test          # default: 72 tests, offline, no surfpool
KASSANDRA_E2E=1 pnpm test:e2e   # gated E2E: 80 tests (≈20s; spawns surfpool, needs network)
# a single arm:
KASSANDRA_E2E=1 pnpm exec vitest run test/surfpool/challenge-market-e2e.test.ts
```

The harness (`harness.ts` `SurfpoolHarness`) spawns `surfpool start --no-tui
--block-production-mode transaction --no-deploy [--network mainnet]`, polls
`getHealth`, writes the `.so` at the fixed id, and tears the process down. Each
suite owns a distinct port (smoke 8899, lifecycle 8901, challenge 8920) so they
never collide.

## Files

| file | what |
| --- | --- |
| `harness.ts` | `SurfpoolHarness` (spawn → wait → deploy → teardown), cheatcode helpers (`setAccount`, `airdrop`, `timeTravel`/`advanceToUnix`), SPL byte fabrication. `fork: "mainnet"` boots a forked simnet (T4). |
| `mock-anthropic.ts` | A local `node:http` Anthropic Messages mock (`POST /v1/messages`) returning the exact shape the runner's `parse_messages_response` consumes; `setOption(N)` / `setRefusal(...)`. |
| `run-runner.ts` | Invoke the real runner binary (`AnthropicProvider` → the mock) and capture the claim metadata. |
| `surfpool-smoke.test.ts` | T1: surfpool up → `.so` deployed → `initProtocol` over RPC → decode Protocol. |
| `runner-mock-anthropic.test.ts` | T2: the real runner against the mock (success + refusal). No surfpool. |
| `lifecycle-e2e.test.ts` | T3: full core lifecycle on a standalone simnet — uncontested resolve + dispute→AI-claim (runner in the loop). |
| `challenge-market-e2e.test.ts` | T4: the challenge-market path against **forked-mainnet** MetaDAO programs. |

## Covered vs deferred

### Covered (proven, real over RPC)

- **Core lifecycle, fully real (T3).** On a standalone simnet, every phase is
  driven by REAL Kassandra instructions over RPC — no `setAccount` seeding of any
  Kassandra program account or phase. Two arms:
  - **Uncontested resolve:** `initProtocol → createOracle → propose×3 (same
    option) → finalizeProposals` ⇒ Oracle `Resolved` + the agreed option (decoded
    over RPC); the stake vault holds Σ bonds.
  - **Dispute → AI-claim (runner in the loop):** `create → propose×2 conflicting
    → finalizeProposals → submitFact → advancePhase → voteFact → finalizeFacts →`
    **run the real runner** (genuine `AnthropicProvider` → the mock server,
    `setOption(N)`) `→ submitAiClaimFromRunner → finalizeAiClaims →
    finalizeOracle` ⇒ Oracle `Resolved` with the AI's option, and the on-chain
    `AiClaim` decodes to the runner's exact model_id/params_hash/io_hash/option.
  - The only fabricated state is SPL plumbing (mints + funded KASS token
    accounts), packed as canonical SPL bytes; the program's own SPL CPIs run
    against the real Token program. Phase windows are crossed with
    `surfnet_timeTravel` (it moves the Clock `unix_timestamp` at ~0.4 s/slot, the
    value the program's `now()` reads).
- **Runner real-provider path (T2).** The real `AnthropicProvider` HTTP + parse
  path is exercised against controllable mock responses (success + refusal).
- **Challenge-market on FORKED MetaDAO (T4).**
  - **Programs load.** All five MetaDAO program ids (conditional-vault `VLTX1ish…`,
    AMM v0.4 `AMMyu265…`, futarchy v0.6 `FUTARELBf…`, Meteora DAMM v2, Squads v4)
    are fetched from the mainnet fork as `executable` BPF-upgradeable programs.
  - **Conditional-vault EXECUTES.** A real `initialize_question` CPI against the
    forked vault creates the on-chain `Question` (decoded `oracle`/`num_outcomes`
    match) — far past "program not found".
  - **A challenge is OPENED.** The full dispute core is driven to `Challenge`,
    the MetaDAO market is COMPOSED over RPC (real `initialize_question` +
    KASS/USDC `initialize_conditional_vault` CPIs), and the Kassandra
    `openChallenge` instruction is sent. Its **program-signed `split_tokens`
    CPI runs against the forked conditional-vault**, physically splitting the
    proposer's KASS bond into pass/fail conditional KASS (each == bond, underlying
    in the vault). Asserted: `Market` PDA created + bound, `ai_claim.challenged`
    flipped, USDC escrow funded with the on-chain-computed amount,
    `open_challenge_count == 1`.

### Deferred (NOT asserted — documented honestly)

- **`settle_challenge` on the fork.** Settlement reads a **swap-driven AMM
  TWAP**: it requires building TWO live MetaDAO AMM pools (`create_amm` +
  `add_liquidity`), seeding their conditional-token reserves, executing a real
  `swap`, and cranking the delayed-twap oracle across ≥150-slot windows — all
  over RPC on a fork. In `open_challenge` the pass/fail AMMs only need to be
  **owned by the AMM program**, so the T4 test uses placeholder AMM-owned
  accounts and stops at a successfully **opened** market. The complete settle
  (real AMM pools + TWAP + redeem + directional fees + KASS/USDC conservation) is
  covered exhaustively in the LiteSVM Rust suite
  (`programs/kassandra/tests/challenge_e2e.rs`, against the bundled MetaDAO
  fixtures) and is left to a future surfpool pass — driving the full real-AMM
  TWAP production over a forked RPC validator is substantial and non-deterministic.
- **`kass_price` from a live futarchy DAO.** The escrow is sized from a
  futarchy spot TWAP. The T4 test fabricates a futarchy-owned `Dao` blob with a
  deterministic TWAP (mirrors the Rust harness `bless_kass_price`) rather than
  driving a real futarchy DAO creation. Reading a genuine on-chain DAO's spot
  oracle from the fork is deferred.
- **Live-cluster submission.** No devnet/mainnet submission with real funds; no
  real (non-mock) Anthropic call (the runner's live test already exists, `#[ignore]`).
- **Full futarchy-governance E2E** (config / dead-end via the DAO, Squads).
- **Making the suite part of the default `pnpm test`** — it is intentionally
  gated (heavier + network for the fork).

# Categorical Markets (N>2) via per-outcome binary sub-markets

**Goal:** Support Kassandra oracles with `options_count > 2` by modeling a categorical question as **N independent binary sub-markets**, one per outcome — each a "will the oracle resolve to option *i*? YES/NO" market that **reuses the entire existing binary stack unchanged** (cYES/cNO vault + single pool, activate, contribute, cancel, refund, claim_lp, collect_fee, redeem). The only new mechanics are: a market binds to `(oracle, outcome_index)`, and resolution compares the oracle's `resolved_option` to that index.

**Confirmed model (user):** per-outcome binary sub-markets (the design doc's "N independent outcome pools"). NOT linked c_i/KASS pools. Sub-markets are independent (not AMM-arbitrage-linked); the app groups them into a categorical view. Binary is the special case `outcome_index = 0` on a 2-option oracle.

**Why it's small:** the conditional vault + AMM are always a 2-outcome (YES/NO) structure regardless of the oracle's N — so activate/trade/claim/fee never change. Only market identity (PDA keyed by outcome) + the resolve comparison change.

---

## Semantics
- A sub-market binds to `(oracle, outcome_index)` where `0 <= outcome_index < oracle.options_count`.
- **YES** = the oracle resolves to `outcome_index`. **NO** = it resolves to any other option.
- resolve: `resolved_option == market.outcome_index → [1,0]` (YES pays); else `→ [0,1]` (NO pays). `InvalidDeadend → [1,1]` (void) — unchanged.
- Multiple sub-markets can bind to one oracle (distinct `outcome_index`), so the market PDA must be keyed by outcome.

## Task 1 — Program

### State + errors
- `Market` (`src/state.rs`): add `outcome_index: u8` (fits the tail padding; keep LEN 400 if possible — re-pin `tests/state_layout.rs`; if it doesn't fit, grow + re-pin). Snapshot at create.
- `error.rs`: `InvalidOutcome = 19` (next free; no renumbering).

### `create_market` (`processor/create_market.rs`)
- **Market PDA seed** changes: `[b"market", oracle]` → `[b"market", oracle, [outcome_index]]` (one sub-market per outcome per oracle). This is the derivation the program does + validates.
- Payload gains `outcome_index: u8` (extend PAYLOAD_LEN + parse).
- Drop the `options_count == 2` (`NotBinary`) requirement → the oracle already guarantees `options_count >= 2`; just require `outcome_index < oracle.options_count` else `InvalidOutcome`.
- Store `outcome_index` in `Market`.
- Everything else (escrow, contribution, fee snapshot) unchanged. `NotBinary` becomes unused (keep the variant, no renumber).

### `resolve_market` (`processor/resolve_market.rs`)
- Replace the hardcoded `resolved_option == 0 → [1,0]`, `== 1 → [0,1]` with: `resolved_option == market.outcome_index → [1,0]`, else `→ [0,1]` (for the `Resolved` phase). Reject `resolved_option >= oracle.options_count`? The oracle guarantees it's valid; keep a defensive check → `InvalidAccount` if out of range. `InvalidDeadend → [1,1]` unchanged. The fee-collected stamping is unchanged.

### Everywhere else: UNCHANGED
- contribute/cancel/refund/activate/claim_lp/collect_fee/redeem take the market as an account (they read `market.oracle`, never re-derive the market from the oracle), so the seed change doesn't touch them. Confirm this by grepping for `b"market"` derivations — only `create_market` + the SDK `pda.market` should build it.

### Tests (`tests/`)
- `state_layout`: new Market offsets/LEN with `outcome_index`.
- `create_market`: an `options_count = 3` oracle, create sub-markets at `outcome_index` 0/1/2 (distinct market PDAs, all succeed); `outcome_index >= options_count` → `InvalidOutcome`; two markets at the same `(oracle, outcome_index)` → the duplicate guard fires.
- `resolve_market`: categorical resolution — a 3-option oracle resolves to option 1; the `outcome_index=1` sub-market resolves YES `[1,0]`, the `outcome_index=0` and `=2` sub-markets resolve NO `[0,1]`; void path.
- Thread `outcome_index = 0` through the Rust harness `create_market` helper + every existing caller/test (binary markets = outcome_index 0) so they keep passing with the new PDA seed. Add a `create_market_full`/param variant for explicit outcome_index.
- `lifecycle_active.rs`: keep binary (outcome_index 0) green; optionally add a categorical lifecycle (create + activate + resolve a non-zero outcome sub-market).

Two-stage review (resolve/create touch settlement logic). Commit `feat: categorical markets — per-outcome binary sub-markets (outcome_index)`.

## Task 2 — sdk-rs + TS SDK
- `sdk-rs/src/pda.rs`: `market(oracle, outcome_index)` seed. `ix.rs::create_market`: add `outcome_index` arg + payload byte. Thread `outcome_index=0` through the Rust harness usage.
- TS SDK: `pda.market(oracle, outcomeIndex)`; `decodeMarket` reads `outcomeIndex`; `createMarket` builder gains `outcomeIndex`; `MarketError.InvalidOutcome=19`; parity (new Market size if changed, InvalidOutcome, error count); re-pin. Update the LiteSVM + surfpool harnesses' `createMarket`/`pda.market` callers (default outcome_index 0). Add a categorical SDK test (create sub-markets at multiple indices on a 3-option fabricated oracle; decode outcomeIndex). `pnpm --filter @kassandra-market/sdk test` + typecheck green; keep the gated surfpool lifecycle compiling (outcome_index 0).

## Task 3 — App
- **Create-market:** for an oracle with `options_count > 2`, the form lets the creator pick which `outcome_index` this sub-market is for (or, nicer: a "create categorical market" flow that creates one sub-market per outcome in sequence — optional; at minimum expose outcome_index). Off-chain outcome LABELS aren't on-chain (bound by prompt_hash) — accept optional user-entered labels stored client-side, or just show "Outcome i".
- **Categorical view:** `data/markets.ts` groups markets by `oracle`; a market list card / detail for a categorical oracle shows each outcome sub-market's YES probability (from its pool reserves) as the implied chance of that outcome, side by side. A single-outcome (binary) oracle renders as today.
- **Market detail:** show "Outcome {i} of {options_count}" and that YES = this outcome winning.
- e2e: seed a 3-option oracle + sub-markets in global-setup; a `categorical.spec.ts` asserting the grouped view renders the N outcomes and one sub-market trades/resolves. Keep existing specs green (binary = outcome_index 0).
- typecheck/build/lint/test green.

## Deferred
- Linked c_i/KASS categorical pools (arbitrage-summed prices) — rejected in favor of independent sub-markets.
- On-chain outcome labels (stay off-chain, bound by the oracle prompt_hash).
- A "one-click create all N sub-markets" batching flow (nice-to-have; per-outcome create suffices).

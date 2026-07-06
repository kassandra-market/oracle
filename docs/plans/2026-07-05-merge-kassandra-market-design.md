# Merging `kassandra-market` into `kassandra` — design

**Date:** 2026-07-05
**Goal:** Fold the `kassandra-market` prediction-market repo into the `kassandra`
oracle repo so there is a **single app** and a **single indexer** serving both.

## Context

`kassandra` is an AI-assisted optimistic oracle on Solana. `kassandra-market` is a
KASS-denominated AMM prediction market that is *resolved by* the Kassandra oracle —
i.e. market is a downstream consumer of oracle. Merging is natural: one repo owns
both on-chain programs, one dApp surfaces both, one indexer indexes both.

Both repos are structurally parallel monorepos (Pinocchio program + hand-written
TS SDK + Rust sdk-rs + Carbon indexer + Vite/React app), share the same design
system (fonts, Tailwind tokens, Layout/NavBar, provider shell, `lazyRoute`
pattern), and both indexers describe themselves as "sibling" Carbon-pipeline +
concurrent-axum mains.

## Decisions

- **Indexer:** deep-unify onto **Postgres** — one binary, one DB, one axum server.
- **Git:** clean copy on branch `merge/kassandra-market`, one merge commit. The old
  `kassandra-market` repo remains the historical record.

## Target layout (single repo)

```
programs/kassandra/            (oracle program — unchanged)
programs/kassandra-market/     (market program — copied in)
runner/                        (oracle AI runner — unchanged)
sdk-rs/                        (oracle rust sdk — unchanged)
sdk-rs-market/  (or crate kept as kassandra-market-sdk)  (market rust sdk — copied in)
sdk/                           (@kassandra/sdk — unchanged)
sdk-market/                    (@kassandra-market/sdk — copied in, pnpm workspace pkg)
indexer/                       (ONE binary: oracle events + market accounts → Postgres)
app/                           (ONE Vite app: /oracles* + /markets*, both SDKs)
docs-site/, docs/, scripts/, .surfpool/, Makefile, render.yaml
```

## Rust workspace

The oracle workspace is on the **granular solana v3** client stack (litesvm 0.13,
spl-token 9). The market program + its sdk-rs/tests are on **solana-sdk v2**
(litesvm 0.6, spl-token 6). Both programs share pinocchio 0.11 + solana-address v2
on-chain.

Strategy — keep the market test crates as a **v2 island** to avoid rewriting them:
- `workspace.dependencies` keeps the oracle set as the shared defaults.
- Market program `[dev-dependencies]` and `kassandra-market-sdk` pin
  `solana-sdk = "2"`, `spl-token = "6"`, `litesvm = "0.6"` **directly** (not
  `workspace = true`). Cargo resolves both majors side by side; no crate needs to
  unify a type across them.
- `solana-address` workspace dep gains the union of features
  (`bytemuck`, `curve25519`) both programs need.
- Workspace `members` gains `programs/kassandra-market` and the market sdk-rs crate.

The **shipped indexer stays v3-clean**: it depends only on
`kassandra-market-program` (pinocchio/bytemuck, no solana-sdk) for the `state`
structs + `ID`, and inlines the MetaDAO `AMM_ACCOUNT_DISCRIMINATOR` const — so it
never pulls market-sdk / solana-sdk v2.

## Unified indexer (Postgres)

One binary runs **two Carbon pipelines concurrently** and serves **one axum app**.

- **Oracle pipeline (unchanged):** RpcTransactionCrawler → decode instructions →
  `events` table + durable `indexer_cursor`.
- **Market pipeline (ported to Postgres):** GpaDatasource + RpcProgramSubscribe →
  decode accounts (Config/Market/Contribution) → new **`market_accounts`** table,
  slot-gated upsert, prune-on-reconcile.

New schema:
```sql
CREATE TABLE IF NOT EXISTS market_accounts (
  pubkey       TEXT PRIMARY KEY,
  account_type SMALLINT NOT NULL,      -- 1=Config 2=Market 3=Contribution
  market_ref   TEXT,                   -- Contribution.market (indexed lookup)
  slot         BIGINT NOT NULL,
  data         BYTEA NOT NULL          -- raw Pod bytes; decoded on read
);
```
Slot-gated upsert: `ON CONFLICT (pubkey) DO UPDATE ... WHERE market_accounts.slot <= EXCLUDED.slot`.
Prune: on a successful getProgramAccounts snapshot at `snapshot_slot`, delete
market_accounts absent from the snapshot with `slot <= snapshot_slot`. Reads decode
`data` via `kassandra_market_program::state` + the existing json DTOs.

**Router union (no path conflicts except `/health`):**
- Oracle: `/status`, `/events`, `/accounts/{pk}/events`, `POST /rpc`.
- Market: `/api/config`, `/api/markets`, `/api/markets/{pk}`, `/api/account/{pk}`,
  `/api/blockhash`, `/api/transaction`, `/api/transaction/{sig}`.
- Shared: `/health`, one CORS layer, one Postgres client, one RPC config.

Env: `DATABASE_URL`, `RPC_URL`/`SOLANA_RPC_URL` (oracle crawler + `/rpc`),
`SOLANA_WS_URL` (market subscribe), `INDEXER_RECONCILE_MS`, `PORT`, program ids.

## Single app

Fold market pages/components/data into `app/`, keep the oracle scaffolding as base:
- Add routes `/markets`, `/markets/new`, `/markets/:pubkey` to `App.tsx`.
- Add a **Markets** nav link beside **Oracles** in the Layout/NavBar.
- Copy market-specific `pages/Markets*.tsx`, `components/markets/**`, market
  `data/**` hooks+actions, market `lib` client helpers.
- Add `@kassandra-market/sdk` as an app dep (keep `@kassandra/sdk` too).
- Reconcile provider/config so the app hits **one indexer** serving both the
  `/rpc`+`/events` (oracle) and `/api/*` (market) route families — one proxy/base
  URL in `vite.config.ts` + `server.mjs`.

(Exact file inventory + config diffs come from the two inventory passes; the
landing page keeps the oracle Landing, extended with a Markets entry point.)

## Deploy / dev

- **render.yaml:** one app service + one indexer service (env for both programs:
  program ids, `RPC_URL`, `SOLANA_WS_URL`, `DATABASE_URL`, `ALLOWED_ORIGIN`) + one
  Postgres.
- **Makefile `make dev`:** boot surfpool (preloading oracle + market + MetaDAO
  conditional-vault/amm binaries) → deploy both programs → seed oracle + market
  config/markets → start the single indexer (both pipelines) → start the app →
  auto-connect funded wallet.
- **CI:** build+test both programs, build the one app, keep docs-site publish.

## Verification

`just build` (both `.so`), `cargo test` (both programs + unified indexer), app
`typecheck` + `build` + `vitest`, then a smoke of `make dev`.

# Kassandra indexer

A Solana indexing backend for the Kassandra program, built on the
[**Carbon**](https://github.com/sevenlabs-hq/carbon) framework. It crawls the
program's transactions into Postgres and serves a read-only JSON API.

**It catches up on any events it missed.** Progress is a durable cursor in
Postgres, not a live subscription — so every launch (including after downtime, a
redeploy, or a crash) resumes from the last processed point and back-fills
everything that happened while it was away.

## How catch-up works

- The datasource is Carbon's `RpcTransactionCrawler`, started with
  `until = <durable cursor>`. `getSignaturesForAddress(program, { until })`
  returns **every** signature newer than the cursor, paginated to chain head, so
  no transaction in the gap is skipped.
- Each Kassandra instruction becomes one row in `events`, inserted
  **idempotently** (`ON CONFLICT (signature, ix_index) DO NOTHING`) — re-crawling
  a range is harmless.
- The durable cursor is only **promoted forward once the backfill frontier goes
  stable** (the backlog has drained and we're caught up — see
  `state::frontier_stable`). It is never advanced mid-backfill, so a crash re-scans
  from the last safe cursor instead of skipping the un-backfilled older range.

That combination gives at-least-once, gap-free indexing across restarts.

## API

| Route | Description |
|---|---|
| `GET /health` | liveness |
| `GET /status` | program id, event count, current cursor |
| `GET /events?type=&account=&beforeSlot=&limit=` | recent events, filterable |
| `GET /accounts/{pubkey}/events` | events touching an account (e.g. an oracle) |
| `GET /api/markets` · `GET /api/markets/{pubkey}` | market-program accounts (config, markets, one market's detail) |
| `GET /api/markets/{pubkey}/candles?interval=&limit=` | OHLC candles of implied YES probability, from the ws-subscribed price series |

An `event` is one program instruction: `signature`, `slot`, `blockTime`,
`ixType` (e.g. `propose`, `submit_fact`, `open_challenge` — from the on-chain
`Ix` discriminant), `account0` (the primary subject, usually the oracle),
`accounts`, and the raw `dataBase64`.

## Configuration (env)

| Var | Required | Default | Notes |
|---|---|---|---|
| `RPC_URL` | ✅ | — | Solana RPC to crawl (mainnet/devnet or custom) |
| `DATABASE_URL` | ✅ | — | Postgres connection string |
| `PORT` | | `3000` | API port (Render sets this) |
| `COMMITMENT` | | `finalized` | or `confirmed` |
| `POLL_INTERVAL_MS` | | `10000` | crawler polling cadence |
| `PROMOTE_INTERVAL_MS` | | `30000` | cursor-promotion check cadence |
| `RUST_LOG` | | `info` | |

## Run locally

```bash
# Postgres (any) + a Solana RPC:
export DATABASE_URL=postgres://localhost/kassandra_indexer
export RPC_URL=https://api.devnet.solana.com
cargo run --release
# then:
curl localhost:3000/status
curl "localhost:3000/events?type=propose&limit=20"
```

`cargo test` covers the instruction decoder, the `Ix` discriminant→name map, and
the cursor-promotion predicate.

## Deploy (Render)

Provisioned by the repo's `render.yaml`: a managed Postgres
(`kassandra-indexer-db`) + this service as a **PRIVATE service** (`type: pserv`,
`runtime: rust`, `rootDir: indexer`) — it has **no public URL**. Only the
`kassandra-app` web service reaches it, over Render's private network, and
reverse-proxies `/indexer/*` to it (`app/server.mjs`); the browser calls the
app's own origin, never the indexer directly. `DATABASE_URL` is injected from the
database; set `RPC_URL` in the dashboard after the first deploy; it binds a fixed
internal `PORT` (10000) that the app's proxy resolves via `fromService`. `/health`
is the health check.

(The read API's CORS layer is only exercised in local dev/e2e, where the app dev
server points straight at the indexer cross-origin — in production the same-origin
proxy makes CORS moot.)

The crate is a **self-contained Cargo workspace** (its own `Cargo.lock`) so
Render builds only the indexer and does not pull in the program's pinned Solana
toolchain.

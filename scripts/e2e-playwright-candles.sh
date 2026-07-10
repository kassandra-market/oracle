#!/usr/bin/env bash
#
# e2e-playwright-candles.sh — run the CANDLE-chart browser E2E.
#
# Boots surfpool (with an explicit ws port), seeds an ACTIVE market with a live
# pool, runs the actual kassandra-indexer binary against surfpool + an ephemeral
# Postgres with SOLANA_WS_URL set (so the price subscriber `accountSubscribe`s the
# pool), fires real swaps that move the price, then drives the app's MarketDetail
# candlestick chart in a browser. Exercises the whole subscription pipeline:
# chain swap → ws accountSubscribe → Postgres market_price → /candles API → chart.
#
# Requirements: `surfpool` on PATH (or SURFPOOL_BIN), the Solana toolchain, a Rust
# toolchain, `just`, pnpm, and the Postgres binaries (initdb/pg_ctl; PG_BIN
# overrides their location).
#
# Usage: scripts/e2e-playwright-candles.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "==> [1/5] surfpool check"
if ! command -v surfpool >/dev/null 2>&1 && [ -z "${SURFPOOL_BIN:-}" ]; then
  echo "ERROR: surfpool not found on PATH (and SURFPOOL_BIN unset)." >&2
  exit 1
fi

echo "==> [2/5] postgres check"
if ! command -v initdb >/dev/null 2>&1 && [ -z "${PG_BIN:-}" ] \
   && [ ! -x /opt/homebrew/opt/postgresql@16/bin/initdb ] \
   && [ ! -x /opt/homebrew/opt/postgresql@15/bin/initdb ] \
   && [ ! -x /usr/lib/postgresql/16/bin/initdb ] \
   && [ ! -x /usr/lib/postgresql/15/bin/initdb ]; then
  echo "ERROR: postgres binaries (initdb) not found. Set PG_BIN to their directory." >&2
  exit 1
fi

echo "==> [3/5] build the program (.so), the SDKs, and the indexer binary"
# Always rebuild (incremental) so a stale .so after a program change isn't deployed.
just build
pnpm --filter @kassandra-market/oracles build >/dev/null
pnpm --filter @kassandra-market/markets build >/dev/null
cargo build --release --locked --manifest-path indexer/Cargo.toml

echo "==> [4/5] ensure Playwright + Chromium are installed"
if [ ! -d "node_modules/@playwright/test" ] && [ ! -d "app/node_modules/@playwright/test" ]; then
  pnpm --filter app add -D @playwright/test >/dev/null
fi
pnpm --filter app exec playwright install chromium >/dev/null

echo "==> [5/5] run the CANDLE Playwright E2E (surfpool + ws + pg + indexer + app)"
pnpm --filter app exec playwright test --config=playwright.candles.config.ts "$@"

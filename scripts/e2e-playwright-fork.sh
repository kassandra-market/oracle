#!/usr/bin/env bash
#
# e2e-playwright-fork.sh — run the FORKED challenge-market browser E2E.
#
# Unlike scripts/e2e-playwright.sh (a local, non-forked validator), this boots a
# surfpool validator FORKING MAINNET so MetaDAO's deployed programs
# (conditional_vault, amm v0.4, futarchy v0.6) are executable, in clock
# block-production mode (the AMM TWAP crank is slot-based). It then drives the
# app's full client-side challenge flow in a browser:
#
#   compose → open challenge → swap → crank TWAP → settle → close market
#
# Requirements: NETWORK (mainnet datasource for the fork), `surfpool` on PATH (or
# SURFPOOL_BIN), the Solana toolchain, `just`, and pnpm.
#
# Usage: scripts/e2e-playwright-fork.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "==> [1/4] surfpool check"
if ! command -v surfpool >/dev/null 2>&1 && [ -z "${SURFPOOL_BIN:-}" ]; then
  echo "ERROR: surfpool not found on PATH (and SURFPOOL_BIN unset)." >&2
  exit 1
fi

echo "==> [2/4] build the program (.so), the runner binary, and the SDK"
# Always rebuild (incremental) so a stale .so after a program change isn't deployed.
just build
# globalSetup seeds AI claims via the REAL runner (mock Anthropic).
if [ ! -x "target/debug/kassandra-runner" ]; then
  cargo build -p kassandra-runner
fi
pnpm --filter @kassandra-market/oracles build >/dev/null

echo "==> [3/4] ensure Playwright + Chromium are installed"
if [ ! -d "node_modules/@playwright/test" ] && [ ! -d "app/node_modules/@playwright/test" ]; then
  pnpm --filter app add -D @playwright/test >/dev/null
fi
pnpm --filter app exec playwright install chromium >/dev/null

echo "==> [4/4] run the FORKED Playwright E2E (mainnet fork + funded challenger + app)"
pnpm --filter app exec playwright test --config=playwright.fork.config.ts "$@"

#!/usr/bin/env bash
#
# e2e-playwright.sh — spin up every local node the Kassandra dApp needs and run
# the Playwright browser E2E against it with a script-funded mock wallet.
#
# What it does:
#   1. Ensures the SBF program (.so) and the TypeScript SDK are built.
#   2. Ensures @playwright/test + the Chromium browser are installed.
#   3. Runs `playwright test`, which:
#        - globalSetup boots a headless surfpool validator, deploys the program,
#          inits the protocol, mints KASS/USDC, GENERATES + FUNDS a wallet
#          keypair (SOL + KASS ATA), and seeds oracles;
#        - webServer starts the Vite dev server pointed at surfpool in e2e mode
#          (VITE_E2E=1) so the real-signing e2e wallet drives the funded key;
#        - the specs inject the funded keypair and drive the app in a browser;
#        - globalTeardown stops surfpool.
#
# Requirements: `surfpool` on PATH (or SURFPOOL_BIN), the Solana toolchain
# (`cargo build-sbf`), `just`, and pnpm. Usage: scripts/e2e-playwright.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "==> [1/4] surfpool check"
if ! command -v surfpool >/dev/null 2>&1 && [ -z "${SURFPOOL_BIN:-}" ]; then
  echo "ERROR: surfpool not found on PATH (and SURFPOOL_BIN unset)." >&2
  echo "Install it from https://surfpool.run or set SURFPOOL_BIN." >&2
  exit 1
fi

echo "==> [2/4] build the program (.so), the runner binary, and the SDK"
# Always rebuild (incremental) so a stale .so after a program change isn't deployed.
echo "    building programs with just build…"
just build
# The e2e seeds AI claims by running the REAL runner (mock Anthropic), not
# fabricated hashes — so the runner binary must exist for globalSetup.
if [ ! -x "target/debug/kassandra-runner" ]; then
  echo "    building the runner binary…"
  cargo build -p kassandra-runner
fi
pnpm --filter @kassandra-market/oracles build >/dev/null

echo "==> [3/4] ensure Playwright + Chromium are installed"
if [ ! -d "node_modules/@playwright/test" ] && [ ! -d "app/node_modules/@playwright/test" ]; then
  echo "    installing @playwright/test…"
  pnpm --filter app add -D @playwright/test >/dev/null
fi
pnpm --filter app exec playwright install chromium >/dev/null

echo "==> [4/4] run the Playwright E2E (surfpool + funded wallet + app)"
pnpm --filter app exec playwright test "$@"

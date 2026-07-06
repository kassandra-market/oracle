#!/usr/bin/env bash
#
# dev-up.sh — bring up the local dev stack: a seeded surfpool chain + the app.
#
#   dev-up.sh chain   boot surfpool + deploy + seed oracles, then HOLD (Ctrl-C to stop)
#   dev-up.sh app     run the app dev server against the local surfpool (funded e2e wallet)
#   dev-up.sh all     run the seeded chain AND the app together (Ctrl-C stops both)
#
# Driven by `make chain` / `make app-local` / `make dev`. Requires `surfpool` on
# PATH (or SURFPOOL_BIN), the Solana toolchain (for the .so), and pnpm.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

MODE="${1:-all}"
RPC_URL="http://127.0.0.1:8899"

ensure_built() {
  if [ ! -f "target/deploy/kassandra_program.so" ] || [ ! -f "target/deploy/kassandra_market_program.so" ]; then
    echo "==> building both programs (.so)…"
    just build
  fi
  echo "==> building both SDKs…"
  pnpm --filter ./sdk build >/dev/null
  pnpm --filter ./sdk-market build >/dev/null
}

run_chain() {
  ensure_built
  echo "==> booting + seeding the local chain (Ctrl-C to stop)…"
  exec pnpm --filter app exec tsx e2e/dev-seed.ts
}

wait_for_chain() {
  echo "==> waiting for surfpool + seed…"
  for _ in $(seq 1 90); do
    if [ -f "app/e2e/.wallet.json" ] && \
       curl -s -X POST "$RPC_URL" -H 'content-type: application/json' \
         -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' 2>/dev/null | grep -q '"result":"ok"'; then
      return 0
    fi
    sleep 1
  done
  echo "ERROR: chain did not come up in time" >&2
  return 1
}

run_app() {
  echo "==> starting the app dev server → $RPC_URL"
  exec env VITE_RPC_URL="$RPC_URL" VITE_E2E=1 pnpm --filter app dev
}

case "$MODE" in
  chain) run_chain ;;
  app)   run_app ;;
  all)
    ensure_built
    echo "==> booting + seeding the local chain (background)…"
    pnpm --filter app exec tsx e2e/dev-seed.ts &
    CHAIN_PID=$!
    trap 'echo; echo "==> stopping…"; kill "$CHAIN_PID" 2>/dev/null || true; pkill -P "$CHAIN_PID" 2>/dev/null || true' EXIT INT TERM
    wait_for_chain
    VITE_RPC_URL="$RPC_URL" VITE_E2E=1 pnpm --filter app dev
    ;;
  *)
    echo "usage: dev-up.sh [chain|app|all]" >&2
    exit 2
    ;;
esac

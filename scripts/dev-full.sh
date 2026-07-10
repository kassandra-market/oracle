#!/usr/bin/env bash
#
# dev-full.sh — bring up the FULL production-like local stack in one command:
# surfpool (seeded) + indexer (ephemeral Postgres) + mock-Anthropic runner + the
# app in real-wallet mode. Driven by `make dev`. Streams each service to
# `logs/<service>.log`; Ctrl-C tears everything down (the TS orchestrator traps
# SIGINT/SIGTERM and cleans up children + the temp Postgres).
#
# Requires: surfpool (or SURFPOOL_BIN), the Solana toolchain (to build the .so),
# and Postgres client binaries (`initdb`/`pg_ctl` — or PG_BIN).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "==> [1/4] preflight: surfpool + postgres binaries"
if ! command -v surfpool >/dev/null 2>&1 && [ -z "${SURFPOOL_BIN:-}" ]; then
  echo "ERROR: surfpool not found on PATH (set SURFPOOL_BIN)." >&2
  exit 1
fi
if ! command -v initdb >/dev/null 2>&1 && [ -z "${PG_BIN:-}" ] \
   && [ ! -x /opt/homebrew/opt/postgresql@16/bin/initdb ] \
   && [ ! -x /opt/homebrew/opt/postgresql@15/bin/initdb ] \
   && [ ! -x /usr/lib/postgresql/16/bin/initdb ] \
   && [ ! -x /usr/lib/postgresql/15/bin/initdb ]; then
  echo "ERROR: postgres binaries (initdb) not found. Set PG_BIN to their directory." >&2
  exit 1
fi

echo "==> [2/4] build both programs (.so), both SDKs, and the indexer binary"
# ALWAYS (re)build the programs. `cargo build-sbf` is incremental (~1s no-op when
# the .so is already current) and — crucially — rebuilds a STALE .so after a
# program change. The old "build only if the .so is MISSING" guard silently
# deployed a stale .so left from before a merge, so the current SDK's instruction
# layout no longer matched the deployed program → "invalid instruction data".
just build
# The app dev server imports BOTH @kassandra-market/oracles and @kassandra-market/markets, so
# both dist/ must exist before vite starts.
pnpm --filter @kassandra-market/oracles build >/dev/null
pnpm --filter @kassandra-market/markets build >/dev/null
cargo build --release --locked --manifest-path indexer/Cargo.toml

echo "==> [3/4] ensure logs/ exists + clear leftovers from a crashed run"
mkdir -p logs
# `make dev` OWNS the local stack, so a surfpool still listening on the fixed port
# is a leftover from a previously HARD-killed run (a clean Ctrl-C tears it down).
# Reusing it would make init_protocol fail with AlreadyInitialized, so clear it.
# (The ephemeral Postgres picks a fresh port per run, so it needs no cleanup.)
if command -v lsof >/dev/null 2>&1; then
  leftover="$(lsof -tiTCP:8899 -sTCP:LISTEN 2>/dev/null || true)"
  if [ -n "$leftover" ]; then
    echo "    clearing a leftover process on :8899 (previous run): $leftover"
    echo "$leftover" | xargs kill -9 2>/dev/null || true
    sleep 1
  fi
fi

echo "==> [4/4] launching the stack (Ctrl-C to stop everything)"
exec pnpm --filter app exec tsx e2e/dev-full.ts

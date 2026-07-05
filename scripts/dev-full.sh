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

echo "==> [2/4] build the program (.so), the SDK, and the indexer binary"
if [ ! -f "target/deploy/kassandra_program.so" ]; then
  just build
fi
pnpm --filter sdk build >/dev/null
cargo build --release --locked --manifest-path indexer/Cargo.toml

echo "==> [3/4] ensure logs/ exists"
mkdir -p logs

echo "==> [4/4] launching the stack (Ctrl-C to stop everything)"
exec pnpm --filter app exec tsx e2e/dev-full.ts

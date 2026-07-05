/**
 * `make dev` — the FULL production-like local stack, in ONE command.
 *
 * Boots and wires four services against a local, pre-seeded chain — everything
 * behaves like production except it runs locally with mock inputs:
 *
 *   surfpool  local Solana node: deploys the program + seeds oracles across phases
 *   indexer   the REAL `kassandra-indexer` binary crawling surfpool → read API,
 *             backed by an auto-managed EPHEMERAL Postgres (created + torn down
 *             here — you never install or manage a database)
 *   runner    a mock Anthropic endpoint so the AI runner produces claims OFFLINE
 *             (no API key, no network); a ready-to-use runner config is written
 *   app       the Vite app in PRODUCTION-LIKE mode: the REAL wallet-adapter (you
 *             connect your own Phantom/Solflare), reading the chain from surfpool
 *             and the ActivityFeed from the local indexer
 *
 * Each service streams to `logs/<service>.log`. Ctrl-C (SIGINT/SIGTERM) tears the
 * whole thing down — kills every child, stops the ephemeral Postgres, removes its
 * temp dir, and shuts down surfpool.
 *
 * NOTE on the DB: the indexer is Postgres-native (JSONB, threaded `Client` type)
 * and production runs on Postgres, so this uses an auto-managed ephemeral
 * Postgres rather than SQLite — same code path as prod, nothing for you to set
 * up. Requires `initdb` on PATH (or PG_BIN); you already have it for the indexer
 * e2e. A dedicated SQLite backend can be a follow-up if you want zero PG binaries.
 */
import { spawn, type ChildProcess } from 'node:child_process'
import { closeSync, mkdirSync, openSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'

import { Keypair } from '@solana/web3.js'
import { TOKEN_PROGRAM_ID, associatedTokenAccount } from '@kassandra/sdk'

import { toHex, tokenAccountBytes } from '../../sdk/test/surfpool/harness.ts'
import { MockAnthropic } from '../../sdk/test/surfpool/mock-anthropic.ts'
import {
  bootAndInit,
  createOracleReal,
  driveToFactProposal,
  driveToResolvedUncontested,
  keepWindowOpen,
  openProposals,
  submitOneFact,
} from './seed.ts'
import { startEphemeralPg, type EphemeralPg } from './indexer/pg.ts'

const SURFPOOL_PORT = 8899
const INDEXER_PORT = 3111
const PG_PORT = 5599
const APP_PORT = 5173

const APP_DIR = process.cwd() // `pnpm --filter app exec` runs here
const ROOT = resolve(APP_DIR, '..')
const LOGS = join(ROOT, 'logs')
const INDEXER_BIN = join(ROOT, 'indexer', 'target', 'release', 'kassandra-indexer')
const RUNNER_CONFIG = join(LOGS, 'runner.config.json')
const WALLET_FILE = join(APP_DIR, 'e2e', '.wallet.json')

/** Minimal base58 (Bitcoin alphabet) encoder — for the Phantom-import secret. */
function base58Encode(bytes: Uint8Array): string {
  const ALPHABET = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'
  const digits = [0]
  for (const byte of bytes) {
    let carry = byte
    for (let i = 0; i < digits.length; i++) {
      carry += digits[i] << 8
      digits[i] = carry % 58
      carry = (carry / 58) | 0
    }
    while (carry > 0) {
      digits.push(carry % 58)
      carry = (carry / 58) | 0
    }
  }
  let out = ''
  for (const b of bytes) {
    if (b === 0) out += ALPHABET[0]
    else break
  }
  for (let i = digits.length - 1; i >= 0; i--) out += ALPHABET[digits[i]]
  return out
}

const rpcUrl = `http://127.0.0.1:${SURFPOOL_PORT}`
const indexerUrl = `http://127.0.0.1:${INDEXER_PORT}`
const appUrl = `http://localhost:${APP_PORT}`

/** Everything we must tear down on exit, in reverse order of creation. */
const teardowns: Array<() => void | Promise<void>> = []
const logFds: number[] = []
let shuttingDown = false

function log(msg: string): void {
  // eslint-disable-next-line no-console
  console.log(msg)
}

/** Open a truncating log file and return its numeric fd (spawn stdio needs an
 *  fd, not a freshly-created WriteStream whose fd is still null). */
function openLog(name: string): number {
  const fd = openSync(join(LOGS, `${name}.log`), 'w')
  logFds.push(fd)
  return fd
}

/** Idempotent teardown: kill children, stop Postgres + surfpool, close logs. */
async function runTeardowns(reason: string): Promise<void> {
  if (shuttingDown) return
  shuttingDown = true
  log(`\n[dev] ${reason} — tearing down…`)
  for (const t of teardowns.reverse()) {
    try {
      await t()
    } catch (e) {
      log(`[dev] teardown error (ignored): ${String(e)}`)
    }
  }
  for (const fd of logFds) {
    try {
      closeSync(fd)
    } catch {
      /* already closed */
    }
  }
}

/** Wait until the indexer's /status reports it has crawled `minEvents`. */
async function waitForIndexer(minEvents: number, timeoutMs = 60_000): Promise<void> {
  const deadline = Date.now() + timeoutMs
  let last = ''
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`${indexerUrl}/status`)
      if (res.ok) {
        const s = (await res.json()) as { eventCount: number }
        last = JSON.stringify(s)
        if (s.eventCount >= minEvents) return
      }
    } catch {
      /* still starting */
    }
    await new Promise((r) => setTimeout(r, 500))
  }
  log(`[dev] ⚠ indexer did not reach ${minEvents} events in ${timeoutMs}ms (last: ${last}) — continuing`)
}

async function main(): Promise<void> {
  mkdirSync(LOGS, { recursive: true })

  // ── 1) surfpool + program + a spread of seeded oracles ─────────────────────
  log('[dev] booting surfpool + deploying the program…')
  const ctx = await bootAndInit(SURFPOOL_PORT)
  teardowns.push(() => ctx.harness.teardown())

  // A funded wallet you IMPORT into your browser extension (real-wallet mode).
  const wallet = await Keypair.generate()
  await ctx.harness.airdrop(wallet.publicKey.toString(), 50_000_000_000)
  const walletKass = (
    await associatedTokenAccount(wallet.publicKey.toString(), ctx.kassMint.publicKey.toString())
  ).address
  await ctx.harness.setAccount(walletKass.toString(), {
    lamports: 5_000_000,
    owner: TOKEN_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(
      tokenAccountBytes(ctx.kassMint.publicKey.toBytes(), wallet.publicKey.toBytes(), 10n ** 15n),
    ),
  })

  log('[dev] seeding oracles across phases…')
  const oracles: Record<string, Record<string, string>> = {}
  {
    const o = await createOracleReal(ctx, 1n, 3, 'Dev: pick an option')
    await openProposals(ctx, o)
    await keepWindowOpen(ctx, o)
    oracles.proposal = { nonce: '1', address: o.toString() }
  }
  {
    const o = await createOracleReal(ctx, 2n, 2, 'Dev: disputed — submit a fact')
    await driveToFactProposal(ctx, o)
    await keepWindowOpen(ctx, o)
    oracles.factProposal = { nonce: '2', address: o.toString() }
  }
  {
    const o = await createOracleReal(ctx, 3n, 2, 'Dev: disputed — vote on facts')
    await driveToFactProposal(ctx, o)
    const fact = await submitOneFact(ctx, o)
    await keepWindowOpen(ctx, o)
    oracles.factVoting = { nonce: '3', address: o.toString(), fact: fact.toString() }
  }
  {
    const o = await createOracleReal(ctx, 4n, 2, 'Dev: resolved uncontested')
    await driveToResolvedUncontested(ctx, o, 1)
    oracles.resolved = { nonce: '4', address: o.toString() }
  }

  // Keep the VITE_E2E fixture in sync too, so `make dev-e2e` / Playwright still work.
  writeFileSync(
    WALLET_FILE,
    JSON.stringify(
      {
        secretKey: Array.from(wallet.secretKey as Uint8Array),
        publicKey: wallet.publicKey.toString(),
        rpcUrl,
        kassMint: ctx.kassMint.publicKey.toString(),
        usdcMint: ctx.usdcMint.publicKey.toString(),
        oracles,
      },
      null,
      2,
    ),
  )

  // ── 2) ephemeral Postgres + the REAL indexer binary crawling surfpool ──────
  log('[dev] starting ephemeral Postgres + indexer…')
  const pg: EphemeralPg = await startEphemeralPg(PG_PORT)
  teardowns.push(() => pg.stop())
  const indexerLog = openLog('indexer')
  const indexer: ChildProcess = spawn(INDEXER_BIN, [], {
    env: {
      ...process.env,
      RPC_URL: rpcUrl,
      DATABASE_URL: pg.databaseUrl,
      PORT: String(INDEXER_PORT),
      COMMITMENT: 'confirmed',
      POLL_INTERVAL_MS: '1000',
      PROMOTE_INTERVAL_MS: '2000',
      RUST_LOG: 'info',
    },
    stdio: ['ignore', indexerLog, indexerLog],
  })
  teardowns.push(() => indexer.kill('SIGKILL'))
  await waitForIndexer(5)

  // ── 3) mock Anthropic — the runner's offline model backend ─────────────────
  log('[dev] starting mock Anthropic (runner backend)…')
  const mock = await MockAnthropic.start()
  teardowns.push(() => mock.stop())
  // A ready-to-run runner config (option 1, 2 options) pointing at the mock.
  writeFileSync(
    RUNNER_CONFIG,
    JSON.stringify(
      {
        interpretation: 'Dev: resolve the disputed oracle to the AI-selected option.',
        options_count: 2,
        option_labels: [
          { index: 0, label: 'Option 0' },
          { index: 1, label: 'Option 1' },
        ],
        facts: [],
      },
      null,
      2,
    ),
  )

  // ── 4) the app, PRODUCTION-LIKE: real wallet-adapter (NO VITE_E2E) ─────────
  log('[dev] starting the app (real wallet, local chain + indexer)…')
  const appLog = openLog('app')
  const app: ChildProcess = spawn('pnpm', ['--filter', 'app', 'dev', '--', '--port', String(APP_PORT)], {
    cwd: ROOT,
    env: {
      ...process.env,
      // Direct RPC to the local node + the local indexer for the ActivityFeed.
      // NO VITE_E2E → the REAL wallet-adapter is active (connect your own wallet).
      VITE_RPC_URL: rpcUrl,
      VITE_INDEXER_URL: indexerUrl,
      VITE_CLUSTER: 'localnet',
      VITE_E2E: '',
      VITE_MOCK: '',
    },
    stdio: ['ignore', appLog, appLog],
    // Own process group: `pnpm` spawns vite as a grandchild, so we must kill the
    // whole group (negative PID) — killing pnpm alone orphans vite on its port.
    detached: true,
  })
  teardowns.push(() => {
    try {
      if (app.pid) process.kill(-app.pid, 'SIGKILL')
    } catch {
      app.kill('SIGKILL')
    }
  })

  const secretB58 = base58Encode(wallet.secretKey as Uint8Array)
  log(`
[dev] ✅ production-like local stack is UP
      app       ${appUrl}          (logs/app.log)
      surfpool  ${rpcUrl}     (RPC)
      indexer   ${indexerUrl}     (logs/indexer.log)
      postgres  ${pg.databaseUrl}  (ephemeral — removed on exit)
      runner    mock Anthropic at ${mock.baseUrl}; config → logs/runner.config.json
      oracles   ${Object.keys(oracles).join(', ')}

      ── connect a wallet in the browser ────────────────────────────────
      The app uses the REAL wallet-adapter. To transact against this local
      chain, import this pre-funded dev keypair into Phantom/Solflare and add
      a custom network pointing at ${rpcUrl}:

        secret (base58):  ${secretB58}
        address:          ${wallet.publicKey.toString()}   (funded: SOL + KASS)

      ── run the AI runner offline ──────────────────────────────────────
        cargo run -p kassandra-runner -- --config logs/runner.config.json \\
          --anthropic-base-url ${mock.baseUrl}

      Ctrl-C to stop everything (services killed, temp Postgres removed).
`)

  const shutdown = async (sig: string) => {
    await runTeardowns(sig)
    process.exit(0)
  }
  process.on('SIGINT', () => void shutdown('SIGINT'))
  process.on('SIGTERM', () => void shutdown('SIGTERM'))
  await new Promise<never>(() => {}) // hold everything alive
}

main().catch(async (e) => {
  // eslint-disable-next-line no-console
  console.error('[dev] failed:', e)
  await runTeardowns('boot failed') // don't leak surfpool / Postgres on a boot error
  process.exit(1)
})

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
 *             connect your own Phantom/Solflare — by default this stack funds your
 *             local Solana CLI wallet `~/.config/solana/id.json`, so just connect
 *             it), reading the chain from surfpool and the ActivityFeed from the
 *             local indexer
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
import { closeSync, existsSync, mkdirSync, openSync, readFileSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
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
  type SeedCtx,
} from './seed.ts'
import { seedMarkets, type ActiveMarketSeed } from './seed-market.ts'
import { swapOnPool } from './seed-market-active.ts'
import { startEphemeralPg, type EphemeralPg } from './indexer/pg.ts'

const SURFPOOL_PORT = 8899
const INDEXER_PORT = 3111
const APP_PORT = 5173

const APP_DIR = process.cwd() // `pnpm --filter app exec` runs here
const ROOT = resolve(APP_DIR, '..')
const LOGS = join(ROOT, 'logs')
// The indexer is a WORKSPACE member, so `cargo build --manifest-path
// indexer/Cargo.toml` writes the binary to the workspace-root target/, NOT
// indexer/target/ (which doesn't exist — the pre-merge per-crate path).
const INDEXER_BIN = join(ROOT, 'target', 'release', 'kassandra-indexer')
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

/**
 * The dev wallet: by default the local Solana CLI keypair
 * (`~/.config/solana/id.json`, override path via `DEV_WALLET_KEYPAIR`), so you
 * transact from the wallet you already hold — no import step. Falls back to a
 * freshly generated (and printed) keypair when no local keypair file exists.
 * Returns the loaded keypair plus whether it came from disk.
 */
async function loadDevWallet(): Promise<{ wallet: Keypair; fromFile: boolean }> {
  const path = process.env.DEV_WALLET_KEYPAIR || join(homedir(), '.config', 'solana', 'id.json')
  if (existsSync(path)) {
    try {
      const secret = Uint8Array.from(JSON.parse(readFileSync(path, 'utf8')) as number[])
      return { wallet: await Keypair.fromSecretKey(secret), fromFile: true }
    } catch (e) {
      log(`[dev] ⚠ could not read ${path} (${(e as Error).message}); generating an ephemeral wallet`)
    }
  }
  return { wallet: await Keypair.generate(), fromFile: false }
}

const rpcUrl = `http://127.0.0.1:${SURFPOOL_PORT}`
// surfpool's websocket (accountSubscribe/programSubscribe) — bound explicitly to
// RPC port + 1 so the indexer's price subscriber has a deterministic ws url.
const wsUrl = `ws://127.0.0.1:${SURFPOOL_PORT + 1}`
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

/**
 * Give the active market a non-trivial price history: wait until the indexer's
 * price subscriber is live (its baseline candle exists), then drive a few swaps
 * that move the pool up and down. Each swap's pool update is recorded as a candle
 * point, so the market's chart shows genuine movement in `make dev`.
 */
async function seedActivePriceHistory(ctx: SeedCtx, seed: ActiveMarketSeed): Promise<void> {
  const candlesUrl = `${indexerUrl}/api/markets/${seed.market}/candles?interval=60&limit=5`
  const deadline = Date.now() + 30_000
  // 1) Wait until the subscriber has captured its baseline point (⇒ it's subscribed).
  for (;;) {
    try {
      const res = await fetch(candlesUrl)
      if (res.ok && ((await res.json()) as unknown[]).length >= 1) break
    } catch {
      /* indexer/subscriber still coming up */
    }
    if (Date.now() > deadline) throw new Error('price subscriber did not produce a baseline candle')
    await new Promise((r) => setTimeout(r, 500))
  }
  // 2) Move the price both ways so the candle has a real range (down → up → down).
  await swapOnPool(ctx, seed, 'down', 2_000_000_000n)
  await new Promise((r) => setTimeout(r, 1_200))
  await swapOnPool(ctx, seed, 'up', 3_000_000_000n)
  await new Promise((r) => setTimeout(r, 1_200))
  await swapOnPool(ctx, seed, 'down', 1_000_000_000n)
}

async function main(): Promise<void> {
  mkdirSync(LOGS, { recursive: true })

  // ── 1) surfpool + program + a spread of seeded oracles ─────────────────────
  log('[dev] booting surfpool + deploying the program…')
  const ctx = await bootAndInit(SURFPOOL_PORT, { wsPort: SURFPOOL_PORT + 1 })
  teardowns.push(() => ctx.harness.teardown())

  // The funded dev wallet: by default your local Solana CLI keypair
  // (~/.config/solana/id.json), so you transact from the wallet you already hold —
  // just connect it in the browser (no import). Falls back to a generated keypair.
  const { wallet, fromFile: walletFromFile } = await loadDevWallet()
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

  // ── 1b) deploy the market program (+ MetaDAO fixtures) + seed demo markets ──
  // Same surfpool node, same KASS mint — so the single indexer picks up both and
  // the app's /markets section is populated. Best-effort: a market-seed failure
  // must not sink the whole dev stack (the oracle side is already useful).
  log('[dev] deploying the market program + seeding demo markets…')
  let markets: Record<string, unknown> | null = null
  let activeSeed: ActiveMarketSeed | null = null
  try {
    const res = await seedMarkets(ctx, oracles)
    markets = res.seeded
    activeSeed = res.active
  } catch (e) {
    log(`[dev] ⚠ market seeding failed (oracle stack still up): ${(e as Error).message}`)
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
        markets,
      },
      null,
      2,
    ),
  )

  // ── 2) ephemeral Postgres + the REAL indexer binary crawling surfpool ──────
  log('[dev] starting ephemeral Postgres + indexer…')
  const pg: EphemeralPg = await startEphemeralPg() // fresh OS-assigned port each run
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
      // The single indexer also runs the market account pipeline + the per-pool
      // websocket price subscriber. SOLANA_WS_URL points at surfpool's ws (RPC
      // port + 1 — surfpool ≥ 1.1.2 implements accountSubscribe/programSubscribe),
      // so the price subscriber records candle points as the pool trades. A short
      // getProgramAccounts reconcile also keeps market_accounts fresh (belt + braces
      // if the ws tail hiccups). MARKET_PROGRAM_ID defaults to the known id.
      SOLANA_WS_URL: wsUrl,
      INDEXER_RECONCILE_MS: '1000',
      RUST_LOG: 'info',
    },
    stdio: ['ignore', indexerLog, indexerLog],
  })
  teardowns.push(() => indexer.kill('SIGKILL'))
  await waitForIndexer(5)

  // Give the active market a real price history: once the indexer's price
  // subscriber is live (a first candle exists), drive a few swaps that move the
  // pool so the market's candlestick chart shows genuine movement, not a flat point.
  if (activeSeed) {
    try {
      await seedActivePriceHistory(ctx, activeSeed)
      log('[dev] seeded price history on the active market (candlestick chart populated)')
    } catch (e) {
      log(`[dev] ⚠ price-history seeding skipped: ${(e as Error).message}`)
    }
  }

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

  // ── 4) the app ─────────────────────────────────────────────────────────────
  // Default: PRODUCTION-LIKE — the real wallet-adapter (connect your own wallet).
  // `WALLET=funded make dev`: auto-connect the pre-funded dev keypair instead
  // (works today; use it to exercise writes while the real-wallet/kit path is
  // sorted). The funded keypair is passed to the E2E provider via env.
  const fundedWallet = process.env.WALLET === 'funded'
  log(`[dev] starting the app (${fundedWallet ? 'funded auto-connect' : 'real'} wallet)…`)
  const appLog = openLog('app')
  const app: ChildProcess = spawn('pnpm', ['--filter', 'app', 'dev', '--', '--port', String(APP_PORT)], {
    cwd: ROOT,
    env: {
      ...process.env,
      VITE_RPC_URL: rpcUrl,
      VITE_INDEXER_URL: indexerUrl,
      VITE_CLUSTER: 'localnet',
      VITE_E2E: fundedWallet ? '1' : '',
      VITE_E2E_WALLET_SECRET: fundedWallet
        ? JSON.stringify(Array.from(wallet.secretKey as Uint8Array))
        : '',
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

  // Only reveal a secret when we GENERATED the wallet — the local CLI keypair is
  // the user's own; they connect it, we never print its secret.
  const walletBlock = walletFromFile
    ? `      ── connect your wallet in the browser ─────────────────────────────
      The app uses the REAL wallet-adapter. This stack funded your LOCAL
      Solana CLI wallet (~/.config/solana/id.json) — connect it in the
      browser and point a custom network at ${rpcUrl}:

        address:          ${wallet.publicKey.toString()}   (funded: SOL + KASS)`
    : `      ── connect a wallet in the browser ────────────────────────────────
      The app uses the REAL wallet-adapter. No local Solana CLI keypair was
      found, so import this generated, pre-funded dev keypair into
      Phantom/Solflare and point a custom network at ${rpcUrl}:

        secret (base58):  ${base58Encode(wallet.secretKey as Uint8Array)}
        address:          ${wallet.publicKey.toString()}   (funded: SOL + KASS)`
  log(`
[dev] ✅ production-like local stack is UP
      app       ${appUrl}          (logs/app.log)
      surfpool  ${rpcUrl}     (RPC)
      indexer   ${indexerUrl}     (logs/indexer.log)
      postgres  ${pg.databaseUrl}  (ephemeral — removed on exit)
      runner    mock Anthropic at ${mock.baseUrl}; config → logs/runner.config.json
      oracles   ${Object.keys(oracles).join(', ')}

${walletBlock}

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

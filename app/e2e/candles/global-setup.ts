/**
 * Playwright globalSetup for the CANDLE e2e — the full-stack proof of the
 * subscription-driven price chart.
 *
 * Boots surfpool (with an explicit ws port), seeds an ACTIVE market with a live
 * cYES/cNO pool, then runs the real `kassandra-indexer` binary against surfpool +
 * an ephemeral Postgres with `SOLANA_WS_URL` pointed at surfpool's websocket. The
 * indexer opens an `accountSubscribe` on the pool; once it's subscribed (a first
 * candle exists), we fire REAL swaps that move the price, and wait until the
 * `/candles` API reflects that movement — proving each swap was captured live.
 *
 * The spec then loads the app at `/markets/:pubkey` and asserts the candlestick
 * chart renders from that indexed data. This exercises the whole pipeline:
 * chain swap → ws accountSubscribe → Postgres market_price → /candles API → chart.
 */
import { spawn, type ChildProcess } from 'node:child_process'
import { openSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

import { Keypair } from '@solana/web3.js'

import { bootAndInit, createOracleReal } from '../seed.ts'
import { seedActiveMarket, swapOnPool, type ActiveMarketSeed } from '../seed-market-active.ts'
import { startEphemeralPg, type EphemeralPg } from '../indexer/pg.ts'

const SURFPOOL_PORT = 8964
const WS_PORT = 8965
const INDEXER_PORT = 3113
const WALLET_FILE = join(process.cwd(), 'e2e', 'candles', '.fixture.json')
// The indexer is a workspace member, so `cargo build -p kassandra-indexer` (and
// `--manifest-path indexer/Cargo.toml`) emit to the WORKSPACE target dir, not
// `indexer/target/`. Point at the workspace binary so we always run the fresh build.
const INDEXER_BIN = join(process.cwd(), '..', 'target', 'release', 'kassandra-indexer')

interface Candle {
  time: number
  open: number
  high: number
  low: number
  close: number
}

async function fetchCandles(indexerUrl: string, market: string): Promise<Candle[]> {
  try {
    const res = await fetch(`${indexerUrl}/api/markets/${market}/candles?interval=60&limit=50`)
    if (!res.ok) return []
    return (await res.json()) as Candle[]
  } catch {
    // Indexer not listening yet (ECONNREFUSED) — treat as "no candles, keep polling".
    return []
  }
}

/** Poll until the candles API satisfies `pred` (or throw after `timeoutMs`). */
async function waitForCandles(
  indexerUrl: string,
  market: string,
  pred: (c: Candle[]) => boolean,
  what: string,
  timeoutMs = 60_000,
): Promise<Candle[]> {
  const deadline = Date.now() + timeoutMs
  let last: Candle[] = []
  while (Date.now() < deadline) {
    last = await fetchCandles(indexerUrl, market)
    if (pred(last)) return last
    await new Promise((r) => setTimeout(r, 500))
  }
  throw new Error(`candles never satisfied "${what}" in ${timeoutMs}ms (last: ${JSON.stringify(last)})`)
}

/** Price range across all candles — the amount the pool price moved. */
function priceRange(candles: Candle[]): number {
  if (candles.length === 0) return 0
  const highs = candles.map((c) => c.high)
  const lows = candles.map((c) => c.low)
  return Math.max(...highs) - Math.min(...lows)
}

async function globalSetup(): Promise<() => Promise<void>> {
  const ctx = await bootAndInit(SURFPOOL_PORT, { wsPort: WS_PORT })
  const rpcUrl = `http://127.0.0.1:${SURFPOOL_PORT}`
  const indexerUrl = `http://127.0.0.1:${INDEXER_PORT}`

  // A funded browser wallet (parity with the other specs; the chart is read-only).
  const wallet = await Keypair.generate()
  await ctx.harness.airdrop(wallet.publicKey.toString(), 50_000_000_000)

  // Seed an oracle + an ACTIVE market with a live pool (starts ~50/50 → P(YES)≈0.5).
  // Any failure here must tear surfpool down, else a leaked node holds the port and
  // the next run hits AlreadyInitialized on its own protocol init.
  let oracle: Awaited<ReturnType<typeof createOracleReal>>
  let seed: ActiveMarketSeed
  try {
    oracle = await createOracleReal(ctx, 1n, 2, 'Candle e2e: tradeable market')
    seed = await seedActiveMarket(ctx, oracle.toString())
  } catch (e) {
    await ctx.harness.teardown()
    throw e
  }

  // Ephemeral Postgres + the REAL indexer. SOLANA_WS_URL points at surfpool's
  // websocket: the market account pipeline uses the live `programSubscribe` tail
  // (surfpool ≥ 1.1.2 implements it) to keep market_accounts fresh, and the price
  // subscriber `accountSubscribe`s each Active pool. No INDEXER_RECONCILE_MS — we
  // deliberately run in subscribe mode so the ws tail (not polling) is exercised.
  const pg: EphemeralPg = await startEphemeralPg()
  const indexerLog = openSync(join(process.cwd(), 'e2e', 'candles', '.indexer.log'), 'w')
  const indexer: ChildProcess = spawn(INDEXER_BIN, [], {
    env: {
      ...process.env,
      RPC_URL: rpcUrl,
      SOLANA_WS_URL: `ws://127.0.0.1:${WS_PORT}`,
      DATABASE_URL: pg.databaseUrl,
      PORT: String(INDEXER_PORT),
      COMMITMENT: 'confirmed',
      RUST_LOG: 'info',
    },
    stdio: ['ignore', indexerLog, indexerLog],
  })

  try {
    // 1) Wait until the subscriber has a baseline point (proves it subscribed).
    await waitForCandles(indexerUrl, seed.market, (c) => c.length >= 1, 'baseline candle', 60_000)

    // 2) Fire REAL swaps that move the price both ways. These happen AFTER the
    //    subscription is live, so each pool account update is captured as a point.
    await swapOnPool(ctx, seed, 'down', 2_000_000_000n) // sell cYES → P(YES) down
    await new Promise((r) => setTimeout(r, 1200))
    await swapOnPool(ctx, seed, 'up', 3_000_000_000n) // buy cYES → P(YES) up
    await new Promise((r) => setTimeout(r, 1200))
    await swapOnPool(ctx, seed, 'down', 1_000_000_000n) // sell cYES → P(YES) down again

    // 3) Wait until the captured series shows real movement (high != low across
    //    candles) — the proof the subscription recorded the live swaps intra-minute.
    const candles = await waitForCandles(
      indexerUrl,
      seed.market,
      (c) => priceRange(c) > 0.001,
      'price movement from live swaps',
      60_000,
    )

    writeFileSync(
      WALLET_FILE,
      JSON.stringify(
        {
          secretKey: Array.from(wallet.secretKey as Uint8Array),
          publicKey: wallet.publicKey.toString(),
          rpcUrl,
          indexerUrl,
          market: seed.market,
          oracle: oracle.toString(),
          candleCount: candles.length,
          priceRange: priceRange(candles),
        },
        null,
        2,
      ),
    )

    // eslint-disable-next-line no-console
    console.log(
      `[e2e:candles] market ${seed.market} active; ${candles.length} candle(s), range ${priceRange(
        candles,
      ).toFixed(3)} — indexed via ws subscription`,
    )
  } catch (e) {
    indexer.kill('SIGKILL')
    pg.stop()
    await ctx.harness.teardown()
    throw e
  }

  return async () => {
    indexer.kill('SIGKILL')
    pg.stop()
    await ctx.harness.teardown()
  }
}

export default globalSetup

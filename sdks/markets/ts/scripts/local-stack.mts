/**
 * Local dev stack — boots a real local Solana node (surfpool, offline) seeded with
 * every program the protocol needs, spawns the indexer against it, seeds some demo
 * data, and blocks until Ctrl-C (tearing everything down on exit).
 *
 * What it stands up:
 *   • surfpool (offline) with the market program + the MetaDAO conditional-vault +
 *     amm programs deployed from the in-repo `.so` fixtures, plus the program's
 *     BPF-Upgradeable-Loader `ProgramData` (so `init_config` accepts the dev wallet).
 *   • the `kassandra-market-indexer` binary, pointed at surfpool, on :10000.
 *   • demo data: a KASS mint + funded dev wallet, the `Config` singleton, and a few
 *     markets (a Funding one, an activatable one, and a categorical group).
 *
 * Run: `make local-node` (or `pnpm --filter @kassandra-market/markets local:stack`).
 * The app (`make app`) proxies `/api/*` to the indexer, so `make dev` gives a fully
 * clickable local app. The dev wallet is written to `sdk/.local/wallet.json`.
 *
 * Reuses `MarketSurfpoolHarness` (same seeding path as the e2e), so this never
 * drifts from what the tests exercise.
 */
import { spawn, type ChildProcess } from 'node:child_process'
import { existsSync, mkdirSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import {
  MARKET_PROGRAM_ID,
  Phase,
  createMarket,
  initConfig,
  pda,
} from '../src/index.js'
import { Keypair, MarketSurfpoolHarness } from '../test/surfpool/harness.ts'

const here = dirname(fileURLToPath(import.meta.url))
const REPO_ROOT = resolve(here, '../..')
const INDEXER_BIN = resolve(REPO_ROOT, 'target/debug/kassandra-market-indexer')
const WALLET_FILE = resolve(here, '..', '.local', 'wallet.json')

const RPC_PORT = Number(process.env.SURFPOOL_PORT ?? 8899)
const INDEXER_PORT = Number(process.env.INDEXER_PORT ?? 10_000)
const RECONCILE_MS = 500 // surfpool has no programSubscribe → GPA-reconcile tail

const MIN_LIQ = 1_000_000_000n // 1 KASS floor (9 dp)
const BELOW_FLOOR = 600_000_000n // stays Funding
const WALLET_KASS = 10n ** 15n

function startIndexer(rpcUrl: string): ChildProcess {
  if (!existsSync(INDEXER_BIN)) {
    throw new Error(`Missing ${INDEXER_BIN}. Run \`make build-indexer\` first.`)
  }
  const child = spawn(INDEXER_BIN, [], {
    cwd: REPO_ROOT,
    env: {
      ...process.env,
      SOLANA_RPC_URL: rpcUrl,
      SOLANA_WS_URL: `ws://127.0.0.1:${RPC_PORT + 1}`,
      PORT: String(INDEXER_PORT),
      MARKET_PROGRAM_ID: MARKET_PROGRAM_ID.toString(),
      INDEXER_RECONCILE_MS: String(RECONCILE_MS),
      RUST_LOG: process.env.RUST_LOG ?? 'info',
    },
    stdio: ['ignore', 'inherit', 'inherit'],
  })
  return child
}

async function waitForHealth(): Promise<void> {
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    try {
      if ((await fetch(`http://127.0.0.1:${INDEXER_PORT}/health`)).ok) return
    } catch {
      /* not up yet */
    }
    await new Promise((r) => setTimeout(r, 250))
  }
  throw new Error('indexer /health never came up')
}

async function main(): Promise<void> {
  console.log('[local-stack] booting surfpool (offline) + deploying programs…')
  const harness = await MarketSurfpoolHarness.start({
    port: RPC_PORT,
    offline: true,
    readyTimeoutMs: 60_000,
  })

  console.log('[local-stack] starting indexer…')
  const indexer = startIndexer(harness.rpcUrl)

  let tornDown = false
  const teardown = async (exitCode = 0) => {
    if (tornDown) return
    tornDown = true
    console.log('\n[local-stack] shutting down…')
    if (indexer.exitCode === null) indexer.kill('SIGKILL')
    await harness.teardown()
    process.exit(exitCode)
  }
  process.on('SIGINT', () => void teardown(0))
  process.on('SIGTERM', () => void teardown(0))
  indexer.on('exit', (code) => {
    if (!tornDown) {
      console.error(`[local-stack] indexer exited (${code}); tearing down`)
      void teardown(1)
    }
  })

  // From here on, a throw MUST tear down surfpool + the indexer (they're already
  // spawned) — otherwise a failed health check or seed tx orphans both processes
  // (surfpool is `detached:false`, so `process.exit` alone would not reap them, and
  // the next `make local-node` would hit a taken port).
  try {
    await seedAndBlock()
  } catch (e) {
    console.error('[local-stack] failed after boot:', e)
    await teardown(1)
  }

  async function seedAndBlock(): Promise<void> {
  await waitForHealth()

  // ── seed demo data (a KASS mint, funded dev wallet, Config, demo markets) ──
  console.log('[local-stack] seeding config + demo markets…')
  const wallet = await Keypair.generate()
  await harness.airdrop(wallet.publicKey.toString(), 50_000_000_000)
  await harness.setUpgradeAuthority(wallet.publicKey) // wallet pays init_config

  const kassMint = await harness.createMint(9, wallet.publicKey)
  await harness.fundTokenAccount(kassMint, wallet.publicKey, WALLET_KASS)
  const feeDestination = await harness.createTokenAccount(kassMint, wallet.publicKey, 0n)
  await harness.sendIx(wallet, [
    await initConfig({
      payer: wallet.publicKey,
      kassMint,
      authority: wallet.publicKey,
      minLiquidity: MIN_LIQ,
      feeBps: 100,
      feeDestination,
    }),
  ])

  const walletKassAta = (await pda.associatedTokenAccount(wallet.publicKey, kassMint)).address
  const seedMarket = async (oracle: Awaited<ReturnType<typeof harness.seedOracle>>, outcomeIndex: number, amount: bigint) => {
    await harness.sendIx(wallet, [
      await createMarket({ creator: wallet.publicKey, oracle, kassMint, creatorKassAta: walletKassAta, seedAmount: amount, outcomeIndex }),
    ])
    return (await pda.market(oracle, outcomeIndex)).address.toString()
  }

  const fundingOracle = await harness.seedOracle({ optionsCount: 2, phase: Phase.Proposal })
  const fundingMarket = await seedMarket(fundingOracle, 0, BELOW_FLOOR)
  const activatableOracle = await harness.seedOracle({ optionsCount: 2, phase: Phase.Proposal })
  const activatableMarket = await seedMarket(activatableOracle, 0, MIN_LIQ)
  const categoricalOracle = await harness.seedOracle({ optionsCount: 3, phase: Phase.Proposal })
  const categoricalMarkets: string[] = []
  for (let i = 0; i < 3; i++) categoricalMarkets.push(await seedMarket(categoricalOracle, i, BELOW_FLOOR))

  mkdirSync(dirname(WALLET_FILE), { recursive: true })
  writeFileSync(
    WALLET_FILE,
    JSON.stringify(
      {
        secretKey: Array.from(wallet.secretKey as Uint8Array),
        publicKey: wallet.publicKey.toString(),
        rpcUrl: harness.rpcUrl,
        kassMint: kassMint.toString(),
        fundingMarket,
        activatableMarket,
        categoricalOracle: categoricalOracle.toString(),
        categoricalMarkets,
      },
      null,
      2,
    ),
  )

  console.log(
    [
      '',
      '  ┌─ local stack ready ────────────────────────────────────────────',
      `  │ surfpool RPC   http://127.0.0.1:${RPC_PORT}  (ws :${RPC_PORT + 1})`,
      `  │ indexer API    http://127.0.0.1:${INDEXER_PORT}  (/health, /api/*)`,
      `  │ dev wallet     ${wallet.publicKey.toString()}`,
      `  │ wallet file    ${WALLET_FILE}`,
      `  │ demo markets   funding=${fundingMarket.slice(0, 8)}… activatable=${activatableMarket.slice(0, 8)}… categorical×3`,
      '  │',
      '  │ → run the app against it:  make app   (or: INDEXER_URL=http://127.0.0.1:' + INDEXER_PORT + ' pnpm --filter @kassandra-market/app dev)',
      '  │ → Ctrl-C to tear everything down',
      '  └────────────────────────────────────────────────────────────────',
      '',
    ].join('\n'),
  )

  // Block forever; teardown runs on SIGINT/SIGTERM.
  await new Promise(() => {})
  }
}

// Last resort: an error BEFORE surfpool/indexer are spawned (nothing to reap yet).
main().catch((e) => {
  console.error('[local-stack] failed to boot:', e)
  process.exit(1)
})

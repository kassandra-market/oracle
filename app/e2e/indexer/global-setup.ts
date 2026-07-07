/**
 * Playwright globalSetup for the INDEXER e2e.
 *
 * Boots surfpool, seeds an oracle with a rich set of REAL transactions
 * (create_oracle → propose×2 → finalize_proposals → submit_fact), then runs the
 * actual `kassandra-indexer` binary — the same one deployed on Render — against
 * surfpool's RPC + an ephemeral Postgres. Once the indexer has crawled the
 * seeded activity, the spec loads the app pointed at the indexer and asserts the
 * on-chain ActivityFeed renders those instructions.
 *
 * This exercises the whole indexing pipeline end to end: chain → crawler →
 * Postgres → read API → app.
 */
import { spawn, type ChildProcess } from 'node:child_process'
import { writeFileSync } from 'node:fs'
import { join } from 'node:path'

import { Keypair } from '@solana/web3.js'

import {
  bootAndInit,
  createOracleReal,
  driveToFactProposal,
  submitOneFact,
} from '../seed.ts'
import { startEphemeralPg, type EphemeralPg } from './pg.ts'

const SURFPOOL_PORT = 8960
const INDEXER_PORT = 3111
const PG_PORT = 5599
const WALLET_FILE = join(process.cwd(), 'e2e', 'indexer', '.wallet.json')
// The indexer is a workspace member, so `cargo build -p kassandra-indexer` emits
// to the WORKSPACE target dir, not `indexer/target/`. Point at the workspace binary.
const INDEXER_BIN = join(process.cwd(), '..', 'target', 'release', 'kassandra-indexer')

async function waitForIndexer(url: string, minEvents: number, timeoutMs = 60_000): Promise<void> {
  const deadline = Date.now() + timeoutMs
  let last = ''
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`${url}/status`)
      if (res.ok) {
        const s = (await res.json()) as { eventCount: number }
        last = JSON.stringify(s)
        if (s.eventCount >= minEvents) return
      }
    } catch {
      /* indexer still starting */
    }
    await new Promise((r) => setTimeout(r, 500))
  }
  throw new Error(`indexer did not reach ${minEvents} events in ${timeoutMs}ms (last: ${last})`)
}

async function globalSetup(): Promise<() => Promise<void>> {
  const ctx = await bootAndInit(SURFPOOL_PORT)
  const rpcUrl = `http://127.0.0.1:${SURFPOOL_PORT}`
  const indexerUrl = `http://127.0.0.1:${INDEXER_PORT}`

  // A funded browser wallet (the app runs in e2e mode and reads its secret) — the
  // ActivityFeed is read-only, but the wallet keeps parity with the other specs.
  const wallet = await Keypair.generate()
  await ctx.harness.airdrop(wallet.publicKey.toString(), 50_000_000_000)

  // Seed ONE oracle with a diverse, real transaction history.
  const oracle = await createOracleReal(ctx, 1n, 2, 'Indexer e2e: rich activity')
  await driveToFactProposal(ctx, oracle) // create + propose×2 + finalize_proposals
  await submitOneFact(ctx, oracle) // + submit_fact
  // Instructions touching this oracle: create_oracle, propose, finalize_proposals,
  // submit_fact (propose fires twice). Expect ≥5 events for it.
  const expectedTypes = ['create_oracle', 'propose', 'finalize_proposals', 'submit_fact']

  // Ephemeral Postgres + the REAL indexer binary crawling surfpool.
  const pg: EphemeralPg = await startEphemeralPg(PG_PORT)
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
    stdio: ['ignore', 'inherit', 'inherit'],
  })

  // Wait until the indexer has crawled the seeded activity (≥5 program events).
  await waitForIndexer(indexerUrl, 5)

  writeFileSync(
    WALLET_FILE,
    JSON.stringify(
      {
        secretKey: Array.from(wallet.secretKey as Uint8Array),
        publicKey: wallet.publicKey.toString(),
        rpcUrl,
        indexerUrl,
        oracle: { address: oracle.toString(), expectedTypes },
      },
      null,
      2,
    ),
  )

  // eslint-disable-next-line no-console
  console.log(
    `[e2e:indexer] surfpool ${rpcUrl}; indexer ${indexerUrl}; oracle ${oracle.toString()} seeded + indexed`,
  )

  return async () => {
    indexer.kill('SIGKILL')
    pg.stop()
    await ctx.harness.teardown()
  }
}

export default globalSetup

/**
 * Boot a local surfpool, deploy the program, and seed a spread of oracles for
 * INTERACTIVE development — then hold the validator alive (Ctrl-C to stop). This
 * is `make chain`: a persistent seeded local chain you can browse in the app dev
 * server (`make app-local`), unlike the Playwright globalSetup which tears down.
 *
 * Reuses the e2e seed harness so there is one source of truth for how oracles are
 * driven into each phase. Writes `e2e/.wallet.json` (the funded wallet + oracle
 * map) so the app in VITE_E2E mode drives the funded keypair.
 */
import { writeFileSync } from 'node:fs'
import { join } from 'node:path'

import { Keypair } from '@solana/web3.js'
import { TOKEN_PROGRAM_ID, associatedTokenAccount } from '@kassandra-market/oracles'

import { toHex, tokenAccountBytes } from '../../sdks/oracles/ts/test/surfpool/harness.ts'
import {
  bootAndInit,
  createOracleReal,
  driveToFactProposal,
  driveToResolvedUncontested,
  keepWindowOpen,
  openProposals,
  submitOneFact,
} from './seed.ts'

const PORT = 8899
const WALLET_FILE = join(process.cwd(), 'e2e', '.wallet.json')

async function main(): Promise<void> {
  console.log('[dev] booting surfpool + deploying the program…')
  const ctx = await bootAndInit(PORT)
  const rpcUrl = `http://127.0.0.1:${PORT}`

  // Funded browser wallet (SOL + KASS), same shape as the e2e globalSetup.
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

  const oracles: Record<string, Record<string, string>> = {}

  console.log('[dev] seeding oracles across phases…')
  // Proposal (window open).
  {
    const o = await createOracleReal(ctx, 1n, 3, 'Dev: pick an option')
    await openProposals(ctx, o)
    await keepWindowOpen(ctx, o)
    oracles.proposal = { nonce: '1', address: o.toString() }
  }
  // FactProposal (disputed, window open).
  {
    const o = await createOracleReal(ctx, 2n, 2, 'Dev: disputed — submit a fact')
    await driveToFactProposal(ctx, o)
    await keepWindowOpen(ctx, o)
    oracles.factProposal = { nonce: '2', address: o.toString() }
  }
  // FactVoting (one fact posted).
  {
    const o = await createOracleReal(ctx, 3n, 2, 'Dev: disputed — vote on facts')
    await driveToFactProposal(ctx, o)
    const fact = await submitOneFact(ctx, o)
    await keepWindowOpen(ctx, o)
    oracles.factVoting = { nonce: '3', address: o.toString(), fact: fact.toString() }
  }
  // Resolved (uncontested) — drives its own proposers → Resolved(option 1).
  {
    const o = await createOracleReal(ctx, 4n, 2, 'Dev: resolved uncontested')
    await driveToResolvedUncontested(ctx, o, 1)
    oracles.resolved = { nonce: '4', address: o.toString() }
  }

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

  console.log(`
[dev] ✅ local chain ready
      surfpool:  ${rpcUrl}
      wallet:    ${wallet.publicKey.toString()} (funded SOL + KASS)
      oracles:   ${Object.keys(oracles).join(', ')}
      fixture:   ${WALLET_FILE}

      Now run the app against it:  make app-local
      (or in another shell:        VITE_RPC_URL=${rpcUrl} VITE_E2E=1 pnpm --filter app dev)

      Ctrl-C to stop the chain.
`)

  const shutdown = async () => {
    console.log('\n[dev] tearing down surfpool…')
    await ctx.harness.teardown()
    process.exit(0)
  }
  process.on('SIGINT', () => void shutdown())
  process.on('SIGTERM', () => void shutdown())
  await new Promise<never>(() => {}) // hold the validator alive
}

main().catch((e) => {
  console.error('[dev] failed:', e)
  process.exit(1)
})

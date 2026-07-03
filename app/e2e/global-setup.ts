/**
 * Playwright globalSetup — spins up surfpool and seeds ONE oracle per browser
 * action so the specs can drive every app UI write against an oracle already in
 * the right phase, with a REAL funded wallet keypair.
 *
 * Seeds (see `seed.ts` for the drivers):
 *   proposal      — Proposal, window open        → wallet proposes
 *   factProposal  — dispute → FactProposal open   → wallet submits a fact
 *   factVoting    — → FactVoting (1 fact)         → wallet votes
 *   aiClaim       — → AiClaim, WALLET is proposer → wallet submits an AI claim
 *   finalizeReady — Proposal, window ELAPSED      → wallet cranks finalize
 *   resolved      — uncontested → Resolved,
 *                   WALLET is a winning proposer  → wallet claims
 *
 * Writes the funded keypair + the seeded oracle map to `e2e/.wallet.json`.
 */
import { writeFileSync } from 'node:fs'
import { join } from 'node:path'

import { Keypair } from '@solana/web3.js'
import { TOKEN_PROGRAM_ID, associatedTokenAccount, finalizeProposals } from '@kassandra/sdk'

import { toHex, tokenAccountBytes } from '../../sdk/test/surfpool/harness.ts'
import {
  advanceToAiClaim,
  advanceToFactVoting,
  advancePastPhaseEnd,
  approveVote,
  bootAndInit,
  createOracleReal,
  driveToFactProposal,
  keepWindowOpen,
  openProposals,
  proposeAs,
  sendIx,
  submitOneFact,
} from './seed.ts'

const PORT = 8899
const WALLET_FILE = join(process.cwd(), 'e2e', '.wallet.json')

async function globalSetup(): Promise<() => Promise<void>> {
  const ctx = await bootAndInit(PORT)
  const rpcUrl = `http://127.0.0.1:${PORT}`

  // The funded browser wallet — SOL + KASS at its canonical ATA (create-fee burn
  // + claim destination). It also plays a proposer in the AiClaim + Resolved seeds.
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

  // 1) Proposal (window open) — wallet proposes.
  {
    const o = await createOracleReal(ctx, 1n, 3, 'E2E propose: pick an option')
    await openProposals(ctx, o)
    await keepWindowOpen(ctx, o)
    oracles.proposal = { nonce: '1', address: o.toString() }
  }

  // 2) FactProposal (open) — wallet submits a fact.
  {
    const o = await createOracleReal(ctx, 2n, 2, 'E2E submitFact: disputed')
    await driveToFactProposal(ctx, o)
    await keepWindowOpen(ctx, o)
    oracles.factProposal = { nonce: '2', address: o.toString() }
  }

  // 3) FactVoting (1 fact) — wallet votes.
  {
    const o = await createOracleReal(ctx, 3n, 2, 'E2E voteFact: disputed')
    await driveToFactProposal(ctx, o)
    const fact = await submitOneFact(ctx, o)
    await advanceToFactVoting(ctx, o)
    await keepWindowOpen(ctx, o)
    oracles.factVoting = { nonce: '3', address: o.toString(), fact: fact.toString() }
  }

  // 4) AiClaim — the WALLET is a locked-in proposer; it submits its AI claim.
  {
    const o = await createOracleReal(ctx, 4n, 2, 'E2E submitAiClaim: wallet is a proposer')
    const proposers = await driveToFactProposal(ctx, o, wallet)
    const fact = await submitOneFact(ctx, o)
    await advanceToFactVoting(ctx, o)
    await approveVote(ctx, o, fact)
    await advanceToAiClaim(ctx, o, 4n, fact)
    await keepWindowOpen(ctx, o)
    oracles.aiClaim = { nonce: '4', address: o.toString(), proposer: proposers[0].toString() }
  }

  // 5) Proposal, window ELAPSED — wallet cranks finalize_proposals.
  {
    const o = await createOracleReal(ctx, 5n, 2, 'E2E finalize crank')
    await openProposals(ctx, o)
    await proposeAs(ctx, o, await Keypair.generate(), 0, 1_000n)
    await proposeAs(ctx, o, await Keypair.generate(), 1, 1_000n)
    await advancePastPhaseEnd(ctx, o)
    oracles.finalizeReady = { nonce: '5', address: o.toString() }
  }

  // 6) Resolved (uncontested) — the WALLET is a winning proposer; it claims.
  {
    const o = await createOracleReal(ctx, 6n, 2, 'E2E claim: wallet wins')
    await openProposals(ctx, o)
    const p: string[] = []
    const walletProposer = (await proposeAs(ctx, o, wallet, 1, 5_000n)).toString()
    p.push(walletProposer)
    p.push((await proposeAs(ctx, o, await Keypair.generate(), 1, 5_000n)).toString())
    p.push((await proposeAs(ctx, o, await Keypair.generate(), 1, 5_000n)).toString())
    await advancePastPhaseEnd(ctx, o)
    await sendIx(ctx, await finalizeProposals({ oracle: o.toString(), proposers: p }))
    oracles.resolved = { nonce: '6', address: o.toString(), proposer: walletProposer }
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

  // eslint-disable-next-line no-console
  console.log(`[e2e] surfpool ${rpcUrl}; wallet ${wallet.publicKey.toString()}; seeded phases: ${Object.keys(oracles).join(', ')}`)

  return async () => {
    await ctx.harness.teardown()
  }
}

export default globalSetup

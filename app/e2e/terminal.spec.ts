import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { expect, test } from '@playwright/test'

import { getAccountData, poll } from './onchain'

/**
 * Browser E2E for the terminal settlement controls on a fully-Resolved oracle
 * where the wallet is the winning proposer + agreed-fact submitter + approve
 * voter + AI claimant. Drives, in the program's required order:
 *   claim_proposer · claim_fact_vote · claim_fact (fact closes only after its
 *   voters have claimed) · close_ai_claim — each closes its account on-chain.
 */
const wallet = JSON.parse(readFileSync(join(process.cwd(), 'e2e', '.wallet.json'), 'utf8')) as {
  secretKey: number[]
  oracles: Record<string, { address: string; proposer: string; fact: string; factVote: string; aiClaim: string }>
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript((secret) => {
    ;(window as unknown as { __E2E_WALLET_SECRET__?: number[] }).__E2E_WALLET_SECRET__ = secret
  }, wallet.secretKey)
})

test('terminal: claim proposer, fact-vote, fact payouts and close the AI claim', async ({ page }) => {
  const o = wallet.oracles.resolvedFull
  await page.goto(`/oracles/${o.address}`)
  await expect(page.getByRole('button', { name: /^Connected:/ })).toBeVisible()

  // The proposer / fact / AI-claim cards (and their terminal settle controls)
  // live under the Records tab.
  await page.getByRole('tab', { name: /Records/ }).click()

  // claim_proposer — the winning proposer's payout; the Proposer account closes.
  await page.getByRole('button', { name: 'Claim proposer payout' }).click()
  await poll(() => getAccountData(o.proposer), (d) => d === null)

  // claim_fact_vote — MUST precede the fact claim (the fact closes only once its
  // voters have claimed); the FactVote account closes.
  await page.getByRole('button', { name: 'Claim your fact vote' }).click()
  await poll(() => getAccountData(o.factVote), (d) => d === null)

  // claim_fact — the agreed fact's submitter payout; the Fact account closes.
  await page.getByRole('button', { name: 'Claim fact payout' }).click()
  await poll(() => getAccountData(o.fact), (d) => d === null)

  // close_ai_claim — reclaim the AiClaim rent; the AiClaim account closes.
  await page.getByRole('button', { name: 'Close AI claim' }).click()
  await poll(() => getAccountData(o.aiClaim), (d) => d === null)
})

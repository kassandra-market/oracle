import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { type Page, expect, test } from '@playwright/test'

import { oracleAt, poll, setClockTo } from './onchain'

/**
 * Browser E2E for the permissionless FinalizeControl cranks that carry an oracle
 * between phases once its window has elapsed:
 *   advance_phase (FactProposal → FactVoting) · finalize_facts (→ AiClaim) ·
 *   finalize_ai_claims (→ Challenge / dead-end).
 * (finalize_proposals is covered in writes.spec.) Each crank oracle is seeded
 * un-patched, so its window elapses; the spec elapses the clock, clicks the
 * crank, and asserts the on-chain phase moved.
 */
const wallet = JSON.parse(readFileSync(join(process.cwd(), 'e2e', '.wallet.json'), 'utf8')) as {
  secretKey: number[]
  oracles: Record<string, { address: string }>
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript((secret) => {
    ;(window as unknown as { __E2E_WALLET_SECRET__?: number[] }).__E2E_WALLET_SECRET__ = secret
  }, wallet.secretKey)
})

async function crank(page: Page, address: string, button: RegExp): Promise<void> {
  await page.goto(`/oracles/${address}`)
  await expect(page.getByRole('button', { name: /^Connected:/ })).toBeVisible()
  const o = await oracleAt(address)
  await setClockTo(Number(o.phaseEndsAt) + 120) // elapse the window
  // The advance/finalize cranks live under the Manage tab.
  await page.getByRole('tab', { name: /Manage/ }).click()
  await page.getByRole('button', { name: button }).click()
}

test('advancePhase: crank FactProposal → FactVoting', async ({ page }) => {
  const o = wallet.oracles.factProposalCrank
  await crank(page, o.address, /Advance to fact voting/i)
  await poll(() => oracleAt(o.address), (x) => x.phaseRaw !== 2)
})

test('finalizeFacts: crank FactVoting → AiClaim', async ({ page }) => {
  const o = wallet.oracles.factVotingCrank
  await crank(page, o.address, /Finalize facts/i)
  await poll(() => oracleAt(o.address), (x) => x.phaseRaw !== 3)
})

test('finalizeAiClaims: crank AiClaim → Challenge/dead-end', async ({ page }) => {
  const o = wallet.oracles.aiClaimCrank
  await crank(page, o.address, /Finalize AI claims/i)
  await poll(() => oracleAt(o.address), (x) => x.phaseRaw !== 4)
})

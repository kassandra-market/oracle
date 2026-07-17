import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { expect, test } from '@playwright/test'

import { oracleAt, poll, setClockTo } from './onchain'

/**
 * Browser E2E: crank a Challenge-phase oracle (no challenge opened, the wallet
 * survived the AI-claim round) into its terminal state via the FinalizeControl
 * "Finalize oracle" button (finalize_oracle → Resolved / InvalidDeadend).
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

test('finalizeOracle: crank Challenge → terminal', async ({ page }) => {
  const o = wallet.oracles.challengeElapsed
  await page.goto(`/oracles/${o.address}`)
  await expect(page.getByRole('button', { name: /^Connected:/ })).toBeVisible()
  const oc = await oracleAt(o.address)
  await setClockTo(Number(oc.phaseEndsAt) + 120) // elapse the Challenge window
  // The advance/finalize crank lives under the Manage tab.
  await page.getByRole('tab', { name: /Manage/ }).click()
  await page.getByRole('button', { name: /Finalize oracle/i }).click()
  // On-chain: the oracle reached a terminal phase (Resolved=7 / InvalidDeadend=8).
  await poll(() => oracleAt(o.address), (x) => x.phaseRaw === 7 || x.phaseRaw === 8)
})

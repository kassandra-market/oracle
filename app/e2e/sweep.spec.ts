import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { expect, test } from '@playwright/test'

import { getAccountData, poll } from './onchain'

/**
 * Browser E2E: sweep a terminal, grace-elapsed oracle. globalSetup resolved it,
 * fabricated DAO governance (so the treasury exists), and back-dated its
 * phase_ends_at past the 30-day sweep grace — so the permissionless
 * SweepControl button is available and sweep_oracle transfers residual dust to
 * the treasury and closes the stake-vault + Oracle.
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

test('sweepOracle: permissionless dust sweep + terminal close', async ({ page }) => {
  const o = wallet.oracles.sweepReady
  await page.goto(`/oracles/${o.address}`)
  await expect(page.getByRole('button', { name: /^Connected:/ })).toBeVisible()
  // The permissionless sweep lives under the Manage tab.
  await page.getByRole('tab', { name: /Manage/ }).click()
  await page.getByRole('button', { name: 'Sweep oracle' }).click()
  // On-chain: sweep closes the Oracle account (rent → creator).
  await poll(() => getAccountData(o.address), (d) => d === null)
})

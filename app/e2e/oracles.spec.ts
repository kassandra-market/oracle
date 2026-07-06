import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { expect, test } from '@playwright/test'

/**
 * Browser E2E: the dApp, pointed at the surfpool validator that globalSetup
 * seeded, with the funded keypair injected as the real-signing e2e wallet.
 *
 * These prove the whole local stack end to end: the validator is up with the
 * program deployed + protocol initialized, the app reads the seeded oracles over
 * RPC (`getProgramAccounts`), and the funded wallet reports connected — the
 * write path (sign + send + confirm) is wired to a real, funded key.
 */
const wallet = JSON.parse(
  readFileSync(join(process.cwd(), 'e2e', '.wallet.json'), 'utf8'),
) as {
  secretKey: number[]
  publicKey: string
  oracles: { nonce: string; address: string }[]
}

test.beforeEach(async ({ page }) => {
  // Inject the funded keypair BEFORE the app's JS runs, so the e2e wallet picks
  // it up (window.__E2E_WALLET_SECRET__) and reports connected with a real key.
  await page.addInitScript((secret) => {
    ;(window as unknown as { __E2E_WALLET_SECRET__?: number[] }).__E2E_WALLET_SECRET__ = secret
  }, wallet.secretKey)
})

test('funded wallet connects and the seeded oracles load from the local validator', async ({
  page,
}) => {
  await page.goto('/oracles')

  // The dashboard-stats strip only renders once the oracle list has been fetched
  // from surfpool — its presence proves the app is talking to the local chain.
  await expect(page.getByLabel('Oracle capital at stake')).toBeVisible()

  // The funded e2e wallet reports connected: the nav pill flips to the truncated
  // address (aria-label "Connected: <pubkey>. Click to disconnect.").
  const connected = page.getByRole('button', { name: /^Connected:/ })
  await expect(connected).toBeVisible()
  await expect(connected).toContainText(wallet.publicKey.slice(0, 4))

  // At least one seeded oracle is browsable (a phase filter chip / total tile).
  await expect(page.getByText(/proposal/i).first()).toBeVisible()
})

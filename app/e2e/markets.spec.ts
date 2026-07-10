import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { expect, test } from '@playwright/test'

/**
 * Browser E2E for the MARKETS product (the merged prediction market). The
 * browser-e2e stack runs surfpool + the app but NOT the indexer, and market data
 * (Meteora/MetaDAO) needs a mainnet fork — so this is a UI-shell + routing smoke:
 * it proves the Markets pages render, the nav routes to them, the list degrades
 * gracefully with no indexer (no white-screen), and the create-market form mounts
 * for the connected wallet. The data-heavy market flows are covered by the gated
 * forked-mainnet suites (`challengeTrade.e2e`, `sdks/oracles/ts/test/surfpool/*market*`).
 */
const wallet = JSON.parse(
  readFileSync(join(process.cwd(), 'e2e', '.wallet.json'), 'utf8'),
) as { secretKey: number[]; publicKey: string }

test.beforeEach(async ({ page }) => {
  // Inject the funded keypair BEFORE the app's JS runs so the e2e wallet reports
  // connected (window.__E2E_WALLET_SECRET__) — the create-market form needs it.
  await page.addInitScript((secret) => {
    ;(window as unknown as { __E2E_WALLET_SECRET__?: number[] }).__E2E_WALLET_SECRET__ = secret
  }, wallet.secretKey)
})

test('the Markets tab routes from Oracles and the list renders (no indexer → graceful)', async ({
  page,
}) => {
  await page.goto('/oracles')

  // The primary nav exposes the Markets product route; clicking it navigates.
  // (.first() guards the desktop vs mobile-menu link duplication in the nav.)
  await page
    .getByRole('navigation', { name: 'Primary' })
    .getByRole('link', { name: 'Markets' })
    .first()
    .click()
  await expect(page).toHaveURL(/\/markets$/)

  // The page shell renders (header + create entry point) regardless of data.
  await expect(page.getByText('Open markets')).toBeVisible()
  await expect(page.getByRole('link', { name: /create a market/i }).first()).toBeVisible()

  // With no indexer in this stack, the list must DEGRADE GRACEFULLY — one of the
  // list states shows and the app does not white-screen (heading still present).
  await expect(
    page
      .getByText(/could not load markets from the indexer/i)
      .or(page.getByText(/no markets/i))
      .or(page.getByLabel('Search markets'))
      .first(),
  ).toBeVisible()

  // The wallet still reports connected (the nav pill flips to the address).
  const connected = page.getByRole('button', { name: /^Connected:/ })
  await expect(connected).toBeVisible()
  await expect(connected).toContainText(wallet.publicKey.slice(0, 4))
})

test('the create-market page renders its form for a connected wallet', async ({ page }) => {
  await page.goto('/markets/new')

  // The create-market page shell + form mount (the form needs the connected e2e
  // wallet; ConnectGate would otherwise show a connect prompt).
  await expect(page.getByText('New market')).toBeVisible()
  await expect(page.getByRole('button', { name: /^Connected:/ })).toBeVisible()
  // The oracle-binding input is the form's first field — its presence proves the
  // CreateMarketForm mounted rather than a connect gate.
  await expect(page.getByLabel(/oracle/i).first()).toBeVisible()
})

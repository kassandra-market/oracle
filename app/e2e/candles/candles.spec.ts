import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { expect, test } from '@playwright/test'

/**
 * The market Price-history candlestick chart, rendered from the REAL indexer.
 *
 * globalSetup stood up an ACTIVE market, ran the actual kassandra-indexer binary
 * with a websocket `accountSubscribe` on the pool, then fired real swaps that moved
 * the price. Here we assert (1) the `/candles` API returns that captured movement,
 * and (2) the app's MarketDetail page renders the candlestick chart from it — the
 * full chain-swap → ws → Postgres → API → chart pipeline, in a browser.
 */
const fixture = JSON.parse(
  readFileSync(join(process.cwd(), 'e2e', 'candles', '.fixture.json'), 'utf8'),
) as {
  secretKey: number[]
  indexerUrl: string
  market: string
  oracle: string
  candleCount: number
  priceRange: number
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript((secret) => {
    ;(window as unknown as { __E2E_WALLET_SECRET__?: number[] }).__E2E_WALLET_SECRET__ = secret
  }, fixture.secretKey)
})

test('candles API returns the swap-driven price series', async ({ request }) => {
  const res = await request.get(
    `${fixture.indexerUrl}/api/markets/${fixture.market}/candles?interval=60&limit=50`,
  )
  expect(res.ok()).toBeTruthy()
  const candles = (await res.json()) as { open: number; high: number; low: number; close: number }[]
  expect(candles.length).toBeGreaterThanOrEqual(1)

  // The subscription captured multiple intra-bucket points from the live swaps, so
  // the price genuinely moved (high != low somewhere) — not a single flat sample.
  const range = Math.max(...candles.map((c) => c.high)) - Math.min(...candles.map((c) => c.low))
  expect(range).toBeGreaterThan(0.001)

  // Every candle is a valid OHLC in probability space (0..1, low ≤ high).
  for (const c of candles) {
    for (const v of [c.open, c.high, c.low, c.close]) {
      expect(v).toBeGreaterThanOrEqual(0)
      expect(v).toBeLessThanOrEqual(1)
    }
    expect(c.low).toBeLessThanOrEqual(c.high)
  }
})

test('MarketDetail renders the candlestick chart from indexed data', async ({ page }) => {
  await page.goto(`/markets/${fixture.market}`)
  await expect(page.getByRole('button', { name: /^Connected:/ })).toBeVisible()

  // The Trade panel is present (Active market → the chart + buy/sell form share it).
  await expect(page.getByRole('heading', { name: 'Trade' })).toBeVisible()

  // The chart mounted and loaded data: the container is not in its empty state, and
  // lightweight-charts painted a canvas inside it.
  const chart = page.getByTestId('price-chart')
  await expect(chart).toBeVisible()
  await expect(page.getByTestId('price-chart-empty')).toHaveCount(0)
  await expect(chart.locator('canvas').first()).toBeVisible()

  // The trading actions live in the SAME panel as the chart (the Trade card).
  const tradeCard = page.locator('div', { has: chart }).filter({ hasText: 'Trade' }).last()
  await expect(tradeCard.getByRole('group', { name: 'Buy or sell' })).toBeVisible()

  // The interval toggle is wired (switching re-queries the series without error).
  await page.getByRole('button', { name: '1m' }).click()
  await expect(page.getByTestId('price-chart-empty')).toHaveCount(0)
})

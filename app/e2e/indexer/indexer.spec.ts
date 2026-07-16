import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { expect, test } from '@playwright/test'

/**
 * The app's on-chain ActivityFeed, rendered from the REAL indexer.
 *
 * globalSetup seeded an oracle with create_oracle → propose×2 →
 * finalize_proposals → submit_fact, and the actual kassandra-indexer binary
 * crawled surfpool into Postgres. Here we open that oracle in the app (pointed at
 * the indexer via VITE_INDEXER_URL) and assert the feed shows those instructions.
 */
const fixture = JSON.parse(
  readFileSync(join(process.cwd(), 'e2e', 'indexer', '.wallet.json'), 'utf8'),
) as {
  secretKey: number[]
  indexerUrl: string
  oracle: { address: string; expectedTypes: string[] }
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript((secret) => {
    ;(window as unknown as { __E2E_WALLET_SECRET__?: number[] }).__E2E_WALLET_SECRET__ = secret
  }, fixture.secretKey)
})

test('RPC gateway: POST /rpc forwards JSON-RPC to the backend RPC', async ({ request }) => {
  // The backend crawls surfpool; its /rpc gateway forwards to that same RPC — so
  // the app performs chain work through the backend, never holding an RPC URL.
  const res = await request.post(`${fixture.indexerUrl}/rpc`, {
    headers: { 'content-type': 'application/json' },
    data: { jsonrpc: '2.0', id: 1, method: 'getHealth' },
  })
  expect(res.ok()).toBeTruthy()
  const body = (await res.json()) as { result?: string }
  expect(body.result).toBe('ok')
})

test('indexer API: /accounts/:oracle/events returns the seeded instructions', async ({
  request,
}) => {
  const res = await request.get(`${fixture.indexerUrl}/accounts/${fixture.oracle.address}/events`)
  expect(res.ok()).toBeTruthy()
  const body = (await res.json()) as { events: { ixType: string; signature: string }[] }
  const types = new Set(body.events.map((e) => e.ixType))
  for (const t of fixture.oracle.expectedTypes) {
    expect(types, `indexer should have a '${t}' event for the oracle`).toContain(t)
  }
})

test('ActivityFeed renders the indexed events on the oracle page', async ({ page }) => {
  await page.goto(`/oracles/${fixture.oracle.address}`)
  await expect(page.getByRole('button', { name: /^Connected:/ })).toBeVisible()

  // The Activity feed lives under the Activity tab (only present when the indexer
  // backend is configured, i.e. VITE_INDEXER_URL is set).
  await page.getByRole('tab', { name: /Activity/ }).click()

  const activity = page.getByRole('heading', { name: 'On-chain activity' })
  await expect(activity).toBeVisible()

  // Scope the row assertions to the active Activity tab panel.
  const feed = page.locator('#panel-activity')

  // Each seeded instruction type shows as a human-labelled row (e.g. "Create
  // oracle", "Propose", "Finalize proposals", "Submit fact").
  const humanLabels: Record<string, RegExp> = {
    create_oracle: /Create oracle/,
    propose: /Propose/,
    finalize_proposals: /Finalize proposals/,
    submit_fact: /Submit fact/,
  }
  for (const t of fixture.oracle.expectedTypes) {
    await expect(feed.getByText(humanLabels[t]).first()).toBeVisible()
  }

  // At least the 5 seeded events are listed.
  const rows = feed.locator('ul > li')
  expect(await rows.count()).toBeGreaterThanOrEqual(5)
})

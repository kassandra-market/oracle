import { readFileSync, mkdirSync } from 'node:fs'
import { join } from 'node:path'

import { expect, test, type Page } from '@playwright/test'

/**
 * Visual capture pass — walks every route + key interaction state (validation
 * errors, a persistent on-chain error alert, disabled/enabled buttons, the
 * challenge compose form, the admin controls) and saves full-page PNGs to
 * `e2e/screenshots/` for a human/agent to eyeball the Auros styling and behaviour.
 *
 * Not an assertion suite — it captures; the review happens on the PNGs. Runs in
 * the default (non-forked) config against the seeded validator.
 */
const wallet = JSON.parse(readFileSync(join(process.cwd(), 'e2e', '.wallet.json'), 'utf8')) as {
  secretKey: number[]
  oracles: Record<string, { address: string }>
}
const OUT = join(process.cwd(), 'e2e', 'screenshots')

test.beforeAll(() => {
  mkdirSync(OUT, { recursive: true })
})

test.beforeEach(async ({ page }) => {
  await page.addInitScript((secret) => {
    ;(window as unknown as { __E2E_WALLET_SECRET__?: number[] }).__E2E_WALLET_SECRET__ = secret
  }, wallet.secretKey)
})

async function connected(page: Page): Promise<void> {
  await expect(page.getByRole('button', { name: /^Connected:/ })).toBeVisible()
}

async function shot(page: Page, name: string): Promise<void> {
  // Bound the network-idle wait: the app polls its RPC on an interval, so the
  // network is never truly idle and an UNBOUNDED wait stalls each screenshot
  // until the whole test hits its timeout. Give it a short window to settle,
  // then capture regardless (the 600ms buffer + prior navigation cover render).
  await page.waitForLoadState('networkidle', { timeout: 2_500 }).catch(() => {})
  await page.waitForTimeout(600)
  await page.screenshot({ path: join(OUT, `${name}.png`), fullPage: true })
}

test('capture: routes + interaction states', async ({ page }) => {
  // ── Landing ────────────────────────────────────────────────────────────────
  await page.goto('/')
  await connected(page)
  await shot(page, '01-landing')

  // ── Oracles list ─────────────────────────────────────────────────────────
  await page.goto('/oracles')
  await connected(page)
  await shot(page, '02-oracles-list')

  // ── Create: empty ───────────────────────────────────────────────────────
  await page.goto('/oracles/new')
  await connected(page)
  await shot(page, '03-create-empty')

  // ── Create: validation errors (submit an empty form) ─────────────────────
  await page.getByRole('button', { name: /Create oracle/i }).click()
  await page.waitForTimeout(300)
  await shot(page, '04-create-validation-errors')

  // ── Create: filled ───────────────────────────────────────────────────────
  await page.getByPlaceholder(/SpaceX Starship/i).fill('Did the e2e demo pass on the first try?')
  await page.waitForTimeout(200)
  await shot(page, '05-create-filled')

  // ── Oracle detail, per phase ─────────────────────────────────────────────
  const details: [string, string][] = [
    ['proposal', '06-detail-proposal'],
    ['factVoting', '07-detail-fact-voting'],
    ['aiClaim', '08-detail-ai-claim'],
    ['finalizeReady', '09-detail-finalize-crank'],
    ['resolvedFull', '10-detail-resolved-terminal'],
    ['challengeElapsed', '11-detail-challenge-compose'],
    ['sweepReady', '12-detail-sweep'],
    ['deadend', '13-detail-deadend'],
  ]
  for (const [key, name] of details) {
    const o = wallet.oracles[key]
    if (!o) continue
    await page.goto(`/oracles/${o.address}`)
    await connected(page)
    await shot(page, name)
  }

  // ── Admin page ────────────────────────────────────────────────────────────
  await page.goto('/admin')
  await connected(page)
  await shot(page, '14-admin')

  // ── Admin: a persistent on-chain ERROR alert (the wallet is not the DAO
  //    authority, so set_config reverts Unauthorized; the /admin page does not
  //    refetch, so the red WriteStatusRegion error stays put). ──
  await page.getByRole('button', { name: /Set config/i }).click()
  await expect(page.getByText(/Program|failed|Unauthorized|custom program error/i).first()).toBeVisible({
    timeout: 30_000,
  })
  await shot(page, '15-admin-error-alert')

  // ── Style guide ───────────────────────────────────────────────────────────
  await page.goto('/styleguide')
  await shot(page, '16-styleguide')
})

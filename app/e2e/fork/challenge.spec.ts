import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { expect, test, type Page } from '@playwright/test'
import { Address } from '@solana/web3.js'
import { ammV04, associatedTokenAccount, futarchy, pda } from '@kassandra-market/oracles'

import {
  ammTwap,
  backdateMarketTwapEnd,
  backdateOraclePhaseEnd,
  fabricateTokenAccount,
  getAccountData,
  marketAt,
  oraclePhaseByte,
  poll,
  tokenBalance,
} from './onchain'

/**
 * FORKED-MAINNET browser E2E for the FULL challenge-market cluster, driven end to
 * end through the app UI against a surfpool fork (executable MetaDAO programs) in
 * clock mode. One serial flow over a Challenge-phase oracle:
 *
 *   compose → open challenge  (ChallengeComposeForm: 7 client-side steps)
 *   swap                      (ChallengeTradeControls · SwapForm — a real fail-pool buy)
 *   crank TWAP                (ChallengeTradeControls · CrankForm — pass + fail pools)
 *   settle challenge          (ChallengeTradeControls · SettleButton — one-click derive)
 *   close market              (CloseControl — reap the settled market)
 *
 * Each write is asserted by its persistent on-chain effect (the UI success line is
 * wiped by the post-write refetch).
 */
const wallet = JSON.parse(readFileSync(join(process.cwd(), 'e2e', 'fork', '.wallet.json'), 'utf8')) as {
  secretKey: number[]
  publicKey: string
  kassMint: string
  usdcMint: string
  kassDao: string
  oracle: string
  nonce: string
  proposer: string
}

const QUESTION_ID = new Uint8Array(32).fill(0x07)
const SWAP_IN = '90000000' // USDC buy on the fail pool — clears the disqualify margin
const FAIL_USDC_TOPUP = 200_000_000n

/** Reload the oracle page and wait for the funded wallet to be connected. */
async function reload(page: Page, url: string): Promise<void> {
  await page.goto(url)
  await expect(page.getByRole('button', { name: /^Connected:/ })).toBeVisible()
}

/** Reload, wait for a per-pool crank button to be enabled (rate-limit lifted), click it. */
async function crank(page: Page, url: string, label: 'Pass' | 'Fail'): Promise<void> {
  await reload(page, url)
  const btn = page.getByRole('button', { name: new RegExp(`Crank ${label} TWAP`, 'i') })
  await expect(btn).toBeEnabled({ timeout: 30_000 })
  await btn.click()
  // Let the refetch settle (the success line is transient); the on-chain TWAP is asserted by the caller.
  await page.waitForTimeout(1500)
}

test.describe.configure({ mode: 'serial', timeout: 720_000 })

test('challenge cluster: compose → open → swap → crank → settle → close', async ({ page }) => {
  await page.addInitScript((secret) => {
    ;(window as unknown as { __E2E_WALLET_SECRET__?: number[] }).__E2E_WALLET_SECRET__ = secret
  }, wallet.secretKey)

  // ── Deterministic account set the browser composes / trades ────────────────
  const aiClaim = (await pda.aiClaim(wallet.oracle, wallet.proposer)).address.toString()
  const market = (await pda.market(aiClaim)).address.toString()
  const question = (await futarchy.pda.question(QUESTION_ID, wallet.oracle, 2)).address.toString()
  const usdcVault = (await futarchy.pda.conditionalVault(question, wallet.usdcMint)).address.toString()
  const kassVault = (await futarchy.pda.conditionalVault(question, wallet.kassMint)).address.toString()
  const failUsdcMint = (await futarchy.pda.conditionalTokenMint(usdcVault, 1)).address.toString()
  const failKassMint = (await futarchy.pda.conditionalTokenMint(kassVault, 1)).address.toString()
  const failAmm = (await ammV04.pda.amm(failKassMint, failUsdcMint)).address.toString()
  const challengerFailUsdc = (
    await associatedTokenAccount(wallet.publicKey, failUsdcMint)
  ).address.toString()

  const url = `/oracles/${wallet.oracle}?proposer=${wallet.proposer}&kassDao=${wallet.kassDao}`

  // ── 1) COMPOSE + OPEN the challenge (7 wallet-signed steps) ────────────────
  // The sequence runs step-by-step in the browser (each a forked tx); on the last
  // step's success the auto-refetch swaps the compose form for the trade controls,
  // so assert the PERSISTENT on-chain Market rather than the transient success line.
  await reload(page, url)
  await page.getByRole('button', { name: /Compose & open challenge/i }).click()
  const opened = await poll(
    () => marketAt(market),
    (x) => x.challenger.toString() === wallet.publicKey,
    300_000,
  )
  expect(opened.oracle.toString()).toBe(wallet.oracle)
  expect(opened.proposer.toString()).toBe(wallet.proposer)

  // ── 2) SWAP — a real BUY on the fail pool. The compose split spent the
  //       challenger's fail-USDC seeding liquidity, so top it up first.
  await fabricateTokenAccount(
    challengerFailUsdc,
    new Address(failUsdcMint).toBytes(),
    new Address(wallet.publicKey).toBytes(),
    FAIL_USDC_TOPUP,
  )
  await reload(page, url)
  await page.getByLabel('Pool').selectOption('fail')
  await page.getByLabel('Side').selectOption('buy')
  await page.getByPlaceholder(/e\.g\. 1000000/).fill(SWAP_IN)
  await page.getByLabel(/Slippage %/i).fill('50')
  await page.getByRole('button', { name: /^Swap$/i }).click()
  // On-chain: the buy consumed fail-USDC from the challenger.
  await poll(() => tokenBalance(challengerFailUsdc), (b) => b < FAIL_USDC_TOPUP, 60_000)

  // ── 3) CRANK the pass + fail pools' TWAP (permissionless; slot-rate-limited) ─
  await crank(page, url, 'Pass')
  await crank(page, url, 'Fail')
  await crank(page, url, 'Fail')
  // On-chain: the swap-driven fail TWAP is a real, non-zero observation.
  await poll(() => ammTwap(failAmm), (t) => t > 0n, 60_000)

  // ── 4) SETTLE — one-click derive-from-Market. Rewind twap_end so the window
  //       has elapsed (both the browser gate + the on-chain check open).
  await backdateMarketTwapEnd(market)
  await reload(page, url)
  await page.getByRole('button', { name: /Settle challenge/i }).click()
  await poll(() => marketAt(market), (x) => x.settled === true, 60_000)

  // ── 5) FINALIZE the oracle → Resolved (the challenge is settled + no challenges
  //       remain open). Backdate the Challenge window so finalize_oracle's
  //       window-elapsed gate passes; this is also what surfaces "Close market".
  await backdateOraclePhaseEnd(wallet.oracle)
  await reload(page, url)
  await page.getByRole('button', { name: /Finalize oracle/i }).click()
  await poll(() => oraclePhaseByte(wallet.oracle), (p) => p === 7 || p === 8, 60_000)

  // ── 6) CLOSE the settled market (permissionless reap; account disappears) ──
  await reload(page, url)
  await page.getByRole('button', { name: /Close market/i }).click()
  await poll(() => getAccountData(market), (d) => d === null, 60_000)
})

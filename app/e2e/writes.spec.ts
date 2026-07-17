import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { type Page, expect, test } from '@playwright/test'

import { getAccountData, oracleAt, factAt, poll, proposerAt, setClockTo } from './onchain'

/**
 * Browser E2E covering every participant write the protocol allows, each driven
 * through the real app UI with the funded e2e wallet against a surfpool oracle
 * seeded (by globalSetup) into the phase where the action is legal:
 *   propose · submitFact · voteFact · submitAiClaim · finalize (crank) · claim.
 *
 * Each write signs + sends + confirms on the local validator (a real funded
 * keypair), and is asserted by its PERSISTENT ON-CHAIN effect (the transient UI
 * success line is wiped by the post-write refetch, so we verify the chain).
 */
const wallet = JSON.parse(
  readFileSync(join(process.cwd(), 'e2e', '.wallet.json'), 'utf8'),
) as {
  secretKey: number[]
  publicKey: string
  oracles: Record<
    string,
    { nonce: string; address: string; fact?: string; proposer?: string; runner?: string }
  >
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript((secret) => {
    ;(window as unknown as { __E2E_WALLET_SECRET__?: number[] }).__E2E_WALLET_SECRET__ = secret
  }, wallet.secretKey)
})

/** Open an oracle's detail page and wait for the funded wallet to connect. */
async function openOracle(page: Page, address: string): Promise<void> {
  await page.goto(`/oracles/${address}`)
  await expect(page.getByRole('button', { name: /^Connected:/ })).toBeVisible()
}

test('propose: submit a categorical option + KASS bond', async ({ page }) => {
  const o = wallet.oracles.proposal
  await openOracle(page, o.address)
  // Participation forms live under the Manage tab.
  await page.getByRole('tab', { name: /Manage/ }).click()
  await page.getByPlaceholder('e.g. 5').fill('5')
  await page.getByRole('button', { name: 'Propose', exact: true }).click()
  // On-chain: a Proposer now exists on the oracle.
  await poll(() => oracleAt(o.address), (x) => x.proposerCount === 1)
})

test('submitFact: post a curated fact with a stake', async ({ page }) => {
  const o = wallet.oracles.factProposal
  await openOracle(page, o.address)
  // Participation forms live under the Manage tab.
  await page.getByRole('tab', { name: /Manage/ }).click()
  await page.getByRole('radio', { name: 'Hash text' }).click()
  await page.getByPlaceholder('The claim to record').fill('The event occurred at block 123')
  await page.getByPlaceholder(/ipfs/).fill('ipfs://e2e-fact-evidence')
  await page.getByPlaceholder('e.g. 1').fill('1')
  await page.getByRole('button', { name: 'Submit fact' }).click()
  await poll(() => oracleAt(o.address), (x) => x.factCount === 1)
})

test('voteFact: approve a fact with a stake', async ({ page }) => {
  const o = wallet.oracles.factVoting
  await openOracle(page, o.address)
  // Fact cards (with their per-fact vote controls) live under the Facts tab.
  await page.getByRole('tab', { name: /Facts/ }).click()
  await page.getByRole('radio', { name: 'Approve' }).click()
  await page.getByPlaceholder('e.g. 1').fill('1')
  await page.getByRole('button', { name: 'Cast vote' }).click()
  // On-chain: the fact's approve tally now reflects the wallet's 1e9 stake.
  await poll(() => factAt(o.fact!), (x) => x.approveStake >= 1_000_000_000n)
})

test('submitAiClaim: a locked-in proposer stamps its AI claim', async ({ page }) => {
  const o = wallet.oracles.aiClaim
  await openOracle(page, o.address)
  // Participation forms live under the Manage tab. Paste the payload produced by the
  // REAL runner (mock Anthropic) in globalSetup — not fabricated hashes — through the
  // form's "Paste runner output" mode.
  await page.getByRole('tab', { name: /Manage/ }).click()
  await page.getByRole('radio', { name: 'Paste runner output' }).click()
  await page.getByPlaceholder(/"model_id".*"io_hash"/s).fill(o.runner!)
  await page.getByRole('button', { name: 'Submit AI claim' }).click()
  // On-chain: the wallet's Proposer now carries a claim option (was 0xFF = none).
  await poll(() => proposerAt(o.proposer!), (x) => x.claimOption !== 0xff)
})

test('finalize: permissionless crank of an elapsed proposal window', async ({ page }) => {
  const o = wallet.oracles.finalizeReady
  await openOracle(page, o.address)
  // Elapse the proposal window (the crank requires now >= phase_ends_at).
  const oc = await oracleAt(o.address)
  await setClockTo(Number(oc.phaseEndsAt) + 120)
  // The advance/finalize crank lives under the Manage tab.
  await page.getByRole('tab', { name: /Manage/ }).click()
  await page.getByRole('button', { name: /Finalize proposals/i }).click()
  // On-chain: the conflicting oracle left Proposal (→ FactProposal on dispute).
  await poll(() => oracleAt(o.address), (x) => x.phaseRaw !== 1)
})

test('claim: a winning proposer claims its payout on a resolved oracle', async ({ page }) => {
  const o = wallet.oracles.resolved
  await openOracle(page, o.address)
  // Proposer cards (with their settle/claim controls) live under the Details tab.
  await page.getByRole('tab', { name: /Details/ }).click()
  await page.getByRole('button', { name: /Claim/i }).first().click()
  // On-chain: claim closes the wallet's Proposer account (rent reclaimed).
  await poll(() => getAccountData(o.proposer!), (data) => data === null)
})

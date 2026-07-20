/**
 * Offline unit tests for the bulk-liquidity action builders: `uniformSplit` (the
 * even base-unit distribution behind the group's default "deposit a uniform
 * share of the total to each market"), and the funding→activation handoff
 * (`outcomesReadyToActivate` / `buildBulkActivateSteps`) that lets a deposit
 * crossing an outcome's floor activate it in the SAME batch. Pure, no chain
 * network I/O — `buildActivateSequence`'s PDA derivation is offline compute
 * only, so real `Keypair`-derived addresses are enough (no mock indexer needed).
 */
import { Keypair, type Address } from '@solana/web3.js'
import { beforeAll, describe, expect, it } from 'vitest'

import {
  outcomesReadyToActivate,
  buildBulkActivateSteps,
  uniformSplit,
  type BulkFundingEntry,
} from '../src/market/data/actions/bulkLiquidity'

describe('uniformSplit', () => {
  it('divides evenly when it divides cleanly', () => {
    expect(uniformSplit(9n, 3)).toEqual([3n, 3n, 3n])
    expect(uniformSplit(1_000_000_000n, 2)).toEqual([500_000_000n, 500_000_000n])
  })

  it('spreads the remainder one base unit at a time across the leading shares', () => {
    expect(uniformSplit(10n, 3)).toEqual([4n, 3n, 3n]) // remainder 1 → first share
    expect(uniformSplit(10n, 4)).toEqual([3n, 3n, 2n, 2n]) // remainder 2 → first two
    expect(uniformSplit(7n, 3)).toEqual([3n, 2n, 2n])
  })

  it('always sums back to the exact total (no dust)', () => {
    for (const [total, n] of [
      [1_000_000_001n, 3],
      [123_456_789n, 7],
      [5n, 4],
    ] as const) {
      const shares = uniformSplit(total, n)
      expect(shares).toHaveLength(n)
      expect(shares.reduce((a, b) => a + b, 0n)).toBe(total)
    }
  })

  it('handles n<=0 and a zero total', () => {
    expect(uniformSplit(100n, 0)).toEqual([])
    expect(uniformSplit(0n, 3)).toEqual([0n, 0n, 0n])
  })

  it('rejects a negative total', () => {
    expect(() => uniformSplit(-1n, 2)).toThrow()
  })
})

let MARKET: Address
let ORACLE: Address
let KASS_MINT: Address
let PAYER: Address

beforeAll(async () => {
  MARKET = (await Keypair.generate()).publicKey
  ORACLE = (await Keypair.generate()).publicKey
  KASS_MINT = (await Keypair.generate()).publicKey
  PAYER = (await Keypair.generate()).publicKey
})

function entry(label: string, opts: { totalContributed: bigint; minLiquidity: bigint; amount: bigint }): BulkFundingEntry {
  return { market: MARKET, oracle: ORACLE, label, ...opts }
}

describe('outcomesReadyToActivate', () => {
  it('includes an entry the deposit pushes AT the floor', () => {
    const e = entry('Outcome 0', { totalContributed: 90n, minLiquidity: 100n, amount: 10n })
    expect(outcomesReadyToActivate([e], false)).toEqual([e])
  })

  it('includes an entry the deposit pushes OVER the floor', () => {
    const e = entry('Outcome 0', { totalContributed: 90n, minLiquidity: 100n, amount: 50n })
    expect(outcomesReadyToActivate([e], false)).toEqual([e])
  })

  it('excludes an entry still short of the floor after the deposit', () => {
    const e = entry('Outcome 0', { totalContributed: 90n, minLiquidity: 100n, amount: 5n })
    expect(outcomesReadyToActivate([e], false)).toEqual([])
  })

  it('sweeps in an entry already over its floor even with a ZERO share this round', () => {
    // Nobody has cranked it yet — "if a transaction is about to fund the market,
    // it should also advance to the next phase if possible" applies even when
    // THIS deposit isn't what crossed the floor.
    const e = entry('Outcome 0', { totalContributed: 150n, minLiquidity: 100n, amount: 0n })
    expect(outcomesReadyToActivate([e], false)).toEqual([e])
  })

  it('excludes everything when the shared oracle is terminal', () => {
    const e = entry('Outcome 0', { totalContributed: 150n, minLiquidity: 100n, amount: 0n })
    expect(outcomesReadyToActivate([e], true)).toEqual([])
  })

  it('filters a mixed batch to only the newly/already-eligible entries', () => {
    const ready = entry('Outcome 0', { totalContributed: 90n, minLiquidity: 100n, amount: 10n })
    const notReady = entry('Outcome 1', { totalContributed: 10n, minLiquidity: 100n, amount: 10n })
    expect(outcomesReadyToActivate([ready, notReady], false)).toEqual([ready])
  })

  it('treats a zero/absent floor as already funded', () => {
    const e = entry('Outcome 0', { totalContributed: 0n, minLiquidity: 0n, amount: 0n })
    expect(outcomesReadyToActivate([e], false)).toEqual([e])
  })
})

describe('buildBulkActivateSteps', () => {
  it('builds one full 4-step activate sequence per entry, label-prefixed by outcome', async () => {
    const steps = await buildBulkActivateSteps({
      kassMint: KASS_MINT,
      payer: PAYER,
      entries: [entry('Outcome 2', { totalContributed: 100n, minLiquidity: 100n, amount: 0n })],
    })
    expect(steps).toHaveLength(4) // Initialize question / vault / AMM / Activate
    expect(steps.every((s) => s.label.startsWith('Outcome 2 · '))).toBe(true)
    expect(steps.map((s) => s.label)).toEqual([
      'Outcome 2 · Initialize question',
      'Outcome 2 · Initialize conditional vault',
      'Outcome 2 · Create AMM pool',
      'Outcome 2 · Activate market',
    ])
  })

  it('concatenates sequences for multiple entries, in order, each still label-prefixed', async () => {
    const steps = await buildBulkActivateSteps({
      kassMint: KASS_MINT,
      payer: PAYER,
      entries: [
        entry('Outcome 0', { totalContributed: 100n, minLiquidity: 100n, amount: 0n }),
        entry('Outcome 1', { totalContributed: 100n, minLiquidity: 100n, amount: 0n }),
      ],
    })
    expect(steps).toHaveLength(8)
    expect(steps.slice(0, 4).every((s) => s.label.startsWith('Outcome 0 · '))).toBe(true)
    expect(steps.slice(4, 8).every((s) => s.label.startsWith('Outcome 1 · '))).toBe(true)
  })

  it('returns an empty sequence for no entries', async () => {
    expect(await buildBulkActivateSteps({ kassMint: KASS_MINT, payer: PAYER, entries: [] })).toEqual([])
  })
})

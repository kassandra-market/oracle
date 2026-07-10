/**
 * Offline unit tests for `src/market/lib/marketView.ts` — the pure market
 * presentation + funding/AMM math: status label/tone, `Funding` exit gating,
 * funding progress (clamped, bigint-true `funded`), implied YES probability from
 * pool reserves, and probability/KASS formatting. No React / chain.
 */
import { MarketStatus } from '@kassandra-market/markets'
import { describe, expect, it } from 'vitest'

import type { AmmReserves } from '../src/market/data/markets'
import {
  formatKass,
  formatProbability,
  fundingActions,
  fundingProgress,
  impliedYesProbability,
  statusLabel,
  statusTone,
} from '../src/market/lib/marketView'

const reserves = (base: bigint, quote: bigint): AmmReserves =>
  ({ base, quote }) as unknown as AmmReserves

describe('statusLabel / statusTone', () => {
  it('labels every status', () => {
    expect(statusLabel(MarketStatus.Funding)).toBe('Funding')
    expect(statusLabel(MarketStatus.Active)).toBe('Active')
    expect(statusLabel(MarketStatus.Resolved)).toBe('Resolved')
    expect(statusLabel(MarketStatus.Void)).toBe('Void')
    expect(statusLabel(MarketStatus.Cancelled)).toBe('Cancelled')
    expect(statusLabel(99 as MarketStatus)).toBe('Unknown')
  })

  it('reserves the ember tone for the live (Active) market', () => {
    expect(statusTone(MarketStatus.Active)).toBe('ember')
    expect(statusTone(MarketStatus.Resolved)).toBe('confirmed')
    expect(statusTone(MarketStatus.Funding)).toBe('info')
    expect(statusTone(MarketStatus.Void)).toBe('muted')
    expect(statusTone(MarketStatus.Cancelled)).toBe('muted')
    expect(statusTone(99 as MarketStatus)).toBe('neutral')
  })
})

describe('fundingActions', () => {
  it('activate needs funded + a live oracle; cancel is the only terminal-oracle exit', () => {
    expect(fundingActions(true, false)).toEqual({ canActivate: true, canCancel: false })
    expect(fundingActions(false, false)).toEqual({ canActivate: false, canCancel: false })
    // Terminal oracle: cancel-only even when fully funded (contributions not stranded).
    expect(fundingActions(true, true)).toEqual({ canActivate: false, canCancel: true })
    expect(fundingActions(false, true)).toEqual({ canActivate: false, canCancel: true })
  })
})

describe('fundingProgress', () => {
  it('reports fully funded when the floor is zero/absent', () => {
    expect(fundingProgress({ totalContributed: 0n, minLiquidity: 0n })).toEqual({
      pct: 1,
      funded: true,
    })
  })

  it('is funded on a true bigint compare (>= floor)', () => {
    expect(fundingProgress({ totalContributed: 100n, minLiquidity: 100n })).toEqual({
      pct: 1,
      funded: true,
    })
    expect(fundingProgress({ totalContributed: 150n, minLiquidity: 100n })).toEqual({
      pct: 1,
      funded: true,
    })
  })

  it('reports a clamped 0..1 ratio below the floor', () => {
    expect(fundingProgress({ totalContributed: 25n, minLiquidity: 100n })).toEqual({
      pct: 0.25,
      funded: false,
    })
    expect(fundingProgress({ totalContributed: 0n, minLiquidity: 100n })).toEqual({
      pct: 0,
      funded: false,
    })
  })
})

describe('impliedYesProbability', () => {
  it('is null when reserves are absent or the pool is empty', () => {
    expect(impliedYesProbability(null)).toBeNull()
    expect(impliedYesProbability(undefined)).toBeNull()
    expect(impliedYesProbability(reserves(0n, 0n))).toBeNull()
  })

  it('is quote / (base + quote) — a large YES reserve → low probability', () => {
    expect(impliedYesProbability(reserves(1n, 1n))).toBe(0.5)
    expect(impliedYesProbability(reserves(3n, 1n))).toBe(0.25) // cheap YES → 25%
    expect(impliedYesProbability(reserves(1n, 3n))).toBe(0.75)
  })
})

describe('formatProbability', () => {
  it('renders whole-percent, or an em-dash for null/NaN', () => {
    expect(formatProbability(0.634)).toBe('63%')
    expect(formatProbability(0)).toBe('0%')
    expect(formatProbability(1)).toBe('100%')
    expect(formatProbability(null)).toBe('—')
    expect(formatProbability(Number.NaN)).toBe('—')
  })
})

describe('formatKass (market)', () => {
  it('scales, groups, and trims trailing fraction zeros', () => {
    expect(formatKass(0n)).toBe('0')
    expect(formatKass(1_000_000_000n)).toBe('1')
    expect(formatKass(1_234_500_000_000n)).toBe('1,234.5')
    expect(formatKass(-1_500_000_000n)).toBe('-1.5')
  })
})

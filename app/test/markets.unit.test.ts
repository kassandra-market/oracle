/**
 * Offline unit tests for the pure grouping in `src/market/data/markets.ts`:
 * `groupByOracle` (collapse sub-markets per oracle, first-appearance group order,
 * sub-markets sorted by outcomeIndex, optionsCount from the first non-null) and
 * `isCategorical`. No React / chain — fixtures are minimal MarketSummary shapes.
 */
import { describe, expect, it } from 'vitest'

import type { MarketSummary, OracleGroup } from '../src/market/data/markets'
import { groupByOracle, isCategorical } from '../src/market/data/markets'

/** Minimal MarketSummary shape for the fields groupByOracle reads. */
const summary = (oracle: string, outcomeIndex: number, optionsCount: number | null): MarketSummary =>
  ({ market: { oracle, outcomeIndex }, oracleOptionsCount: optionsCount }) as unknown as MarketSummary

describe('groupByOracle', () => {
  it('collapses each oracle into one group, keeping first-appearance order', () => {
    const groups = groupByOracle([
      summary('OraB', 0, 2),
      summary('OraA', 1, 3),
      summary('OraA', 0, 3),
    ])
    expect(groups.map((g) => g.oracle)).toEqual(['OraB', 'OraA']) // first appearance
    expect(groups.map((g) => g.markets.length)).toEqual([1, 2])
  })

  it('sorts each group’s sub-markets by outcomeIndex ascending', () => {
    const [group] = groupByOracle([summary('Ora', 2, 3), summary('Ora', 0, 3), summary('Ora', 1, 3)])
    expect(group.markets.map((m) => m.market.outcomeIndex)).toEqual([0, 1, 2])
  })

  it('takes optionsCount from the first non-null summary in the group', () => {
    const [group] = groupByOracle([summary('Ora', 0, null), summary('Ora', 1, 4)])
    expect(group.optionsCount).toBe(4)
  })

  it('leaves optionsCount null when every summary lacks it', () => {
    const [group] = groupByOracle([summary('Ora', 0, null)])
    expect(group.optionsCount).toBeNull()
  })

  it('returns an empty list for no markets', () => {
    expect(groupByOracle([])).toEqual([])
  })
})

describe('isCategorical', () => {
  const group = (optionsCount: number | null, marketCount: number): OracleGroup =>
    ({
      oracle: 'Ora',
      optionsCount,
      markets: Array.from({ length: marketCount }, () => ({})),
    }) as unknown as OracleGroup

  it('is true when the oracle has more than two options', () => {
    expect(isCategorical(group(3, 1))).toBe(true)
    expect(isCategorical(group(2, 1))).toBe(false)
  })

  it('falls back to the sub-market count when optionsCount is unknown', () => {
    expect(isCategorical(group(null, 2))).toBe(true) // >1 sub-market → categorical
    expect(isCategorical(group(null, 1))).toBe(false)
  })
})

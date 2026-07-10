/**
 * Offline unit tests for the dashboard stats + filter/sort predicates
 * (`src/lib/oracleStats.ts`). Pure, no chain / RPC / React: the fixtures are
 * decoded-shaped {@link OracleSummary} objects across every phase group, built
 * from the same full-shape `Oracle` the mock fixtures use. Asserts the by-phase
 * counts, the bonds-at-risk bigint sum (active oracles only), the resolved
 * count + recent resolutions, and the filter/sort predicates.
 */
import { Phase } from '@kassandra-market/oracles'
import type { Oracle } from '@kassandra-market/oracles'
import { describe, expect, it } from 'vitest'
import type { OracleSummary } from '../src/data/oracles'
import {
  deriveStats,
  filterByPhaseGroup,
  isTerminal,
  oracleBonds,
  phaseGroup,
  sortOracles,
} from '../src/lib/oracleStats'

// --- minimal full-shape Oracle builder (only the stats-relevant fields vary) --

const ZERO = 0n
function makeOracle(over: Partial<Oracle>): Oracle {
  const base = {
    accountType: 1,
    deadline: ZERO,
    phaseEndsAt: ZERO,
    twapWindow: ZERO,
    optionsCount: 2,
    phaseRaw: Phase.Proposal,
    phase: Phase.Proposal,
    proposerCount: 0,
    survivingCount: 0,
    factCount: 0,
    totalOracleStake: ZERO,
    bondPool: ZERO,
    disputeBondTotal: ZERO,
    settledCount: 0,
    aiFinalizedCount: 0,
    bump: 254,
    resolvedOption: 0xff,
    openChallengeCount: 0,
    thresholdNum: 2n,
    thresholdDen: 3n,
    marketThresholdNum: 1n,
    marketThresholdDen: 10n,
    flipSlashNum: 1n,
    flipSlashDen: 2n,
    phaseWindow: 3600n,
    proposalWindow: 3600n,
    factVoteSlashNum: 1n,
    factVoteSlashDen: 2n,
    rewardProposerWeight: 1n,
    rewardFactWeight: 1n,
    challengeFailUsdcFeeNum: 1n,
    challengeFailUsdcFeeDen: 100n,
    challengeSuccessKassFeeNum: 1n,
    challengeSuccessKassFeeDen: 100n,
    totalCorrectProposerStake: ZERO,
    totalApprovedFactStake: ZERO,
    rewardPool: ZERO,
    rewardEmission: ZERO,
  } as unknown as Oracle
  return { ...base, ...over }
}

function summary(pubkey: string, over: Partial<Oracle>): OracleSummary {
  return { pubkey, oracle: makeOracle({ phase: over.phase, ...over }) }
}

// One oracle per phase group, with distinct bonds + deadlines.
const list: OracleSummary[] = [
  summary('p', { phase: Phase.Proposal, deadline: 100n, bondPool: 0n }),
  summary('fp', { phase: Phase.FactProposal, deadline: 200n, bondPool: 8n }),
  summary('fv', { phase: Phase.FactVoting, deadline: 300n, bondPool: 9n }),
  summary('ai', { phase: Phase.AiClaim, deadline: 400n, bondPool: 2n, disputeBondTotal: 20n }),
  summary('fr', { phase: Phase.FinalRecompute, deadline: 500n, totalOracleStake: 3n }),
  summary('ch', {
    phase: Phase.Challenge,
    deadline: 600n,
    bondPool: 12n,
    disputeBondTotal: 40n,
  }),
  summary('r1', { phase: Phase.Resolved, deadline: 50n, resolvedOption: 1, bondPool: 5n }),
  summary('r2', { phase: Phase.Resolved, deadline: 70n, resolvedOption: 0, bondPool: 99n }),
  summary('dd', { phase: Phase.InvalidDeadend, deadline: 10n, bondPool: 40n }),
]

describe('phaseGroup / isTerminal', () => {
  it('folds the on-chain phases into their coarse browse groups', () => {
    expect(phaseGroup(Phase.Created)).toBe('proposal')
    expect(phaseGroup(Phase.Proposal)).toBe('proposal')
    expect(phaseGroup(Phase.FactProposal)).toBe('inDispute')
    expect(phaseGroup(Phase.FactVoting)).toBe('inDispute')
    expect(phaseGroup(Phase.AiClaim)).toBe('aiClaim')
    expect(phaseGroup(Phase.FinalRecompute)).toBe('aiClaim')
    expect(phaseGroup(Phase.Challenge)).toBe('challenge')
    expect(phaseGroup(Phase.Resolved)).toBe('resolved')
    expect(phaseGroup(Phase.InvalidDeadend)).toBe('invalidDeadend')
    expect(phaseGroup(undefined)).toBe('invalidDeadend')
  })

  it('marks only Resolved + InvalidDeadend as terminal', () => {
    expect(isTerminal(Phase.Resolved)).toBe(true)
    expect(isTerminal(Phase.InvalidDeadend)).toBe(true)
    expect(isTerminal(Phase.Challenge)).toBe(false)
    expect(isTerminal(Phase.Proposal)).toBe(false)
  })
})

describe('oracleBonds', () => {
  it('sums bondPool + disputeBondTotal + totalOracleStake as a bigint', () => {
    expect(oracleBonds(makeOracle({ bondPool: 2n, disputeBondTotal: 20n, totalOracleStake: 1n }))).toBe(
      23n,
    )
  })
})

describe('deriveStats', () => {
  it('computes the by-phase counts, the bonds-at-risk sum, and the resolved count', () => {
    const stats = deriveStats(list)

    expect(stats.counts).toEqual({
      proposal: 1,
      inDispute: 2, // FactProposal + FactVoting
      aiClaim: 2, // AiClaim + FinalRecompute
      challenge: 1,
      resolved: 2,
      invalidDeadend: 1,
      total: 9,
    })

    // Active (non-terminal) bonds: p(0) + fp(8) + fv(9) + ai(22) + fr(3) + ch(52)
    // Terminal r1/r2/dd are excluded.
    expect(stats.bondsAtRisk).toBe(0n + 8n + 9n + 22n + 3n + 52n)
    expect(typeof stats.bondsAtRisk).toBe('bigint')

    expect(stats.resolvedCount).toBe(2)
  })

  it('lists recent resolutions latest-deadline first with their resolvedOption', () => {
    const { recentResolved } = deriveStats(list)
    expect(recentResolved).toEqual([
      { pubkey: 'r2', resolvedOption: 0 }, // deadline 70 > 50
      { pubkey: 'r1', resolvedOption: 1 },
    ])
  })

  it('is safe on an empty list', () => {
    const stats = deriveStats([])
    expect(stats.counts.total).toBe(0)
    expect(stats.bondsAtRisk).toBe(0n)
    expect(stats.resolvedCount).toBe(0)
    expect(stats.recentResolved).toEqual([])
  })
})

describe('filterByPhaseGroup', () => {
  it('all is the identity (new array, not mutated)', () => {
    const out = filterByPhaseGroup(list, 'all')
    expect(out).toHaveLength(list.length)
    expect(out).not.toBe(list)
  })

  it('keeps only the selected group, folding sibling phases in', () => {
    expect(filterByPhaseGroup(list, 'inDispute').map((s) => s.pubkey)).toEqual(['fp', 'fv'])
    expect(filterByPhaseGroup(list, 'aiClaim').map((s) => s.pubkey)).toEqual(['ai', 'fr'])
    expect(filterByPhaseGroup(list, 'challenge').map((s) => s.pubkey)).toEqual(['ch'])
    expect(filterByPhaseGroup(list, 'resolved').map((s) => s.pubkey)).toEqual(['r1', 'r2'])
    expect(filterByPhaseGroup(list, 'proposal').map((s) => s.pubkey)).toEqual(['p'])
  })
})

describe('sortOracles', () => {
  it('sorts by deadline descending without mutating the input', () => {
    const out = sortOracles(list, 'deadline')
    expect(out.map((s) => s.oracle.deadline)).toEqual([600n, 500n, 400n, 300n, 200n, 100n, 70n, 50n, 10n])
    expect(out).not.toBe(list)
    expect(list[0].pubkey).toBe('p') // input unchanged
  })

  it('sorts by bonds-at-risk descending', () => {
    const out = sortOracles(list, 'bondsAtRisk')
    // ch=52, r2=99, ... top should be r2 (99), then ch (52), then dd (40)…
    expect(out[0].pubkey).toBe('r2')
    expect(out[1].pubkey).toBe('ch')
    expect(out[2].pubkey).toBe('dd')
    expect(oracleBonds(out[0].oracle)).toBe(99n)
  })
})

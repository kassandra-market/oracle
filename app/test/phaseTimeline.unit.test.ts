/**
 * Offline unit tests for the pure oracle-detail lifecycle model (RU2):
 * `phaseTimelineModel` (ordered done/current/future split + the terminal end
 * state) and `verdictFor` (resolved / dead-end / in-flight). No React, no chain.
 */
import { Phase } from '@kassandra-market/oracles'
import { describe, expect, it } from 'vitest'

import { phaseTimelineModel, verdictFor } from '../src/lib/phaseTimeline'

const STEP_KEYS = ['proposal', 'factProposal', 'factVoting', 'aiClaim', 'settlement', 'resolution']

/** The current step's key, or undefined when nothing is current (Created/unknown). */
function currentKey(phase: Phase | undefined): string | undefined {
  return phaseTimelineModel({ phase }).find((s) => s.status === 'current')?.key
}

describe('phaseTimelineModel', () => {
  it('always yields the 6 canonical steps in order', () => {
    const steps = phaseTimelineModel({ phase: Phase.Proposal })
    expect(steps.map((s) => s.key)).toEqual(STEP_KEYS)
    expect(steps.at(-1)?.terminal).toBe(true)
    expect(steps.slice(0, -1).every((s) => !s.terminal)).toBe(true)
  })

  it('Proposal → first step current, the rest future', () => {
    const steps = phaseTimelineModel({ phase: Phase.Proposal })
    expect(steps.map((s) => s.status)).toEqual([
      'current',
      'future',
      'future',
      'future',
      'future',
      'future',
    ])
    expect(currentKey(Phase.Proposal)).toBe('proposal')
  })

  it('FactVoting → the two prior steps done, factVoting current, later future', () => {
    const steps = phaseTimelineModel({ phase: Phase.FactVoting })
    expect(steps.map((s) => s.status)).toEqual([
      'done',
      'done',
      'current',
      'future',
      'future',
      'future',
    ])
  })

  it('AiClaim → three done, aiClaim current', () => {
    const steps = phaseTimelineModel({ phase: Phase.AiClaim })
    expect(steps.map((s) => s.status)).toEqual([
      'done',
      'done',
      'done',
      'current',
      'future',
      'future',
    ])
    expect(currentKey(Phase.AiClaim)).toBe('aiClaim')
  })

  it('Challenge relabels the settlement step and marks it current', () => {
    const steps = phaseTimelineModel({ phase: Phase.Challenge })
    const settlement = steps.find((s) => s.key === 'settlement')!
    expect(settlement.status).toBe('current')
    expect(settlement.label).toBe('Challenge')
  })

  it('FinalRecompute relabels the settlement step', () => {
    const settlement = phaseTimelineModel({ phase: Phase.FinalRecompute }).find(
      (s) => s.key === 'settlement',
    )!
    expect(settlement.status).toBe('current')
    expect(settlement.label).toBe('Final recompute')
  })

  it('Resolved → everything done, the terminal step current (not a dead-end)', () => {
    const steps = phaseTimelineModel({ phase: Phase.Resolved })
    expect(steps.slice(0, -1).every((s) => s.status === 'done')).toBe(true)
    const end = steps.at(-1)!
    expect(end.status).toBe('current')
    expect(end.terminal).toBe(true)
    expect(end.deadend).toBe(false)
    expect(end.label).toBe('Resolved')
  })

  it('InvalidDeadend → terminal current, flagged as a dead-end with its own label', () => {
    const end = phaseTimelineModel({ phase: Phase.InvalidDeadend }).at(-1)!
    expect(end.status).toBe('current')
    expect(end.terminal).toBe(true)
    expect(end.deadend).toBe(true)
    expect(end.label).toBe('Dead end')
  })

  it('Created / undefined → nothing current, all future', () => {
    expect(currentKey(Phase.Created)).toBeUndefined()
    expect(currentKey(undefined)).toBeUndefined()
    expect(phaseTimelineModel({ phase: undefined }).every((s) => s.status === 'future')).toBe(true)
  })
})

describe('verdictFor', () => {
  it('Resolved with an option → confirmed "Resolved · Option N"', () => {
    const v = verdictFor({ phase: Phase.Resolved, resolvedOption: 1 })
    expect(v.kind).toBe('resolved')
    expect(v.title).toBe('Resolved · Option 1')
    expect(v.tone).toBe('confirmed')
  })

  it('Resolved with the 0xFF sentinel → "Resolved" (no option)', () => {
    const v = verdictFor({ phase: Phase.Resolved, resolvedOption: 0xff })
    expect(v.kind).toBe('resolved')
    expect(v.title).toBe('Resolved')
  })

  it('InvalidDeadend → muted dead-end verdict', () => {
    const v = verdictFor({ phase: Phase.InvalidDeadend, resolvedOption: 0xff })
    expect(v.kind).toBe('deadend')
    expect(v.title).toBe('No resolution · dead-ended')
    expect(v.tone).toBe('muted')
  })

  it('FactVoting → in-flight with the phase label + a what-next line', () => {
    const v = verdictFor({ phase: Phase.FactVoting, resolvedOption: 0xff })
    expect(v.kind).toBe('in-flight')
    expect(v.title).toBe('In fact voting')
    expect(v.detail.length).toBeGreaterThan(0)
    // Subtle cyan `info` hint for the fact-staking phases (see phaseView).
    expect(v.tone).toBe('info')
  })

  it('Challenge → in-flight, reuses the ember tone from phaseView', () => {
    const v = verdictFor({ phase: Phase.Challenge, resolvedOption: 0xff })
    expect(v.kind).toBe('in-flight')
    expect(v.tone).toBe('ember')
  })
})

/**
 * Offline unit tests for the pure presentation helpers in `src/lib/oracleView.ts`
 * — phase → chip mapping, id/hash truncation + hex, thousands grouping, scaled
 * KASS formatting, and the hand-rolled relative-deadline / window labels. No React,
 * no chain; `Date.now()` is faked for the relative-time assertions.
 */
import { Phase } from '@kassandra-market/oracles'
import { afterEach, describe, expect, it, vi } from 'vitest'

import {
  formatKass,
  groupDigits,
  hashHex,
  hashPreview,
  phaseView,
  relativeDeadline,
  truncateMiddle,
  windowLabel,
} from '../src/lib/oracleView'

describe('phaseView', () => {
  it('maps every phase to its label + on-brand tone', () => {
    expect(phaseView(Phase.Created)).toEqual({ label: 'Created', tone: 'neutral' })
    expect(phaseView(Phase.Proposal)).toEqual({ label: 'Proposal', tone: 'neutral' })
    expect(phaseView(Phase.FactProposal)).toEqual({ label: 'Fact proposal', tone: 'info' })
    expect(phaseView(Phase.FactVoting)).toEqual({ label: 'Fact voting', tone: 'info' })
    expect(phaseView(Phase.AiClaim)).toEqual({ label: 'AI claim', tone: 'accent' })
    expect(phaseView(Phase.Challenge)).toEqual({ label: 'Challenged', tone: 'ember' })
    expect(phaseView(Phase.FinalRecompute)).toEqual({ label: 'Final recompute', tone: 'accent' })
    expect(phaseView(Phase.Resolved)).toEqual({ label: 'Resolved', tone: 'confirmed' })
    expect(phaseView(Phase.InvalidDeadend)).toEqual({ label: 'Dead end', tone: 'muted' })
  })

  it('the ember tone is reserved for the single Challenge moment', () => {
    const embers = [
      Phase.Created,
      Phase.Proposal,
      Phase.FactProposal,
      Phase.FactVoting,
      Phase.AiClaim,
      Phase.Challenge,
      Phase.FinalRecompute,
      Phase.Resolved,
      Phase.InvalidDeadend,
    ].filter((p) => phaseView(p).tone === 'ember')
    expect(embers).toEqual([Phase.Challenge])
  })

  it('falls back to Unknown/muted for an undefined or out-of-range phase', () => {
    expect(phaseView(undefined)).toEqual({ label: 'Unknown', tone: 'muted' })
    expect(phaseView(999 as unknown as Phase)).toEqual({ label: 'Unknown', tone: 'muted' })
  })
})

describe('truncateMiddle', () => {
  it('returns short values unchanged (<= head + tail + 1)', () => {
    expect(truncateMiddle('abcdefghi')).toBe('abcdefghi') // 9 == 4+4+1
    expect(truncateMiddle('short')).toBe('short')
  })

  it('truncates the middle keeping head + tail', () => {
    expect(truncateMiddle('ABCDEFGHIJKLMNOP')).toBe('ABCD…MNOP')
    expect(truncateMiddle('ABCDEFGHIJKLMNOP', 2, 3)).toBe('AB…NOP')
  })

  it('only truncates once past the head+tail+1 boundary', () => {
    expect(truncateMiddle('abcdefghij')).toBe('abcd…ghij') // 10 > 9 → truncated
  })
})

describe('hashPreview / hashHex', () => {
  const bytes = Uint8Array.from([0x1a, 0x2b, 0x00, 0xff, 0xc3, 0xd4])

  it('hashHex is full lower-case hex with a 0x prefix + zero-padding', () => {
    expect(hashHex(bytes)).toBe('0x1a2b00ffc3d4')
    expect(hashHex(Uint8Array.from([0x0a, 0x00]))).toBe('0x0a00')
  })

  it('hashPreview shows a 0x head…tail slice of the hex', () => {
    // hex = "1a2b00ffc3d4"; head 4 = "1a2b", tail 4 = "c3d4"
    expect(hashPreview(bytes)).toBe('0x1a2b…c3d4')
    expect(hashPreview(bytes, 2, 2)).toBe('0x1a…d4')
  })

  it('renders an empty byte array as just the prefix', () => {
    expect(hashHex(new Uint8Array())).toBe('0x')
  })
})

describe('groupDigits', () => {
  it('inserts thousands separators', () => {
    expect(groupDigits(0n)).toBe('0')
    expect(groupDigits(999n)).toBe('999')
    expect(groupDigits(1000n)).toBe('1,000')
    expect(groupDigits(1234567n)).toBe('1,234,567')
    expect(groupDigits(1000000000n)).toBe('1,000,000,000')
  })

  it('handles negatives', () => {
    expect(groupDigits(-1234n)).toBe('-1,234')
  })
})

describe('formatKass', () => {
  it('scales raw base units by 9 decimals, trimming the fraction', () => {
    expect(formatKass(0n)).toBe('0')
    expect(formatKass(1_000_000_000n)).toBe('1') // exactly 1 KASS
    expect(formatKass(1_500_000_000n)).toBe('1.5')
    expect(formatKass(500_000_000n)).toBe('0.5') // sub-1 amount
    expect(formatKass(1n)).toBe('0.000000001') // one base unit
  })

  it('groups the whole part and trims trailing fraction zeros', () => {
    expect(formatKass(1_234_567_000_000_000n)).toBe('1,234,567')
    expect(formatKass(2_100_000_000n)).toBe('2.1')
    expect(formatKass(1_050_000_000n)).toBe('1.05')
  })

  it('handles negative amounts', () => {
    expect(formatKass(-1_500_000_000n)).toBe('-1.5')
  })
})

describe('relativeDeadline', () => {
  afterEach(() => vi.useRealTimers())

  it('labels a future deadline as "ends in …"', () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2026-01-01T00:00:00Z'))
    const now = 1_767_225_600n // 2026-01-01T00:00:00Z in unix seconds
    expect(relativeDeadline(now + 3n * 86_400n)).toBe('ends in 3d')
    expect(relativeDeadline(now + 5n * 3_600n)).toBe('ends in 5h')
    expect(relativeDeadline(now + 45n * 60n)).toBe('ends in 45m')
    expect(relativeDeadline(now + 30n)).toBe('ends in 30s')
  })

  it('labels a past deadline as "ended … ago"', () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2026-01-01T00:00:00Z'))
    const now = 1_767_225_600n
    expect(relativeDeadline(now - 2n * 86_400n)).toBe('ended 2d ago')
    expect(relativeDeadline(now - 90n)).toBe('ended 1m ago')
  })
})

describe('windowLabel', () => {
  it('renders a coarse duration for a governable window', () => {
    expect(windowLabel(30n)).toBe('30s')
    expect(windowLabel(60n)).toBe('1m')
    expect(windowLabel(2_700n)).toBe('45m')
    expect(windowLabel(3_600n)).toBe('1h')
    expect(windowLabel(172_800n)).toBe('2d')
  })
})

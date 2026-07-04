/**
 * Pure presentation helpers for the oracle browse views (FA3) — phase → chip
 * mapping, hash/pubkey truncation, relative-deadline formatting (hand-rolled
 * from bigint unix seconds, NO date lib), and base-unit digit grouping. No
 * React here; the pages + chip components consume these.
 */
import { Phase } from '@kassandra/sdk'
import type { ChipTone } from '../components/oracles/Chip'

/** `resolvedOption` sentinel: no valid option (dead-end). Mirrors `state.rs` 0xFF. */
export const RESOLVED_OPTION_NONE = 0xff

export interface PhaseView {
  /** Accessible, human label for the phase (never color-only). */
  label: string
  /** On-brand chip tone. Ember is reserved for the single "Challenged" punctuation moment. */
  tone: ChipTone
}

/**
 * Map an on-chain {@link Phase} to a readable label + an on-brand chip tone.
 * Ember is used ONLY for the active Challenge (the 1–2 status punctuation
 * moments per the Auros rules); resolution is a calm "confirmed" chestnut,
 * dead-ends are muted stone, everything mid-flight is neutral sepia/bronze.
 */
export function phaseView(phase: Phase | undefined): PhaseView {
  switch (phase) {
    case Phase.Created:
      return { label: 'Created', tone: 'neutral' }
    case Phase.Proposal:
      return { label: 'Proposal', tone: 'neutral' }
    case Phase.FactProposal:
      return { label: 'Fact proposal', tone: 'neutral' }
    case Phase.FactVoting:
      return { label: 'Fact voting', tone: 'neutral' }
    case Phase.AiClaim:
      return { label: 'AI claim', tone: 'neutral' }
    case Phase.Challenge:
      return { label: 'Challenged', tone: 'ember' }
    case Phase.FinalRecompute:
      return { label: 'Final recompute', tone: 'neutral' }
    case Phase.Resolved:
      return { label: 'Resolved', tone: 'confirmed' }
    case Phase.InvalidDeadend:
      return { label: 'Dead end', tone: 'muted' }
    default:
      return { label: 'Unknown', tone: 'muted' }
  }
}

/** Truncate a long identifier keeping `head`+`tail` chars: `Abc1…Xy9z`. */
export function truncateMiddle(value: string, head = 4, tail = 4): string {
  if (value.length <= head + tail + 1) return value
  return `${value.slice(0, head)}…${value.slice(-tail)}`
}

/** Lower-case hex string for raw bytes (no `0x`). */
function toHex(bytes: Uint8Array): string {
  let out = ''
  for (const b of bytes) out += b.toString(16).padStart(2, '0')
  return out
}

/** Short `0x1a2b…c3d4` preview of a 32-byte hash (prompt/content/model/params/io). */
export function hashPreview(bytes: Uint8Array, head = 4, tail = 4): string {
  const hex = toHex(bytes)
  return `0x${hex.slice(0, head)}…${hex.slice(-tail)}`
}

/** Full `0x…` hex string of a hash (used as the copy payload / title). */
export function hashHex(bytes: Uint8Array): string {
  return `0x${toHex(bytes)}`
}

/** Group an integer's digits with thousands separators: `1234567n` → `1,234,567`. */
export function groupDigits(n: bigint): string {
  const neg = n < 0n
  const s = (neg ? -n : n).toString().replace(/\B(?=(\d{3})+(?!\d))/g, ',')
  return neg ? `-${s}` : s
}

/** A coarse duration label for a non-negative number of seconds: `3d` / `4h` / `12m` / `30s`. */
function coarseDuration(seconds: number): string {
  if (seconds >= 86400) return `${Math.floor(seconds / 86400)}d`
  if (seconds >= 3600) return `${Math.floor(seconds / 3600)}h`
  if (seconds >= 60) return `${Math.floor(seconds / 60)}m`
  return `${seconds}s`
}

/**
 * Relative deadline from a bigint unix-seconds timestamp, hand-rolled (no date
 * lib): `ends in 3d` when future, `ended 2d ago` when past.
 */
export function relativeDeadline(unixSeconds: bigint): string {
  const now = BigInt(Math.floor(Date.now() / 1000))
  const diff = unixSeconds - now
  const abs = diff < 0n ? -diff : diff
  const label = coarseDuration(Number(abs))
  return diff >= 0n ? `ends in ${label}` : `ended ${label} ago`
}

/** Readable duration (seconds) for governable windows, e.g. `1h` / `45m`. */
export function windowLabel(seconds: bigint): string {
  const n = Number(seconds < 0n ? -seconds : seconds)
  return coarseDuration(n)
}

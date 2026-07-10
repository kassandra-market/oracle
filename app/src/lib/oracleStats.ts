/**
 * Pure, side-effect-free dashboard stats + filter/sort predicates for the oracle
 * browse page (RU1). Everything here is computed CLIENT-SIDE from the already
 * decoded {@link OracleSummary} list `useOracles()` fetched — NO new on-chain
 * read, NO React. The `/oracles` page (and the offline unit test) consume these.
 *
 * Grouping mirrors the on-chain {@link Phase} lifecycle
 * (`sdk/src/constants.ts`): Proposal → FactProposal → FactVoting → AiClaim →
 * Challenge → FinalRecompute → Resolved / InvalidDeadend. The mid-flight phases
 * collapse into a handful of human-sensible groups; the two terminal phases
 * (Resolved, InvalidDeadend) are "settled" and excluded from the active
 * bonds-at-risk figure.
 */
import { Phase } from '@kassandra-market/oracles'
import type { OracleSummary } from '../data/oracles'

/**
 * A coarse phase grouping used for both the by-phase counts and the filter
 * chips. Several on-chain {@link Phase} values fold into one group:
 *   - `proposal`       — Created, Proposal (the opening window)
 *   - `inDispute`      — FactProposal, FactVoting (facts being staked + voted)
 *   - `aiClaim`        — AiClaim, FinalRecompute (AI adjudication + recompute)
 *   - `challenge`      — Challenge (a market is live — the one ember moment)
 *   - `resolved`       — Resolved (terminal, a valid answer)
 *   - `invalidDeadend` — InvalidDeadend (terminal, no valid answer)
 */
export type PhaseGroup =
  | 'proposal'
  | 'inDispute'
  | 'aiClaim'
  | 'challenge'
  | 'resolved'
  | 'invalidDeadend'

/** A filter selection: any single {@link PhaseGroup} shown as a chip, or `all`. */
export type PhaseFilter =
  | 'all'
  | 'proposal'
  | 'inDispute'
  | 'aiClaim'
  | 'challenge'
  | 'resolved'
  | 'invalidDeadend'

/** How the grid is ordered. */
export type SortBy = 'deadline' | 'bondsAtRisk'

/** Map an on-chain {@link Phase} to its coarse browse group. */
export function phaseGroup(phase: Phase | undefined): PhaseGroup {
  switch (phase) {
    case Phase.Created:
    case Phase.Proposal:
      return 'proposal'
    case Phase.FactProposal:
    case Phase.FactVoting:
      return 'inDispute'
    case Phase.AiClaim:
    case Phase.FinalRecompute:
      return 'aiClaim'
    case Phase.Challenge:
      return 'challenge'
    case Phase.Resolved:
      return 'resolved'
    case Phase.InvalidDeadend:
      return 'invalidDeadend'
    default:
      // Unknown / out-of-range phase — treat as a dead-end so it never inflates
      // an active group or the bonds-at-risk figure.
      return 'invalidDeadend'
  }
}

/** True once an oracle has reached a terminal phase (Resolved or InvalidDeadend). */
export function isTerminal(phase: Phase | undefined): boolean {
  return phase === Phase.Resolved || phase === Phase.InvalidDeadend
}

/**
 * Per-oracle capital at stake: the surviving-proposer bond pool plus the
 * dispute bond plus any staked total. Raw base units (unscaled) — the UI groups
 * the digits. Used both for the headline figure (summed over active oracles)
 * and the `bondsAtRisk` sort key (per oracle, regardless of phase).
 */
export function oracleBonds(oracle: OracleSummary['oracle']): bigint {
  return oracle.bondPool + oracle.disputeBondTotal + oracle.totalOracleStake
}

/** Counts per {@link PhaseGroup} plus a grand total. */
export interface PhaseCounts {
  proposal: number
  inDispute: number
  aiClaim: number
  challenge: number
  resolved: number
  invalidDeadend: number
  total: number
}

/** The derived dashboard figures for a list of oracles. */
export interface OracleStats {
  /** By-phase-group counts + the total. */
  counts: PhaseCounts
  /**
   * Sum of {@link oracleBonds} across NON-terminal (active) oracles — the capital
   * still contestable on chain, in raw base units (the UI scales it to KASS).
   */
  bondsAtRisk: bigint
  /**
   * The monetary breakdown of {@link bondsAtRisk} across active oracles — the
   * surviving-proposer bond pool, the dispute bonds, and the total staked. Raw
   * base units (the UI scales them to KASS). Sum == {@link bondsAtRisk}.
   */
  bondPoolActive: bigint
  disputeBondsActive: bigint
  stakedActive: bigint
  /** How many oracles are in the terminal `Resolved` phase. */
  resolvedCount: number
  /**
   * The most-recently-resolved oracles (latest deadline first), each with its
   * `resolvedOption`, capped for a compact "recent resolutions" strip.
   */
  recentResolved: { pubkey: string; resolvedOption: number }[]
}

/** How many recent resolutions {@link deriveStats} surfaces. */
const RECENT_RESOLVED_LIMIT = 3

/**
 * Derive the dashboard stats from an already-decoded oracle list. Pure: no
 * network, no mutation of the input. Safe on an empty list (all zeros).
 */
export function deriveStats(summaries: OracleSummary[]): OracleStats {
  const counts: PhaseCounts = {
    proposal: 0,
    inDispute: 0,
    aiClaim: 0,
    challenge: 0,
    resolved: 0,
    invalidDeadend: 0,
    total: 0,
  }
  let bondsAtRisk = 0n
  let bondPoolActive = 0n
  let disputeBondsActive = 0n
  let stakedActive = 0n

  for (const { oracle } of summaries) {
    counts[phaseGroup(oracle.phase)] += 1
    counts.total += 1
    if (!isTerminal(oracle.phase)) {
      bondsAtRisk += oracleBonds(oracle)
      bondPoolActive += oracle.bondPool
      disputeBondsActive += oracle.disputeBondTotal
      stakedActive += oracle.totalOracleStake
    }
  }

  const recentResolved = summaries
    .filter((s) => s.oracle.phase === Phase.Resolved)
    .slice()
    .sort((a, b) => cmpDeadlineDesc(a, b))
    .slice(0, RECENT_RESOLVED_LIMIT)
    .map((s) => ({ pubkey: s.pubkey, resolvedOption: s.oracle.resolvedOption }))

  return {
    counts,
    bondsAtRisk,
    bondPoolActive,
    disputeBondsActive,
    stakedActive,
    resolvedCount: counts.resolved,
    recentResolved,
  }
}

/**
 * Keep only the oracles in a given filter group; `all` is the identity. Pure —
 * returns a new array, never mutates. Composable with a text search + sort.
 */
export function filterByPhaseGroup(
  summaries: OracleSummary[],
  filter: PhaseFilter,
): OracleSummary[] {
  if (filter === 'all') return summaries.slice()
  return summaries.filter((s) => phaseGroup(s.oracle.phase) === filter)
}

/** Compare two summaries by deadline, latest first (mirrors `fetchOracles`). */
function cmpDeadlineDesc(a: OracleSummary, b: OracleSummary): number {
  return b.oracle.deadline > a.oracle.deadline
    ? 1
    : b.oracle.deadline < a.oracle.deadline
      ? -1
      : 0
}

/** Compare two summaries by capital-at-stake, largest first. */
function cmpBondsDesc(a: OracleSummary, b: OracleSummary): number {
  const da = oracleBonds(a.oracle)
  const db = oracleBonds(b.oracle)
  return db > da ? 1 : db < da ? -1 : 0
}

/**
 * Order a list by the chosen key (descending): `deadline` = latest-expiring
 * first; `bondsAtRisk` = most capital at stake first. Stable-ish and pure —
 * returns a new array. Composable after {@link filterByPhaseGroup} + search.
 */
export function sortOracles(summaries: OracleSummary[], by: SortBy): OracleSummary[] {
  const out = summaries.slice()
  out.sort(by === 'bondsAtRisk' ? cmpBondsDesc : cmpDeadlineDesc)
  return out
}

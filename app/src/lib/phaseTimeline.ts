/**
 * Pure lifecycle-model helpers for the oracle DETAIL view (RU2) — presentation
 * over already-decoded {@link Oracle} data (NO chain read, NO React). Two pure,
 * unit-testable functions:
 *
 *   - {@link phaseTimelineModel} — the ordered lifecycle steps + each step's
 *     `done` / `current` / `future` status, with a distinct terminal end
 *     (Resolved vs InvalidDeadend), derived solely from `oracle.phase`.
 *   - {@link verdictFor} — the at-a-glance verdict (resolved / dead-end /
 *     in-flight) with a headline + one-line "what's next", reusing `phaseView`.
 */
import { Phase } from '@kassandra-market/oracles'
import type { Oracle } from '@kassandra-market/oracles'
import type { ChipTone } from '../components/oracles/Chip'
import { RESOLVED_OPTION_NONE, phaseView } from './oracleView'

export type StepStatus = 'done' | 'current' | 'future'

/** One rendered step of the lifecycle strip. */
export interface TimelineStep {
  /** Stable key (also the aria list order). */
  key: string
  /** Human label (the two folded stages relabel to the live phase). */
  label: string
  status: StepStatus
  /** The terminal resolution step (Resolved / InvalidDeadend). */
  terminal: boolean
  /** True only for the InvalidDeadend terminal — a muted (non-chestnut) end. */
  deadend: boolean
}

/**
 * The canonical lifecycle spine. Each stage covers one or more on-chain phases:
 * the settlement stage folds Challenge + FinalRecompute (the two mid-settlement
 * paths); the terminal stage folds Resolved + InvalidDeadend (the two ends).
 * `Phase.Created` (0, reserved/unused) maps to no stage → the whole strip reads
 * `future`.
 */
interface Stage {
  key: string
  label: string
  phases: Phase[]
  terminal?: boolean
}

const STAGES: Stage[] = [
  { key: 'proposal', label: 'Proposal', phases: [Phase.Proposal] },
  { key: 'factProposal', label: 'Fact proposal', phases: [Phase.FactProposal] },
  { key: 'factVoting', label: 'Fact voting', phases: [Phase.FactVoting] },
  { key: 'aiClaim', label: 'AI claim', phases: [Phase.AiClaim] },
  { key: 'settlement', label: 'Settlement', phases: [Phase.Challenge, Phase.FinalRecompute] },
  {
    key: 'resolution',
    label: 'Resolved',
    phases: [Phase.Resolved, Phase.InvalidDeadend],
    terminal: true,
  },
]

/** Index of the stage owning `phase`, or -1 (Created / undefined / unknown). */
function stageIndexOf(phase: Phase | undefined): number {
  if (phase === undefined) return -1
  return STAGES.findIndex((s) => s.phases.includes(phase))
}

/**
 * The ordered lifecycle steps for an oracle, each tagged `done` / `current` /
 * `future` from `oracle.phase`. The folded stages relabel to the live phase when
 * current (Challenge / Final recompute; Resolved / Dead end); the terminal step
 * carries the distinct end state.
 */
export function phaseTimelineModel(oracle: Pick<Oracle, 'phase'>): TimelineStep[] {
  const currentIndex = stageIndexOf(oracle.phase)
  return STAGES.map((stage, i) => {
    const status: StepStatus =
      currentIndex < 0 || i > currentIndex ? 'future' : i === currentIndex ? 'current' : 'done'

    let label = stage.label
    if (stage.key === 'settlement') {
      if (oracle.phase === Phase.Challenge) label = 'Challenge'
      else if (oracle.phase === Phase.FinalRecompute) label = 'Final recompute'
    } else if (stage.terminal && oracle.phase === Phase.InvalidDeadend) {
      label = 'Dead end'
    }

    return {
      key: stage.key,
      label,
      status,
      terminal: !!stage.terminal,
      deadend: !!stage.terminal && oracle.phase === Phase.InvalidDeadend,
    }
  })
}

export type VerdictKind = 'resolved' | 'deadend' | 'in-flight'

/** The at-a-glance verdict banner model. */
export interface Verdict {
  kind: VerdictKind
  /** Headline (rendered as the banner h2). */
  title: string
  /** One-line supporting detail — the outcome, or what happens next. */
  detail: string
  /** On-brand tone (reused from `phaseView` for in-flight). */
  tone: ChipTone
}

/** One-line "what's next" per in-flight phase. */
const NEXT_HINT: Partial<Record<Phase, string>> = {
  [Phase.Created]: 'Proposals open once the deadline passes.',
  [Phase.Proposal]: 'Proposers stake bonds until the proposal window closes.',
  [Phase.FactProposal]: 'Supporting facts are submitted for the options.',
  [Phase.FactVoting]: 'Finalize the facts when the voting window closes.',
  [Phase.AiClaim]: 'An AI claim settles the winning option unless challenged.',
  [Phase.Challenge]: 'A challenge market resolves via TWAP, then recomputes.',
  [Phase.FinalRecompute]: 'Finalize the oracle to record the resolved option.',
}

/**
 * The verdict for an oracle: a calm confirmed "Resolved · Option N" when
 * resolved, a muted "No resolution · dead-ended" on a dead-end, else the
 * in-flight phase + a one-line "what's next" (tone reused from `phaseView`).
 */
export function verdictFor(oracle: Pick<Oracle, 'phase' | 'resolvedOption'>): Verdict {
  if (oracle.phase === Phase.Resolved) {
    const hasOption = oracle.resolvedOption !== RESOLVED_OPTION_NONE
    return {
      kind: 'resolved',
      title: hasOption ? `Resolved · Option ${oracle.resolvedOption}` : 'Resolved',
      detail: hasOption
        ? 'The dispute settled on this option — participants can claim their payouts.'
        : 'The dispute resolved with no single valid option.',
      tone: 'confirmed',
    }
  }
  if (oracle.phase === Phase.InvalidDeadend) {
    return {
      kind: 'deadend',
      title: 'No resolution · dead-ended',
      detail: 'The dispute could not reach a valid option; bonds are returned or swept.',
      tone: 'muted',
    }
  }
  const { label, tone } = phaseView(oracle.phase)
  return {
    kind: 'in-flight',
    title: `In ${label.toLowerCase()}`,
    detail: NEXT_HINT[oracle.phase ?? Phase.Created] ?? 'The dispute is progressing through its lifecycle.',
    tone,
  }
}

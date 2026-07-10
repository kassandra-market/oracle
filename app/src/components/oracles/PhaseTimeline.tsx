import type { Oracle } from '@kassandra-market/oracles'
import { phaseTimelineModel, type StepStatus, type TimelineStep } from '../../lib/phaseTimeline'
import { relativeDeadline } from '../../lib/oracleView'

/** Segment (progress-bar) fill per step status. Current = the chestnut highlight. */
function barClass(step: TimelineStep): string {
  if (step.status === 'current') return step.deadend ? 'bg-stone' : 'bg-chestnut'
  if (step.status === 'done') return 'bg-driftwood'
  return 'bg-pebble/50'
}

/** Label colour per step status. */
function labelClass(status: StepStatus): string {
  if (status === 'current') return 'text-chestnut font-medium'
  if (status === 'done') return 'text-driftwood'
  return 'text-stone'
}

/**
 * The oracle lifecycle strip — a flat, hairline segmented progress meter
 * (Proposal → Fact proposal → Fact voting → AI claim → Settlement → Resolved,
 * with a distinct dead-end terminal). Past steps read muted-done, the CURRENT
 * step is the single chestnut punctuation (its deadline shown top-right), future
 * steps are faint. Renders as an ordered list; the current step is
 * `aria-current="step"`. Stacks vertically on mobile, a row on ≥sm.
 */
export function PhaseTimeline({ oracle }: { oracle: Oracle }) {
  const steps = phaseTimelineModel(oracle)
  const current = steps.find((s) => s.status === 'current')

  return (
    <section
      aria-label="Dispute lifecycle"
      className="mt-6 rounded-card border border-pebble bg-pure-card p-5"
    >
      <div className="flex items-baseline justify-between gap-3">
        <span className="font-inter text-[11px] uppercase tracking-[0.06em] text-driftwood">
          Lifecycle
        </span>
        {current ? (
          <span className="font-inter text-[12px] text-bronze">
            {current.terminal ? current.label : relativeDeadline(oracle.phaseEndsAt)}
          </span>
        ) : null}
      </div>
      <ol className="mt-3 flex flex-col gap-2.5 sm:flex-row sm:gap-2">
        {steps.map((step) => (
          <li
            key={step.key}
            aria-current={step.status === 'current' ? 'step' : undefined}
            className="flex flex-1 items-center gap-3 sm:flex-col sm:items-stretch sm:gap-1.5"
          >
            <span
              aria-hidden
              className={`h-1.5 w-10 shrink-0 rounded-full sm:w-full ${barClass(step)}`}
            />
            <span className={`font-inter text-[12px] ${labelClass(step.status)}`}>
              {step.label}
            </span>
          </li>
        ))}
      </ol>
    </section>
  )
}

export default PhaseTimeline

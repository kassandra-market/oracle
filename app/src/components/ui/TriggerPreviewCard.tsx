import type { ReactNode } from 'react'

export interface TriggerPreviewCardProps {
  /** The "When" clause label text. Defaults to "When". */
  whenLabel?: string
  /** The condition line. Use `variable` to highlight the value slot in ember. */
  condition: ReactNode
  /** Highlighted variable slot, rendered in ember orange (monospace). */
  variable?: ReactNode
  /** Text for the subdued action row. Defaults to "+ Add Action". */
  actionLabel?: string
  className?: string
}

/**
 * Auros trigger preview — a nested sub-card (8px radius, cream fill) that reads
 * like a rule snippet: a driftwood "When" label, an Inter 14px condition line
 * with a monospace variable slot highlighted in ember orange, and a subdued
 * stone "+ Add Action" row.
 */
export function TriggerPreviewCard({
  whenLabel = 'When',
  condition,
  variable,
  actionLabel = '+ Add Action',
  className = '',
}: TriggerPreviewCardProps) {
  return (
    <div className={`rounded-tag border border-pebble bg-soft-cream p-4 ${className}`}>
      <div className="font-inter text-[11px] font-medium uppercase tracking-[0.08em] text-driftwood">
        {whenLabel}
      </div>
      <p className="mt-1.5 font-inter text-[14px] leading-snug text-charcoal-bark">
        {condition}
        {variable ? (
          <>
            {' '}
            <span className="font-mono text-lavender-phosphor">{variable}</span>
          </>
        ) : null}
      </p>
      <div className="mt-3 border-t border-pebble pt-2.5 font-inter text-[13px] text-stone">
        {actionLabel}
      </div>
    </div>
  )
}

export default TriggerPreviewCard

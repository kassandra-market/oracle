import type { HTMLAttributes, ReactNode } from 'react'

/**
 * On-brand chip tones. `ember` is the reserved punctuation accent (the active
 * "Challenged" moment). `info` (cyan) and `accent` (lavender) are SUBTLE, low-
 * opacity phase hints — just enough color to let the eye distinguish the
 * mid-flight phases at a glance, never loud. The label text is always present, so
 * color is never the only signal (a11y `color-not-only`).
 */
export type ChipTone = 'neutral' | 'info' | 'accent' | 'ember' | 'confirmed' | 'muted'

const toneClass: Record<ChipTone, string> = {
  // Opening / quiet default — achromatic warm.
  neutral: 'border-pebble bg-soft-cream text-bronze',
  // Subtle cyan hint — evidence being staked/voted (in dispute).
  info: 'border-cobalt/30 bg-cobalt/10 text-cyan-phosphor',
  // Subtle lavender hint — AI adjudication in progress.
  accent: 'border-lavender-phosphor/30 bg-lavender-phosphor/10 text-lavender-phosphor',
  // The ember punctuation moment — the active challenge.
  ember: 'border-ember-orange/40 bg-ember-orange/10 text-ember-orange',
  // A calm, grounded aqua "confirmed" for resolution.
  confirmed: 'border-chestnut/30 bg-chestnut/10 text-chestnut',
  // Lowest-emphasis stone for dead-ends / disqualified.
  muted: 'border-pebble bg-transparent text-stone',
}

export interface ChipProps extends HTMLAttributes<HTMLSpanElement> {
  tone?: ChipTone
  children: ReactNode
}

/**
 * A small flat status chip — hairline border, 8px radius, Inter 12/500. The
 * label text is always present (never color-only) so the status is accessible.
 */
export function Chip({ tone = 'neutral', className = '', children, ...rest }: ChipProps) {
  return (
    <span
      className={`inline-flex items-center rounded-tag border px-2.5 py-1 font-inter text-[12px] font-medium ${toneClass[tone]} ${className}`}
      {...rest}
    >
      {children}
    </span>
  )
}

export default Chip

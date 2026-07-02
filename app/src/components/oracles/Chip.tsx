import type { HTMLAttributes, ReactNode } from 'react'

/**
 * On-brand chip tones. `ember` is the single reserved punctuation accent
 * (the Delphi rule: ember/saffron only 1–2 moments per viewport) — used for the
 * active "Challenged" status; everything else stays achromatic-warm.
 */
export type ChipTone = 'neutral' | 'ember' | 'confirmed' | 'muted'

const toneClass: Record<ChipTone, string> = {
  // Mid-flight states — quiet sepia/bronze on soft-cream.
  neutral: 'border-pebble bg-soft-cream text-bronze',
  // The single ember punctuation moment — the active challenge.
  ember: 'border-ember-orange/40 bg-ember-orange/10 text-ember-orange',
  // A calm, grounded "confirmed" for resolution.
  confirmed: 'border-pebble bg-soft-cream text-chestnut',
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

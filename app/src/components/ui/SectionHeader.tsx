import type { ReactNode } from 'react'
import { EyebrowTag } from './EyebrowTag'

export interface SectionHeaderProps {
  /** Optional eyebrow label above the headline. */
  eyebrow?: ReactNode
  /** Render the eyebrow inside a hairline pill. */
  eyebrowPill?: boolean
  /** First display line (sepia). */
  line1: ReactNode
  /** Second display line — rendered lighter/italic for two-tone contrast. */
  line2?: ReactNode
  /** One short supporting paragraph, Inter ~17px bronze. */
  paragraph?: ReactNode
  className?: string
}

/**
 * Delphi section headline block. Centered: eyebrow + a two-line Cormorant-300
 * display headline (line 2 lighter + italic for two-tone contrast) at the
 * `heading` scale with negative tracking + one bronze Inter paragraph.
 * Constrained to ~640px so it reads like editorial column copy.
 */
export function SectionHeader({
  eyebrow,
  eyebrowPill = false,
  line1,
  line2,
  paragraph,
  className = '',
}: SectionHeaderProps) {
  return (
    <div className={`mx-auto flex max-w-[640px] flex-col items-center text-center ${className}`}>
      {eyebrow ? (
        <div className="mb-4">
          <EyebrowTag pill={eyebrowPill}>{eyebrow}</EyebrowTag>
        </div>
      ) : null}
      <h2 className="font-serif font-light text-heading text-sepia">
        <span className="block">{line1}</span>
        {line2 ? <span className="block italic text-bronze">{line2}</span> : null}
      </h2>
      {paragraph ? (
        <p className="mt-5 font-inter text-[17px] leading-relaxed text-bronze">{paragraph}</p>
      ) : null}
    </div>
  )
}

export default SectionHeader

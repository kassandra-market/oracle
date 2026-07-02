import type { HTMLAttributes, ReactNode } from 'react'

export interface EyebrowTagProps extends HTMLAttributes<HTMLSpanElement> {
  children: ReactNode
  /** Render inside a hairline pebble pill. Defaults to a bare label. */
  pill?: boolean
}

/**
 * Delphi section eyebrow. Small saffron Inter ~12px uppercase label, centered
 * above a heading. Optionally wrapped in a hairline pebble pill (8px radius).
 */
export function EyebrowTag({ children, pill = false, className = '', ...rest }: EyebrowTagProps) {
  const base =
    'inline-block font-inter font-medium uppercase text-saffron-pulse ' +
    'text-[13px] tracking-[0.08em]'
  const pillClasses = pill ? 'rounded-tag border border-pebble px-3 py-1' : ''
  return (
    <span className={`${base} ${pillClasses} ${className}`} {...rest}>
      {children}
    </span>
  )
}

export default EyebrowTag

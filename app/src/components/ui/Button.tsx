import type { ButtonHTMLAttributes, ReactNode } from 'react'

export type ButtonVariant = 'PrimaryChestnut' | 'GhostOutline' | 'NavPill'

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant
  children: ReactNode
}

/**
 * Delphi buttons. Chestnut is the ONLY fill; the peach bloom is the ONLY
 * button shadow. 12px radius, Inter 15/500. Never a blue/green CTA.
 */
const base =
  'inline-flex items-center justify-center gap-2 rounded-button font-inter ' +
  'text-body font-medium cursor-pointer transition-all duration-150 ' +
  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-offset-2 ' +
  'focus-visible:ring-offset-parchment disabled:opacity-50 disabled:cursor-not-allowed'

const variants: Record<ButtonVariant, string> = {
  // Chestnut fill, white text, ~16px/10px padding, the signature peach bloom.
  PrimaryChestnut:
    'bg-chestnut text-white px-4 py-2.5 shadow-bloom ' +
    'hover:bg-sepia hover:-translate-y-px focus-visible:ring-peach-glow',
  // Transparent, 1px pebble hairline, sepia text. No bg/shadow.
  GhostOutline:
    'bg-transparent text-sepia border border-pebble px-4 py-2.5 ' +
    'hover:border-charcoal-bark focus-visible:ring-pebble',
  // Soft-cream chip, sepia text, generous horizontal padding.
  NavPill:
    'bg-soft-cream text-sepia px-5 py-2.5 ' +
    'hover:bg-pebble/60 focus-visible:ring-pebble',
}

export function Button({ variant = 'PrimaryChestnut', className = '', children, ...rest }: ButtonProps) {
  return (
    <button className={`${base} ${variants[variant]} ${className}`} {...rest}>
      {children}
    </button>
  )
}

export default Button

import type { ButtonHTMLAttributes, ReactNode } from 'react'

export type ButtonVariant = 'PrimaryChestnut' | 'GhostOutline' | 'NavPill'

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant
  children: ReactNode
}

/**
 * Auros buttons. Depth is surface color, not shadow (there are none). The
 * filled action is Liquid Kelp; the signature CTA is the aurora gradient pill.
 * 6px radius, medium weight. Never bold.
 */
const base =
  'inline-flex items-center justify-center gap-2 rounded-button font-inter ' +
  'text-body font-medium cursor-pointer transition-all duration-150 ' +
  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-offset-2 ' +
  'focus-visible:ring-offset-liquid-abyss disabled:opacity-50 disabled:cursor-not-allowed'

const variants: Record<ButtonVariant, string> = {
  // Filled Liquid Kelp action, platinum text. The primary form action.
  PrimaryChestnut:
    'bg-liquid-kelp text-platinum px-4 py-2.5 ' +
    'hover:bg-[#04524c] focus-visible:ring-cyan-phosphor',
  // Transparent, 1px hairline, platinum text. No bg/shadow.
  GhostOutline:
    'bg-transparent text-platinum border border-pebble px-4 py-2.5 ' +
    'hover:border-silver-mist focus-visible:ring-pebble',
  // The signature aurora gradient pill (cyan -> white -> pink), dark text,
  // uppercase tracked. Reserved for the top-level CTA (wallet connect).
  NavPill:
    'bg-aurora text-liquid-abyss px-5 py-2.5 uppercase tracking-[0.08em] text-[14px] ' +
    'hover:brightness-105 focus-visible:ring-cyan-phosphor',
}

export function Button({ variant = 'PrimaryChestnut', className = '', children, ...rest }: ButtonProps) {
  return (
    <button className={`${base} ${variants[variant]} ${className}`} {...rest}>
      {children}
    </button>
  )
}

export default Button

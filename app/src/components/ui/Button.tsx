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
// `transition` (not `transition-all`) animates only Tailwind's curated property
// list — colors, transform, box-shadow, opacity — never literally everything.
// `active:scale-[0.96]` gives a tactile press; disabled buttons don't scale.
const base =
  'inline-flex items-center justify-center gap-2 rounded-button font-inter ' +
  'text-body font-medium cursor-pointer transition duration-150 ' +
  'active:scale-[0.96] disabled:active:scale-100 ' +
  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-offset-2 ' +
  'focus-visible:ring-offset-liquid-abyss disabled:opacity-50 disabled:cursor-not-allowed'

const variants: Record<ButtonVariant, string> = {
  // Filled lifted-teal action with a cyan hairline, platinum text. The primary
  // form action — a rest fill DISTINCT from the kelp card surface it sits on (a
  // flat kelp button on a kelp card is invisible), brightening on hover.
  PrimaryChestnut:
    'bg-[#0c6f64] text-platinum px-4 py-2.5 border border-cyan-phosphor/30 ' +
    'hover:bg-[#128577] hover:border-cyan-phosphor/60 focus-visible:ring-cyan-phosphor ' +
    'hover:-translate-y-0.5 active:translate-y-0',
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

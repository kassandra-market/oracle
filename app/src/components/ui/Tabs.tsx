import { useCallback, useRef, type KeyboardEvent, type ReactNode } from 'react'

export interface TabItem {
  /** Stable id, also the panel's aria linkage. */
  id: string
  /** Visible label. */
  label: ReactNode
  /** Optional count badge (e.g. Facts (3)). */
  count?: number
  /** A quiet accent dot before the label — the single ember/aqua punctuation. */
  dot?: 'aqua' | 'coral' | null
}

export interface TabsProps {
  items: TabItem[]
  value: string
  onChange: (id: string) => void
  /** Accessible name for the tablist (e.g. "Market sections"). */
  ariaLabel: string
  className?: string
}

/**
 * Auros segmented tab bar — a recessed soft-cream track (16px radius, 4px pad)
 * holding raised pure-card pills (concentric 12px radius). The active pill lifts
 * to sepia text over a pure-card surface with a thin chestnut underline; inactive
 * tabs are quiet driftwood that warm to sepia on hover. Colour-only state is
 * avoided (the underline + weight carry the selection too). Flat — no shadow.
 *
 * Roving-tabindex keyboard model: ←/→ move focus+selection, Home/End jump to the
 * ends. `role=tablist`; each button is a `role=tab` linked to its panel via
 * `aria-controls`/`id` (`<Tabs.Panel>` renders the matching `role=tabpanel`).
 */
export function Tabs({ items, value, onChange, ariaLabel, className = '' }: TabsProps) {
  const refs = useRef<(HTMLButtonElement | null)[]>([])

  const onKeyDown = useCallback(
    (e: KeyboardEvent<HTMLDivElement>) => {
      const i = items.findIndex((t) => t.id === value)
      if (i < 0) return
      let next = i
      if (e.key === 'ArrowRight' || e.key === 'ArrowDown') next = (i + 1) % items.length
      else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') next = (i - 1 + items.length) % items.length
      else if (e.key === 'Home') next = 0
      else if (e.key === 'End') next = items.length - 1
      else return
      e.preventDefault()
      onChange(items[next].id)
      refs.current[next]?.focus()
    },
    [items, value, onChange],
  )

  return (
    <div
      role="tablist"
      aria-label={ariaLabel}
      onKeyDown={onKeyDown}
      className={`flex justify-around gap-1 overflow-x-auto rounded-card border border-hairline bg-liquid-deep p-1 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden ${className}`}
    >
      {items.map((t, i) => {
        const selected = t.id === value
        const dotClass = t.dot === 'coral' ? 'bg-coral' : t.dot === 'aqua' ? 'bg-aqua' : ''
        return (
          <button
            key={t.id}
            ref={(el) => {
              refs.current[i] = el
            }}
            role="tab"
            id={`tab-${t.id}`}
            aria-selected={selected}
            aria-controls={`panel-${t.id}`}
            tabIndex={selected ? 0 : -1}
            onClick={() => onChange(t.id)}
            className={[
              'group relative flex shrink-0 items-center gap-2 rounded-[12px] px-3.5 py-2',
              'font-inter text-[13px] whitespace-nowrap transition-[color,background-color]',
              'duration-200 ease-[cubic-bezier(0.2,0,0,1)]',
              'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-aqua/40',
              selected
                ? 'bg-liquid-kelp font-medium text-platinum'
                : 'text-silver hover:text-platinum active:scale-[0.96]',
            ].join(' ')}
          >
            {dotClass ? (
              <span aria-hidden className={`h-1.5 w-1.5 shrink-0 rounded-full ${dotClass}`} />
            ) : null}
            <span>{t.label}</span>
            {t.count != null ? (
              <span
                className={`tabular-nums text-[11px] ${selected ? 'text-aqua' : 'text-silver-dim'}`}
              >
                {t.count}
              </span>
            ) : null}
            {/* Active underline — a second, non-colour selection cue. */}
            <span
              aria-hidden
              className={`pointer-events-none absolute inset-x-3.5 -bottom-px h-0.5 rounded-full bg-aqua transition-opacity duration-200 ${
                selected ? 'opacity-100' : 'opacity-0'
              }`}
            />
          </button>
        )
      })}
    </div>
  )
}

/** The panel matching a tab id — carries the aria linkage + focus target. */
export function TabPanel({
  id,
  active,
  children,
  className = '',
}: {
  id: string
  active: boolean
  children: ReactNode
  className?: string
}) {
  if (!active) return null
  return (
    <div
      role="tabpanel"
      id={`panel-${id}`}
      aria-labelledby={`tab-${id}`}
      tabIndex={0}
      className={`focus-visible:outline-none ${className}`}
    >
      {children}
    </div>
  )
}

export default Tabs

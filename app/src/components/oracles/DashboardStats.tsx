/**
 * The `/oracles` dashboard strip + filter toolbar (RU1). Pure presentation over
 * the already-fetched oracle list — {@link DashboardStats} renders the MONETARY
 * headline + its KASS breakdown (all scaled), and {@link OracleFilters} is the
 * accessible search + phase-filter + sort toolbar (the per-phase COUNTS live on
 * the filter chips). No data fetching here; the page passes decoded data down.
 */
import { formatKass } from '../../lib/oracleView'
import type { OracleStats, PhaseCounts, PhaseFilter, SortBy } from '../../lib/oracleStats'

const focusRing =
  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40 ' +
  'focus-visible:ring-offset-2 focus-visible:ring-offset-parchment'

/** One monetary figure: a scaled-KASS serif value over an Inter label. */
function MoneyTile({ amount, label }: { amount: bigint; label: string }) {
  return (
    <div className="flex flex-col gap-0.5">
      <span className="font-serif text-heading-sm font-light leading-none tabular-nums text-sepia">
        {formatKass(amount)}
      </span>
      <span className="font-inter text-[12px] text-driftwood">{label}</span>
    </div>
  )
}

/**
 * The dashboard stats strip — MONETARY ONLY. The headline "Value at stake" is the
 * scaled-KASS sum contestable on chain (the single lavender accent moment) beside
 * its breakdown (bond pool · dispute bonds · staked). Oracle counts are NOT here —
 * they live on the phase-filter chips below. Read-only, computed client-side.
 */
export function DashboardStats({ stats }: { stats: OracleStats }) {
  return (
    <section
      aria-label="Oracle capital at stake"
      className="mt-10 rounded-card border border-pebble bg-pure-card px-6 py-5"
    >
      <div className="flex flex-col gap-6 sm:flex-row sm:items-center sm:justify-between">
        {/* Headline: total capital at stake — scaled KASS, the lavender accent. */}
        <div className="flex flex-col gap-0.5">
          <span className="font-inter text-[12px] uppercase tracking-wide text-driftwood">
            Value at stake
          </span>
          <span className="font-serif text-heading font-light leading-none tabular-nums text-lavender-phosphor">
            {formatKass(stats.bondsAtRisk)}
          </span>
          <span className="font-inter text-[12px] text-bronze">KASS · across active oracles</span>
        </div>

        {/* The monetary breakdown (all scaled KASS). */}
        <div className="flex flex-wrap gap-x-8 gap-y-3">
          <MoneyTile amount={stats.bondPoolActive} label="Bond pool" />
          <MoneyTile amount={stats.disputeBondsActive} label="Dispute bonds" />
          <MoneyTile amount={stats.stakedActive} label="Staked" />
        </div>
      </div>
    </section>
  )
}

// --- filter + sort toolbar ---------------------------------------------------

/**
 * The phase filter chips, in display order. `dot` is a subtle color hint that
 * mirrors the phase chip tones so the filter reads as the same lifecycle at a
 * glance (color is never the only signal — every chip has its text label + count).
 */
const FILTERS: { value: PhaseFilter; label: string; countKey: keyof PhaseCounts; dot: string }[] = [
  { value: 'all', label: 'All', countKey: 'total', dot: 'bg-driftwood' },
  { value: 'proposal', label: 'Proposal', countKey: 'proposal', dot: 'bg-bronze' },
  { value: 'inDispute', label: 'In dispute', countKey: 'inDispute', dot: 'bg-cyan-phosphor' },
  { value: 'aiClaim', label: 'AI claim', countKey: 'aiClaim', dot: 'bg-lavender-phosphor' },
  { value: 'challenge', label: 'Challenged', countKey: 'challenge', dot: 'bg-ember-orange' },
  { value: 'resolved', label: 'Resolved', countKey: 'resolved', dot: 'bg-chestnut' },
  { value: 'invalidDeadend', label: 'Dead end', countKey: 'invalidDeadend', dot: 'bg-stone' },
]

const SORTS: { value: SortBy; label: string }[] = [
  { value: 'deadline', label: 'Deadline' },
  { value: 'bondsAtRisk', label: 'Bonds at risk' },
]

/** A single toggle button (filter chip or sort option) with `aria-pressed`. */
function ToggleChip({
  active,
  label,
  onClick,
  dot,
  count,
}: {
  active: boolean
  label: string
  onClick: () => void
  /** Optional subtle color-hint dot (a phase's tone) + count badge. */
  dot?: string
  count?: number
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={`inline-flex items-center gap-2 rounded-tag border px-3 py-2 font-inter text-[13px] font-medium transition-colors ${focusRing} ${
        active
          ? 'border-sepia/30 bg-soft-cream text-sepia'
          : 'border-pebble bg-transparent text-bronze hover:border-driftwood hover:text-sepia'
      }`}
    >
      {dot && <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${dot}`} aria-hidden="true" />}
      <span>{label}</span>
      {count !== undefined && (
        <span className="tabular-nums text-[12px] text-driftwood">{count}</span>
      )}
    </button>
  )
}

export interface OracleFiltersProps {
  search: string
  onSearch: (value: string) => void
  filter: PhaseFilter
  onFilter: (value: PhaseFilter) => void
  sort: SortBy
  onSort: (value: SortBy) => void
  /** Per-phase counts — shown on the filter chips (moved off the stats strip). */
  counts: PhaseCounts
  /** Count shown after filtering (for the "showing N" a11y hint). */
  shown: number
}

/**
 * The accessible browse toolbar: a text search, the phase filter chips (real
 * `aria-pressed` toggle buttons carrying each phase's count + a subtle color
 * hint), and a sort control. All three compose — the page applies search →
 * filter → sort in that order.
 */
export function OracleFilters({
  search,
  onSearch,
  filter,
  onFilter,
  sort,
  onSort,
  counts,
  shown,
}: OracleFiltersProps) {
  return (
    <div className="mt-8 flex flex-col gap-4">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <label className="flex items-center gap-2">
          <span className="sr-only">Search oracles</span>
          <input
            type="search"
            value={search}
            onChange={(e) => onSearch(e.target.value)}
            placeholder="Search by phase or address…"
            className={`w-full rounded-button border border-pebble bg-pure-card px-3 py-2 font-inter text-[14px] text-sepia placeholder:text-driftwood sm:w-72 ${focusRing}`}
          />
        </label>

        <div className="flex items-center gap-2" role="group" aria-label="Sort oracles">
          <span className="font-inter text-[12px] text-driftwood">Sort</span>
          {SORTS.map(({ value, label }) => (
            <ToggleChip
              key={value}
              active={sort === value}
              label={label}
              onClick={() => onSort(value)}
            />
          ))}
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-2" role="group" aria-label="Filter by phase">
        {FILTERS.map(({ value, label, countKey, dot }) => (
          <ToggleChip
            key={value}
            active={filter === value}
            label={label}
            dot={dot}
            count={counts[countKey]}
            onClick={() => onFilter(value)}
          />
        ))}
        <span className="ml-auto font-inter text-[12px] text-driftwood" aria-live="polite">
          {shown} shown
        </span>
      </div>
    </div>
  )
}

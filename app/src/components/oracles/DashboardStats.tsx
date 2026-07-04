/**
 * The `/oracles` dashboard strip + filter toolbar (RU1). Pure presentation over
 * the already-fetched oracle list — {@link DashboardStats} renders the derived
 * {@link OracleStats} as a quiet Auros strip (by-phase count tiles, the
 * bonds-at-risk headline — the ONE ember punctuation moment, the resolved
 * count); {@link OracleFilters} is the accessible search + phase-filter + sort
 * toolbar. No data fetching here; the page passes decoded data down.
 */
import { groupDigits } from '../../lib/oracleView'
import type { OracleStats, PhaseFilter, SortBy } from '../../lib/oracleStats'

const focusRing =
  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40 ' +
  'focus-visible:ring-offset-2 focus-visible:ring-offset-parchment'

/** Ordered by-phase tiles for the stats strip. */
const PHASE_TILES: { key: keyof OracleStats['counts']; label: string }[] = [
  { key: 'proposal', label: 'Proposal' },
  { key: 'inDispute', label: 'In dispute' },
  { key: 'aiClaim', label: 'AI claim' },
  { key: 'challenge', label: 'Challenged' },
  { key: 'resolved', label: 'Resolved' },
  { key: 'invalidDeadend', label: 'Dead end' },
]

/** One small count tile: a serif figure over an Inter label. */
function CountTile({ count, label }: { count: number; label: string }) {
  return (
    <div className="flex flex-col gap-0.5">
      <span className="font-serif text-heading-sm font-light leading-none text-sepia">
        {count}
      </span>
      <span className="font-inter text-[12px] text-driftwood">{label}</span>
    </div>
  )
}

/**
 * The dashboard stats strip: the bonds-at-risk headline (raw base units,
 * `groupDigits`, the single ember accent) beside the by-phase count tiles and
 * the total. Read-only, computed client-side from the fetched list.
 */
export function DashboardStats({ stats }: { stats: OracleStats }) {
  return (
    <section
      aria-label="Oracle dashboard stats"
      className="mt-10 rounded-card border border-pebble bg-pure-card px-6 py-5"
    >
      <div className="flex flex-col gap-6 sm:flex-row sm:items-center sm:justify-between">
        {/* Headline: capital at stake — the ONE ember punctuation moment. */}
        <div className="flex flex-col gap-0.5">
          <span className="font-inter text-[12px] uppercase tracking-wide text-driftwood">
            Bonds &amp; stake at risk
          </span>
          <span className="font-serif text-heading font-light leading-none text-lavender-phosphor">
            {groupDigits(stats.bondsAtRisk)}
          </span>
          <span className="font-inter text-[12px] text-bronze">
            raw base units (unscaled) · across{' '}
            {stats.counts.total - stats.resolvedCount - stats.counts.invalidDeadend} active
          </span>
        </div>

        {/* By-phase count tiles. */}
        <div className="flex flex-wrap gap-x-6 gap-y-3">
          {PHASE_TILES.map(({ key, label }) => (
            <CountTile key={key} count={stats.counts[key]} label={label} />
          ))}
          <CountTile count={stats.counts.total} label="Total" />
        </div>
      </div>
    </section>
  )
}

// --- filter + sort toolbar ---------------------------------------------------

/** The phase filter chips, in display order. */
const FILTERS: { value: PhaseFilter; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'proposal', label: 'Proposal' },
  { value: 'inDispute', label: 'In dispute' },
  { value: 'aiClaim', label: 'AI claim' },
  { value: 'challenge', label: 'Challenged' },
  { value: 'resolved', label: 'Resolved' },
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
}: {
  active: boolean
  label: string
  onClick: () => void
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={`rounded-tag border px-3 py-1.5 font-inter text-[13px] font-medium transition-colors ${focusRing} ${
        active
          ? 'border-sepia/30 bg-soft-cream text-sepia'
          : 'border-pebble bg-transparent text-bronze hover:border-driftwood hover:text-sepia'
      }`}
    >
      {label}
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
  /** Count shown after filtering (for the "showing N" a11y hint). */
  shown: number
}

/**
 * The accessible browse toolbar: a text search, the phase filter chips (real
 * `aria-pressed` toggle buttons, keyboard-reachable), and a sort control. All
 * three compose — the page applies search → filter → sort in that order.
 */
export function OracleFilters({
  search,
  onSearch,
  filter,
  onFilter,
  sort,
  onSort,
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
        {FILTERS.map(({ value, label }) => (
          <ToggleChip
            key={value}
            active={filter === value}
            label={label}
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

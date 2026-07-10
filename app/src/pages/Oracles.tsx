import { useMemo, useState } from 'react'
import { Link, useLocation } from 'react-router-dom'
import { Button, Card, SectionHeader } from '../components/ui'
import { PhaseChip } from '../components/oracles/PhaseChip'
import { DashboardStats, OracleFilters } from '../components/oracles/DashboardStats'
import { useOracles } from '../hooks/useOracles'
import { useOracleMeta, type OracleMetaView } from '../hooks/useOracleMeta'
import type { OracleSummary } from '../data/oracles'
import { CLUSTER_LABELS, useCluster } from '../lib/cluster'
import { RESOLVED_OPTION_NONE, phaseView, relativeDeadline } from '../lib/oracleView'
import {
  deriveStats,
  filterByPhaseGroup,
  sortOracles,
  type PhaseFilter,
  type SortBy,
} from '../lib/oracleStats'
import { Phase } from '@kassandra-market/oracles'

const focusRing =
  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40 ' +
  'focus-visible:ring-offset-2 focus-visible:ring-offset-parchment'

/** One oracle rendered as a clickable Auros card. */
function OracleCard({
  summary,
  search,
  meta,
}: {
  summary: OracleSummary
  search: string
  meta?: OracleMetaView
}) {
  const { pubkey, oracle } = summary
  const { label } = phaseView(oracle.phase)
  const resolved = oracle.phase === Phase.Resolved
  const hasResolvedOption = resolved && oracle.resolvedOption !== RESOLVED_OPTION_NONE
  const options = meta?.options ?? []
  const SHOWN = 6

  return (
    <Link
      to={{ pathname: `/oracles/${pubkey}`, search }}
      className={`group block rounded-card ${focusRing}`}
    >
      <Card className="flex h-full flex-col gap-3 transition-colors group-hover:border-driftwood">
        <div className="flex items-center justify-between gap-2">
          <PhaseChip phase={oracle.phase} />
          <span className="font-inter text-[12px] text-driftwood">
            {relativeDeadline(oracle.deadline)}
          </span>
        </div>

        {/* Subject (the question) near the top — the on-chain plaintext when the
            metadata has loaded, else the phase label. */}
        <h3 className="font-serif text-subheading font-light text-sepia">
          {meta?.subject ?? label}
        </h3>
        {options.length > 0 && (
          <div className="flex flex-wrap gap-1.5">
            {options.slice(0, SHOWN).map((opt, i) => (
              <span
                key={i}
                className="rounded-tag border border-pebble bg-soft-cream px-2 py-0.5 font-inter text-[12px] text-bronze"
              >
                {opt}
              </span>
            ))}
            {options.length > SHOWN && (
              <span className="rounded-tag px-2 py-0.5 font-inter text-[12px] text-driftwood">
                +{options.length - SHOWN}
              </span>
            )}
          </div>
        )}

        <dl className="mt-auto flex flex-wrap gap-x-5 gap-y-1 font-inter text-[13px] text-bronze">
          <div className="flex gap-1">
            <dt className="text-driftwood">Proposers</dt>
            <dd className="font-medium text-sepia">{oracle.proposerCount}</dd>
          </div>
          <div className="flex gap-1">
            <dt className="text-driftwood">Facts</dt>
            <dd className="font-medium text-sepia">{oracle.factCount}</dd>
          </div>
          <div className="flex gap-1">
            <dt className="text-driftwood">Options</dt>
            <dd className="font-medium text-sepia">{oracle.optionsCount}</dd>
          </div>
        </dl>

        {resolved ? (
          <p className="font-inter text-[13px] text-chestnut">
            {hasResolvedOption
              ? `Resolved · option ${oracle.resolvedOption}`
              : 'Resolved · no valid option'}
          </p>
        ) : null}
      </Card>
    </Link>
  )
}

/** A single skeleton placeholder card (loading state). */
function SkeletonCard() {
  return (
    <Card className="flex h-full animate-pulse flex-col gap-3" aria-hidden="true">
      <div className="flex items-center justify-between">
        <div className="h-6 w-20 rounded-tag bg-soft-cream" />
        <div className="h-4 w-16 rounded-sm bg-soft-cream" />
      </div>
      <div className="h-6 w-32 rounded-sm bg-soft-cream" />
      <div className="h-4 w-24 rounded-sm bg-soft-cream" />
      <div className="mt-2 h-4 w-full rounded-sm bg-soft-cream" />
    </Card>
  )
}

/** A quiet skeleton for the stats strip (loading state). */
function SkeletonStats() {
  return (
    <div
      className="mt-10 animate-pulse rounded-card border border-pebble bg-pure-card px-6 py-5"
      aria-hidden="true"
    >
      <div className="flex flex-col gap-6 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex flex-col gap-2">
          <div className="h-3 w-32 rounded-sm bg-soft-cream" />
          <div className="h-8 w-40 rounded-sm bg-soft-cream" />
        </div>
        <div className="flex flex-wrap gap-6">
          {Array.from({ length: 6 }, (_, i) => (
            <div key={i} className="flex flex-col gap-1">
              <div className="h-6 w-8 rounded-sm bg-soft-cream" />
              <div className="h-3 w-16 rounded-sm bg-soft-cream" />
            </div>
          ))}
        </div>
      </div>
    </div>
  )
}

/** Case-insensitive text match against an oracle's phase label + address. */
function matchesQuery(summary: OracleSummary, query: string): boolean {
  const q = query.trim().toLowerCase()
  if (q === '') return true
  const haystack = `${phaseView(summary.oracle.phase).label} ${summary.pubkey}`.toLowerCase()
  return haystack.includes(q)
}

const gridClass = 'mt-8 grid grid-cols-1 gap-6 sm:grid-cols-2 lg:grid-cols-3'

/**
 * The oracle browser at `/oracles` — a `SectionHeader` intro over a responsive
 * grid of Auros cards, one per decoded oracle. Read-only: consumes the FA2
 * data layer via `useOracles` (over FA1's connection). Loading / error / empty.
 */
export default function Oracles() {
  const { cluster } = useCluster()
  const { search } = useLocation()
  const { data, loading, error, refetch } = useOracles()

  // Client-side view state — search + phase filter + sort, all composed over the
  // already-fetched list (no re-fetch). `search` (URL query) is unrelated: it
  // preserves `?mock` through the card links.
  const [query, setQuery] = useState('')
  const [filter, setFilter] = useState<PhaseFilter>('all')
  const [sort, setSort] = useState<SortBy>('deadline')

  const stats = useMemo(() => deriveStats(data ?? []), [data])
  const visible = useMemo(() => {
    if (!data) return []
    const searched = data.filter((s) => matchesQuery(s, query))
    return sortOracles(filterByPhaseGroup(searched, filter), sort)
  }, [data, query, filter, sort])

  // Verified plaintext subject + option labels (from the indexer), fetched once
  // for the whole loaded set so filtering/sorting doesn't refetch.
  const metaItems = useMemo(() => (data ?? []).map((s) => s.pubkey), [data])
  const meta = useOracleMeta(metaItems)

  return (
    <main className="mx-auto max-w-[1200px] px-6 py-16 md:py-20">
      <SectionHeader
        as="h1"
        eyebrow="Oracles"
        eyebrowPill
        line1="Every dispute,"
        line2="proposed, contested, settled"
        paragraph="Browse the optimistic-oracle disputes this program has opened on chain — their phase, their deadline, and the proposers and facts staked against each answer."
      />

      <div className="mt-8 flex justify-center">
        <Link
          to={{ pathname: '/oracles/new', search }}
          className="inline-flex items-center justify-center gap-2 rounded-button bg-chestnut px-4 py-2.5 font-inter text-body font-medium text-liquid-abyss transition-all duration-150 hover:-translate-y-px hover:brightness-110 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-phosphor focus-visible:ring-offset-2 focus-visible:ring-offset-liquid-abyss"
        >
          Create oracle
        </Link>
      </div>

      {loading ? (
        <>
          <SkeletonStats />
          <p className="mt-8 text-center font-inter text-[15px] text-bronze" role="status">
            Reading the chain…
          </p>
          <div className={gridClass} aria-hidden="true">
            {Array.from({ length: 6 }, (_, i) => (
              <SkeletonCard key={i} />
            ))}
          </div>
        </>
      ) : error ? (
        <div className="mx-auto mt-12 max-w-[560px]">
          <Card>
            <h2 className="font-serif text-heading-sm font-light text-sepia">
              Couldn’t load oracles
            </h2>
            <p className="mt-2 font-inter text-[15px] text-bronze">{error.message}</p>
            <div className="mt-5">
              <Button variant="GhostOutline" onClick={refetch}>
                Retry
              </Button>
            </div>
          </Card>
        </div>
      ) : !data || data.length === 0 ? (
        <div className="mx-auto mt-12 max-w-[560px] text-center">
          <Card>
            <p className="font-inter text-[15px] text-bronze">
              No oracles found on{' '}
              <span className="font-medium text-sepia">{CLUSTER_LABELS[cluster]}</span>.
            </p>
            <p className="mt-2 font-inter text-[13px] text-driftwood">
              Switch cluster in the top bar, or point the app at a seeded validator.
            </p>
          </Card>
        </div>
      ) : (
        <>
          <DashboardStats stats={stats} />
          <OracleFilters
            search={query}
            onSearch={setQuery}
            filter={filter}
            onFilter={setFilter}
            sort={sort}
            onSort={setSort}
            counts={stats.counts}
            shown={visible.length}
          />
          {visible.length === 0 ? (
            <div className="mx-auto mt-12 max-w-[560px] text-center">
              <Card>
                <p className="font-inter text-[15px] text-bronze">
                  No oracles match the current search and filters.
                </p>
              </Card>
            </div>
          ) : (
            <div className={gridClass}>
              {visible.map((summary) => (
                <OracleCard
                  key={summary.pubkey}
                  summary={summary}
                  search={search}
                  meta={meta.get(summary.pubkey)}
                />
              ))}
            </div>
          )}
        </>
      )}
    </main>
  )
}

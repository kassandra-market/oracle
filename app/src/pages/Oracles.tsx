import { Link, useLocation } from 'react-router-dom'
import { Button, Card, SectionHeader } from '../components/ui'
import { PhaseChip } from '../components/oracles/PhaseChip'
import { useOracles } from '../hooks/useOracles'
import type { OracleSummary } from '../data/oracles'
import { CLUSTER_LABELS, useCluster } from '../lib/cluster'
import {
  RESOLVED_OPTION_NONE,
  hashPreview,
  phaseView,
  relativeDeadline,
} from '../lib/oracleView'
import { Phase } from '@kassandra/sdk'

const focusRing =
  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40 ' +
  'focus-visible:ring-offset-2 focus-visible:ring-offset-parchment'

/** One oracle rendered as a clickable Delphi card. */
function OracleCard({ summary, search }: { summary: OracleSummary; search: string }) {
  const { pubkey, oracle } = summary
  const { label } = phaseView(oracle.phase)
  const resolved = oracle.phase === Phase.Resolved
  const hasResolvedOption = resolved && oracle.resolvedOption !== RESOLVED_OPTION_NONE

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

        <h3 className="font-serif text-subheading font-light text-sepia">{label}</h3>
        <p className="font-mono text-[12px] text-bronze" title="Prompt hash">
          {hashPreview(oracle.promptHash)}
        </p>

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

const gridClass = 'mt-12 grid grid-cols-1 gap-6 sm:grid-cols-2 lg:grid-cols-3'

/**
 * The oracle browser at `/oracles` — a `SectionHeader` intro over a responsive
 * grid of Delphi cards, one per decoded oracle. Read-only: consumes the FA2
 * data layer via `useOracles` (over FA1's connection). Loading / error / empty.
 */
export default function Oracles() {
  const { cluster } = useCluster()
  const { search } = useLocation()
  const { data, loading, error, refetch } = useOracles()

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

      {loading ? (
        <>
          <p className="mt-12 text-center font-inter text-[15px] text-bronze" role="status">
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
        <div className={gridClass}>
          {data.map((summary) => (
            <OracleCard key={summary.pubkey} summary={summary} search={search} />
          ))}
        </div>
      )}
    </main>
  )
}

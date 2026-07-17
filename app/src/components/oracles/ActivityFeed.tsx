import { useCluster } from '../../lib/cluster'
import { explorerTxUrl, shortSig } from '../../lib/explorer'
import { relativeDeadline } from '../../lib/oracleView'
import { isIndexerConfigured, type IndexedEvent } from '../../data/indexer'
import { useAccountEvents } from '../../hooks/useIndexer'
import { Card } from '../ui'
import { Chip } from './Chip'

/** Human label for an instruction type (e.g. `submit_fact` → "Submit fact"). */
function ixLabel(ixType: string): string {
  if (ixType === 'unknown') return 'Unknown instruction'
  const s = ixType.replace(/_/g, ' ')
  return s.charAt(0).toUpperCase() + s.slice(1)
}

/** A failed tx shows a muted "reverted" chip; ordinary events stay quiet. */
function EventRow({ event }: { event: IndexedEvent }) {
  const { cluster } = useCluster()
  const url = explorerTxUrl(cluster, event.signature)
  const when = event.blockTime != null ? relativeDeadline(BigInt(event.blockTime)) : `slot ${event.slot}`

  return (
    <li className="flex items-baseline justify-between gap-3 border-b border-pebble py-2.5 last:border-0">
      <div className="flex items-baseline gap-2">
        <span className="font-inter text-[14px] text-sepia">{ixLabel(event.ixType)}</span>
        <Chip tone="muted">{when}</Chip>
      </div>
      <span className="font-mono text-[12px] text-driftwood">
        {url ? (
          <a
            href={url}
            target="_blank"
            rel="noreferrer noopener"
            className="underline decoration-pebble underline-offset-4 hover:text-lavender-phosphor focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40"
          >
            {shortSig(event.signature)}
          </a>
        ) : (
          shortSig(event.signature)
        )}
      </span>
    </li>
  )
}

/**
 * On-chain activity for one oracle, read from the indexer backend. Every
 * program instruction touching this oracle PDA (propose, submit_fact, votes,
 * finalize cranks, claims, challenge ops…) in reverse-chronological order.
 *
 * Renders nothing when the indexer is not configured (`VITE_INDEXER_URL` unset),
 * so it is an additive enhancement over the direct-chain reads.
 */
export function ActivityFeed({ oracle }: { oracle: string }) {
  const { data, loading, error, refetch } = useAccountEvents(oracle, 50)

  if (!isIndexerConfigured()) return null

  return (
    <Card className="flex flex-col gap-3" data-testid="activity-feed">
      <div className="flex items-center justify-between">
        <h3 className="font-serif text-subheading font-light text-sepia">On-chain activity</h3>
        {data && data.length > 0 ? (
          <span className="font-inter text-[13px] text-driftwood">{data.length} events</span>
        ) : null}
      </div>

      {loading ? (
        <p className="font-inter text-[13px] text-bronze">Reading the index…</p>
      ) : error ? (
        <div className="flex items-center gap-3">
          <p className="font-inter text-[13px] text-ember-orange">
            Couldn&apos;t reach the indexer.
          </p>
          <button
            type="button"
            onClick={refetch}
            className="font-inter text-[13px] text-sepia underline decoration-pebble underline-offset-4 hover:text-lavender-phosphor"
          >
            Retry
          </button>
        </div>
      ) : !data || data.length === 0 ? (
        <p className="font-inter text-[13px] text-driftwood">
          No indexed activity yet for this oracle.
        </p>
      ) : (
        <ul className="flex flex-col">
          {data.map((e) => (
            <EventRow key={`${e.signature}:${e.ixIndex}`} event={e} />
          ))}
        </ul>
      )}
    </Card>
  )
}

export default ActivityFeed

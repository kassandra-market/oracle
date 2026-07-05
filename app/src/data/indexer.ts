/**
 * Read client for the Kassandra INDEXER backend (`indexer/`, a Carbon service).
 *
 * The indexer crawls the program's transactions into Postgres and serves a
 * read-only JSON API; the app reads its per-account event history to render an
 * on-chain activity feed. It is OPTIONAL — when `VITE_INDEXER_URL` is unset the
 * feature is simply absent (the rest of the app reads chain directly).
 */

/** The indexer base URL, or `undefined` when the feature is not configured. */
export function indexerBaseUrl(): string | undefined {
  const raw = (import.meta.env.VITE_INDEXER_URL as string | undefined)?.trim()
  if (!raw) return undefined
  return raw.replace(/\/+$/, '') // strip any trailing slash
}

/** Whether the indexer-backed features should render. */
export function isIndexerConfigured(): boolean {
  return indexerBaseUrl() !== undefined
}

/** One indexed event (a single Kassandra instruction), as the API returns it. */
export interface IndexedEvent {
  signature: string
  ixIndex: number
  ixType: string
  discriminant: number
  slot: number
  blockTime: number | null
  account0: string | null
  accounts: string[]
  dataBase64: string
}

/** Indexer status: how far it has caught up. */
export interface IndexerStatus {
  programId: string
  eventCount: number
  cursor: { signature: string; slot: number } | null
}

class IndexerError extends Error {}

async function getJson<T>(path: string, signal?: AbortSignal): Promise<T> {
  const base = indexerBaseUrl()
  if (!base) throw new IndexerError('indexer not configured (set VITE_INDEXER_URL)')
  const res = await fetch(`${base}${path}`, { signal, headers: { accept: 'application/json' } })
  if (!res.ok) throw new IndexerError(`indexer ${path} → ${res.status}`)
  return (await res.json()) as T
}

/** The event history touching an account (e.g. an oracle PDA), newest first. */
export async function fetchAccountEvents(
  account: string,
  opts: { limit?: number; beforeSlot?: number; signal?: AbortSignal } = {},
): Promise<IndexedEvent[]> {
  const params = new URLSearchParams({ limit: String(opts.limit ?? 50) })
  if (opts.beforeSlot !== undefined) params.set('beforeSlot', String(opts.beforeSlot))
  const body = await getJson<{ events: IndexedEvent[] }>(
    `/accounts/${account}/events?${params.toString()}`,
    opts.signal,
  )
  return body.events
}

/** Recent events across the program, optionally filtered by instruction type. */
export async function fetchEvents(
  opts: { type?: string; limit?: number; signal?: AbortSignal } = {},
): Promise<IndexedEvent[]> {
  const params = new URLSearchParams({ limit: String(opts.limit ?? 50) })
  if (opts.type) params.set('type', opts.type)
  const body = await getJson<{ events: IndexedEvent[] }>(`/events?${params.toString()}`, opts.signal)
  return body.events
}

/** The indexer's catch-up status (event count + cursor). */
export async function fetchIndexerStatus(signal?: AbortSignal): Promise<IndexerStatus> {
  return getJson<IndexerStatus>('/status', signal)
}

/**
 * Off-chain oracle metadata captured from the CreateOracle memo: the plaintext
 * SUBJECT (== the hashed question) + the option LABELS. The chain stores only a
 * prompt hash + options_count, so this is how the browse/detail views show the
 * real question + options. The client verifies `subject` against the on-chain
 * `prompt_hash` (see `verifyOracleSubject`); `options` are advisory (not hashed).
 */
export interface OracleMeta {
  oracle: string
  subject: string
  options: string[]
  slot: number
}

/**
 * Fetch metadata for a batch of oracle PDAs. Best-effort: returns an empty map
 * when the indexer is not configured or the request fails, so the browse view
 * degrades gracefully to the prompt-hash display.
 */
export async function fetchOracleMeta(
  pubkeys: string[],
  signal?: AbortSignal,
): Promise<Map<string, OracleMeta>> {
  if (!indexerBaseUrl() || pubkeys.length === 0) return new Map()
  try {
    const params = new URLSearchParams({ accounts: pubkeys.join(',') })
    const body = await getJson<{ meta: OracleMeta[] }>(`/oracles/meta?${params.toString()}`, signal)
    return new Map(body.meta.map((m) => [m.oracle, m]))
  } catch {
    return new Map()
  }
}

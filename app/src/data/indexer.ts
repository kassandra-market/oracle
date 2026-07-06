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
 * Oracle metadata indexed from the on-chain `oracle_meta` account: the plaintext
 * SUBJECT + option LABELS (both on-chain, authoritative) plus the extended-JSON
 * `uri` and its `uriHash` (hex sha256). The browse/detail views read this mirror;
 * the detail view fetches the `uri` JSON and verifies it against `uriHash`.
 */
export interface OracleMeta {
  oracle: string
  subject: string
  options: string[]
  uri: string
  uriHash: string
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

/** Decode base64 (the indexer serves raw Pod account bytes as base64) to bytes. */
function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64)
  const out = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i += 1) out[i] = bin.charCodeAt(i)
  return out
}

/** One indexed account: its pubkey + raw Pod bytes (decoded by the caller with the SDK). */
export interface IndexedAccount {
  pubkey: string
  data: Uint8Array
}

/** A detail-view account, tagged by its on-chain `account_type`. */
export interface IndexedChildAccount extends IndexedAccount {
  accountType: number
}

/**
 * Every indexed Oracle account (raw bytes) from the indexer's account mirror
 * (`oracle_accounts`, kept fresh by gpa snapshot + programSubscribe). Returns
 * `null` when the indexer isn't configured or the request fails, so the caller
 * falls back to a direct `getProgramAccounts`.
 */
export async function fetchOracleAccounts(signal?: AbortSignal): Promise<IndexedAccount[] | null> {
  if (!indexerBaseUrl()) return null
  try {
    const body = await getJson<{ accounts: { pubkey: string; data: string }[] }>(
      '/oracles/accounts',
      signal,
    )
    return body.accounts.map((a) => ({ pubkey: a.pubkey, data: base64ToBytes(a.data) }))
  } catch {
    return null
  }
}

/**
 * The oracle account + its children (Proposer/Fact/FactVote/AiClaim/Market), tagged
 * by `accountType`, from the indexer's account mirror. `null` → caller falls back
 * to `getProgramAccounts`.
 */
export async function fetchOracleDetailAccounts(
  oracle: string,
  signal?: AbortSignal,
): Promise<IndexedChildAccount[] | null> {
  if (!indexerBaseUrl()) return null
  try {
    const body = await getJson<{ accounts: { pubkey: string; accountType: number; data: string }[] }>(
      `/oracles/${oracle}/accounts`,
      signal,
    )
    return body.accounts.map((a) => ({
      pubkey: a.pubkey,
      accountType: a.accountType,
      data: base64ToBytes(a.data),
    }))
  } catch {
    return null
  }
}

/**
 * POST the extended metadata JSON to the app's OWN metadata host (a relative URL;
 * the app server proxies it to the private indexer). Best-effort — errors are
 * swallowed: the JSON is only ever served once its sha256 matches the on-chain
 * `uri_hash`, so a failed or late POST is harmless.
 */
export async function postOracleMetadata(oracle: string, jsonString: string): Promise<void> {
  try {
    await fetch(`/api/oracle/${oracle}/metadata.json`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: jsonString,
    })
  } catch {
    // Swallow — hosting the JSON is a convenience, not required for correctness.
  }
}

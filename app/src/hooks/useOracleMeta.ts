/**
 * Fetch oracle metadata (the plaintext SUBJECT + option LABELS, plus the extended
 * JSON `uri`/`uriHash`) indexed from the on-chain `oracle_meta` account. Unlike
 * the old memo path, the subject + labels are now ON CHAIN (authoritative) — the
 * indexer is just a queryable mirror, so no client-side re-hash is needed. The
 * extended JSON (behind `uri`) is fetched separately by the detail view and
 * verified against `uriHash`. Best-effort: no indexer / a fetch failure yields an
 * empty map.
 */
import { useEffect, useState } from 'react'

import { fetchOracleMeta } from '../data/indexer'

/** An indexed metadata view for one oracle. */
export interface OracleMetaView {
  /** The plaintext question (on-chain, authoritative). */
  subject?: string
  /** The option labels (on-chain, authoritative). */
  options?: string[]
  /** URL of the extended off-chain metadata JSON (may be empty). */
  uri?: string
  /** Hex sha256 binding the extended JSON. */
  uriHash?: string
}

/**
 * Load metadata for a set of oracle PDAs. Refetches only when the pubkey set
 * changes (not on every render).
 */
export function useOracleMeta(pubkeys: string[]): Map<string, OracleMetaView> {
  const [map, setMap] = useState<Map<string, OracleMetaView>>(new Map())
  const key = pubkeys.join(',')

  useEffect(() => {
    if (pubkeys.length === 0) {
      setMap(new Map())
      return
    }
    const ac = new AbortController()
    void (async () => {
      const raw = await fetchOracleMeta(pubkeys, ac.signal)
      if (ac.signal.aborted) return
      const out = new Map<string, OracleMetaView>()
      for (const pubkey of pubkeys) {
        const m = raw.get(pubkey)
        if (!m) continue
        out.set(pubkey, {
          subject: m.subject,
          options: Array.isArray(m.options) ? m.options : undefined,
          uri: m.uri,
          uriHash: m.uriHash,
        })
      }
      if (!ac.signal.aborted) setMap(out)
    })()
    return () => ac.abort()
    // Refetch keyed on the pubkey SET; `pubkeys` identity changes every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key])

  return map
}

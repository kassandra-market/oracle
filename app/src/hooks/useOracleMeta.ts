/**
 * Fetch + verify off-chain oracle metadata (the plaintext SUBJECT + option
 * LABELS captured from the CreateOracle memo, served by the indexer). The
 * `subject` is trusted ONLY when its SHA-256 matches the on-chain `prompt_hash`
 * (the same hash the chain commits to), so a lying indexer can't fake a question.
 * Option labels are advisory (not part of the hash). Best-effort: no indexer, or
 * a fetch failure, just yields an empty map and the UI falls back to the hash.
 */
import { useEffect, useState } from 'react'

import { fetchOracleMeta } from '../data/indexer'

/** A verified metadata view for one oracle. */
export interface OracleMetaView {
  /** The plaintext question — present ONLY when it hash-matches `prompt_hash`. */
  subject?: string
  /** The option labels (advisory; not hashed). */
  options?: string[]
}

async function sha256(s: string): Promise<Uint8Array> {
  return new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(s)))
}

function bytesEqual(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false
  for (let i = 0; i < a.length; i += 1) if (a[i] !== b[i]) return false
  return true
}

/**
 * Load verified metadata for a set of oracles. Refetches only when the pubkey set
 * changes (not on every render). Each entry's `subject` is included only if it
 * verifies against that oracle's `promptHash`.
 */
export function useOracleMeta(
  items: { pubkey: string; promptHash: Uint8Array }[],
): Map<string, OracleMetaView> {
  const [map, setMap] = useState<Map<string, OracleMetaView>>(new Map())
  const key = items.map((i) => i.pubkey).join(',')

  useEffect(() => {
    if (items.length === 0) {
      setMap(new Map())
      return
    }
    const ac = new AbortController()
    void (async () => {
      const raw = await fetchOracleMeta(
        items.map((i) => i.pubkey),
        ac.signal,
      )
      if (ac.signal.aborted) return
      const out = new Map<string, OracleMetaView>()
      for (const it of items) {
        const m = raw.get(it.pubkey)
        if (!m) continue
        const view: OracleMetaView = {
          options: Array.isArray(m.options) ? m.options : undefined,
        }
        try {
          if (bytesEqual(await sha256(m.subject), it.promptHash)) view.subject = m.subject
        } catch {
          // Leave the subject unverified (fall back to the hash in the UI).
        }
        out.set(it.pubkey, view)
      }
      if (!ac.signal.aborted) setMap(out)
    })()
    return () => ac.abort()
    // Refetch keyed on the pubkey SET; `items` identity changes every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key])

  return map
}

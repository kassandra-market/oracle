/**
 * Minimal query hooks over the FA2 read data layer. TanStack Query is NOT a
 * dependency (and shouldn't be added for read-only slice 2) — this is a tiny
 * `useEffect`+state async wrapper with loading/error/data, an unmount guard to
 * avoid setState-after-unmount, and a `refetch`. The underlying `Connection`
 * comes from FA1's `useConnection()`, so switching cluster (a new endpoint →
 * new memoized `Connection`) re-runs the fetch automatically.
 *
 * Mock affordance: when {@link isMockMode} is on (`VITE_MOCK=1` or `?mock`), the
 * hooks resolve fixtures WITHOUT touching the connection, keeping the real data
 * path (`fetchOracles`/`fetchOracleDetail`) untouched.
 */
import { useCallback, useEffect, useRef, useState } from 'react'
import { useConnection } from '../lib/cluster'
import { fetchOracleDetail, fetchOracles } from '../data/oracles'
import type { OracleDetail, OracleSummary } from '../data/oracles'
import { isMockMode, mockOracleDetail, mockOracles } from '../data/mockOracles'

export interface AsyncState<T> {
  data: T | undefined
  loading: boolean
  error: Error | undefined
  /** Re-run the fetch (e.g. from an error-state retry button). */
  refetch: () => void
}

/**
 * Run `task` on mount + whenever `deps` change, tracking loading/error/data.
 * `task` receives an `isCurrent()` so late resolutions from a superseded run
 * (cluster switched, component unmounted) are dropped instead of clobbering
 * state. `refetch` bumps an internal nonce to force a re-run.
 */
function useAsync<T>(task: () => Promise<T>, deps: readonly unknown[]): AsyncState<T> {
  const [data, setData] = useState<T | undefined>(undefined)
  const [error, setError] = useState<Error | undefined>(undefined)
  const [loading, setLoading] = useState(true)
  const [nonce, setNonce] = useState(0)
  const taskRef = useRef(task)
  taskRef.current = task

  const refetch = useCallback(() => setNonce((n) => n + 1), [])

  useEffect(() => {
    let active = true
    setLoading(true)
    setError(undefined)
    taskRef.current().then(
      (result) => {
        if (!active) return
        setData(result)
        setLoading(false)
      },
      (err: unknown) => {
        if (!active) return
        setError(err instanceof Error ? err : new Error(String(err)))
        setData(undefined)
        setLoading(false)
      },
    )
    return () => {
      active = false
    }
    // taskRef is a stable ref; deps + nonce drive re-runs intentionally.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, nonce])

  return { data, loading, error, refetch }
}

/** The oracle list: every decoded {@link OracleSummary} (or the mock set). */
export function useOracles(): AsyncState<OracleSummary[]> {
  const { connection } = useConnection()
  return useAsync(
    () => (isMockMode() ? Promise.resolve(mockOracles()) : fetchOracles(connection)),
    [connection],
  )
}

/** One oracle plus its children; rejects with `OracleNotFoundError` when absent. */
export function useOracleDetail(pubkey: string | undefined): AsyncState<OracleDetail> {
  const { connection } = useConnection()
  return useAsync(
    () =>
      isMockMode()
        ? mockOracleDetail(pubkey ?? '')
        : fetchOracleDetail(connection, pubkey ?? ''),
    [connection, pubkey],
  )
}

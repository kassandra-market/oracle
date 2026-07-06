/**
 * The read-layer async primitive: a tiny `useEffect`+state wrapper with
 * loading/error/data, an unmount guard (drops late resolutions from a superseded
 * run), and a `refetch` (bumps an internal nonce). TanStack Query is deliberately
 * NOT a dependency for the read-only slice — this is all the query hooks need.
 *
 * The data source is the {@link IndexerClient} (from `useIndexer`), so a caller
 * that lists `indexer` in `deps` re-runs the fetch if that client ever changes.
 */
import { useCallback, useEffect, useRef, useState } from "react";

export interface AsyncState<T> {
  data: T | undefined;
  loading: boolean;
  error: Error | undefined;
  /** Re-run the fetch (e.g. from an error-state retry button, or after a write). */
  refetch: () => void;
}

/**
 * Run `task` on mount + whenever `deps` change, tracking loading/error/data. An
 * `active` flag drops late resolutions from a superseded run (deps changed,
 * component unmounted) instead of clobbering state. `refetch` bumps an internal
 * nonce to force a re-run.
 */
export function useAsync<T>(task: () => Promise<T>, deps: readonly unknown[]): AsyncState<T> {
  const [data, setData] = useState<T | undefined>(undefined);
  const [error, setError] = useState<Error | undefined>(undefined);
  const [loading, setLoading] = useState(true);
  const [nonce, setNonce] = useState(0);
  const taskRef = useRef(task);
  taskRef.current = task;

  const refetch = useCallback(() => setNonce((n) => n + 1), []);

  useEffect(() => {
    let active = true;
    setLoading(true);
    setError(undefined);
    taskRef.current().then(
      (result) => {
        if (!active) return;
        setData(result);
        setLoading(false);
      },
      (err: unknown) => {
        if (!active) return;
        setError(err instanceof Error ? err : new Error(String(err)));
        setData(undefined);
        setLoading(false);
      },
    );
    return () => {
      active = false;
    };
    // taskRef is a stable ref; deps + nonce drive re-runs intentionally.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, nonce]);

  return { data, loading, error, refetch };
}

export default useAsync;

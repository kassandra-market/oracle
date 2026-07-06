import { useMemo, type ReactNode } from "react";
import { IndexerClient, IndexerContext } from "./indexer";

/**
 * Provides the app's single {@link IndexerClient} — the sole data + transaction
 * gateway (same-origin `/api/*`). Replaces the old RPC connection provider: the
 * app no longer knows about RPC endpoints or clusters, it only ever talks to the
 * indexer. The `useIndexer()` hook lives in `./indexer`.
 */
export function IndexerProvider({ children }: { children: ReactNode }) {
  const client = useMemo(() => new IndexerClient(), []);
  return <IndexerContext.Provider value={client}>{children}</IndexerContext.Provider>;
}

export default IndexerProvider;

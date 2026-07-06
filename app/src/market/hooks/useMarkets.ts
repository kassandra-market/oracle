/**
 * Query hook over the read layer: every market from the indexer. Re-runs on mount
 * (and whenever the {@link IndexerClient} identity changes), plus a manual
 * `refetch` for the error-state retry.
 */
import { useIndexer } from "../lib/indexer";
import { fetchMarkets, type MarketSummary } from "../data/markets";
import { useAsync, type AsyncState } from "./useAsync";

/** The market list: every mapped {@link MarketSummary}, most-funded first. */
export function useMarkets(): AsyncState<MarketSummary[]> {
  const indexer = useIndexer();
  return useAsync(() => fetchMarkets(indexer), [indexer]);
}

export default useMarkets;

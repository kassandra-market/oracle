/**
 * The categorical GROUP a market's oracle spans — every sub-market sharing that
 * oracle, plus the lifecycle-derived subsets {@link GroupLiquidityPanel} and the
 * market-detail page both need (which outcomes are still funding, which can take
 * AMM liquidity, which have LP to claim). Extracted from `GroupLiquidityPanel` so
 * the detail page can compute the SAME group once and use it both to gate the
 * per-market contribute form (no independent per-option funding once a group
 * exists) and to render one cumulative funding bar.
 */
import { useMemo } from "react";
import { MarketStatus } from "@kassandra-market/markets";
import { useMarkets } from "./useMarkets";
import type { MarketSummary } from "../data/markets";

export interface OracleGroupState {
  /** Every sub-market sharing this oracle, in outcome order. Empty while loading. */
  siblings: MarketSummary[];
  /** True once more than one sub-market exists for this oracle — a real group,
   *  as opposed to a lone (binary) market with no sibling outcomes. */
  isGroup: boolean;
  /** Siblings still in the Funding phase. */
  funding: MarketSummary[];
  /** Active siblings with known live reserves (can take AMM liquidity). */
  active: MarketSummary[];
  /** Siblings whose fee has been collected (can claim LP). */
  claimable: MarketSummary[];
  /** Funding + Active-with-reserves, outcome-ordered — everything that can take
   *  liquidity right now. */
  depositable: MarketSummary[];
  loading: boolean;
  /** Refetch resilient to indexer/RPC propagation lag — use as a write action's onSuccess. */
  refetch: () => void;
}

/** The group of sub-markets sharing `oracle`, with the lifecycle subsets pre-split. */
export function useOracleGroup(oracle: string): OracleGroupState {
  const { data: allMarkets, loading, refetchAfterWrite } = useMarkets();

  const siblings = useMemo<MarketSummary[]>(
    () =>
      (allMarkets ?? [])
        .filter((m) => m.market.oracle.toString() === oracle)
        .sort((a, b) => a.market.outcomeIndex - b.market.outcomeIndex),
    [allMarkets, oracle],
  );

  const funding = useMemo(
    () => siblings.filter((m) => m.market.status === MarketStatus.Funding),
    [siblings],
  );
  // Active outcomes can take AMM liquidity, but only when their live reserves are
  // known (needed to size the balanced deposit); drop any whose reserves haven't
  // loaded yet.
  const active = useMemo(
    () => siblings.filter((m) => m.market.status === MarketStatus.Active && m.reserves != null),
    [siblings],
  );
  const depositable = useMemo(
    () => [...funding, ...active].sort((a, b) => a.market.outcomeIndex - b.market.outcomeIndex),
    [funding, active],
  );
  const claimable = useMemo(() => siblings.filter((m) => m.market.feeCollected), [siblings]);

  return {
    siblings,
    isGroup: siblings.length > 1,
    funding,
    active,
    claimable,
    depositable,
    loading,
    refetch: refetchAfterWrite,
  };
}

export default useOracleGroup;

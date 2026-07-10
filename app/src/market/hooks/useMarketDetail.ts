/**
 * Query hooks over the read layer for one market's detail + the program config.
 *
 * {@link useMarketDetail} wraps {@link fetchMarketDetail} in {@link useAsync} and
 * — mirroring the sibling `useMarketAmms` setInterval-in-effect pattern — polls
 * every {@link POLL_MS} while the market is **Active** (its cYES/cNO pool prices
 * move), so the live YES probability + reserves refresh without a websocket.
 * Settled/funding markets don't poll.
 */
import { useCallback, useEffect, useRef } from "react";
import { MarketStatus } from "@kassandra-market/markets";
import { useIndexer } from "../lib/indexer";
import { fetchConfig, fetchMarketDetail, type MarketDetail } from "../data/markets";
import { useAsync, type AsyncState } from "./useAsync";
import type { Config } from "@kassandra-market/markets";

/** Poll interval (ms) while a live, Active market is on screen. */
const POLL_MS = 15_000;

/**
 * Post-write refetch schedule (ms after an action confirms). The indexer's
 * account store trails the chain by up to its reconcile interval — and there is a
 * brief window between a tx confirming and the store reflecting it — so a SINGLE
 * refetch right after an action can read stale state (e.g. a just-activated market
 * still reading `Funding`), leaving the UI stuck until a manual reload. This short
 * burst reliably catches the update within a few seconds regardless of that lag.
 */
const AFTER_WRITE_MS = [0, 800, 1800, 3500];

export interface MarketDetailState extends AsyncState<MarketDetail> {
  /** Refetch resilient to indexer reconcile lag — use as an action's onSuccess. */
  refetchAfterWrite: () => void;
}

/** One market plus its children, oracle, and live reserves; polls while Active. */
export function useMarketDetail(pubkey: string | undefined): MarketDetailState {
  const indexer = useIndexer();
  const state = useAsync(
    () =>
      pubkey
        ? fetchMarketDetail(indexer, pubkey)
        : Promise.reject<MarketDetail>(new Error("No market address supplied.")),
    [indexer, pubkey],
  );

  const active = state.data?.market.status === MarketStatus.Active;
  const { refetch } = state;
  useEffect(() => {
    if (!active) return;
    const timer = setInterval(refetch, POLL_MS);
    return () => clearInterval(timer);
  }, [active, refetch]);

  // Burst of refetches after a write, cleared on unmount.
  const timers = useRef<ReturnType<typeof setTimeout>[]>([]);
  useEffect(() => () => timers.current.forEach(clearTimeout), []);
  const refetchAfterWrite = useCallback(() => {
    timers.current.forEach(clearTimeout);
    timers.current = AFTER_WRITE_MS.map((ms) => setTimeout(refetch, ms));
  }, [refetch]);

  return { ...state, refetchAfterWrite };
}

/** The program `Config` singleton (KASS mint + funding floor), or `null` if uninitialised. */
export function useConfig(): AsyncState<Config | null> {
  const indexer = useIndexer();
  return useAsync(() => fetchConfig(indexer), [indexer]);
}

export default useMarketDetail;

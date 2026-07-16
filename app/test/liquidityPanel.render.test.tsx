/**
 * Headless render coverage for the liquidity panel — now on EVERY market, not
 * just a group. Data hooks are mocked so the panel sees a categorical GROUP, a
 * lone market, or a settled market the wallet has a position in, and we assert
 * (via `renderToStaticMarkup`):
 *   - a group → uniform-split "Provide liquidity to N outcomes";
 *   - a lone funding market → renders a single "Provide liquidity" (no self-hide);
 *   - only funding outcomes are depositable;
 *   - withdraw appears only when the wallet holds an unclaimed, fee-collected LP.
 */
import { vi } from "vitest";

const WALLET = "Wa11et11111111111111111111111111111111111111";
const state = vi.hoisted(() => ({ siblings: [] as unknown[] }));

vi.mock("../src/market/hooks/useMarkets", () => ({
  useMarkets: () => ({ data: state.siblings, loading: false, error: undefined, refetch: () => {} }),
}));
vi.mock("../src/market/hooks/useMarketDetail", () => ({
  useConfig: () => ({ data: { kassMint: { toString: () => "Kass1111111111111111111111111111111111111111" } }, loading: false, error: undefined, refetch: () => {} }),
}));
vi.mock("../src/market/hooks/useKassBalance", () => ({
  useKassBalance: () => ({ balance: 1_000_000_000_000n, loading: false, refetch: () => {} }),
}));
vi.mock("../src/market/hooks/useActionSequence", () => ({
  useActionSequence: () => ({ statuses: [], busy: false, connected: true, address: WALLET, allDone: false, run: async () => {}, reset: () => {} }),
}));
vi.mock("../src/market/lib/indexer", () => ({ useIndexer: () => ({}) }));
vi.mock("../src/components/markets/actions/ConnectGate", () => ({
  ConnectGate: ({ children }: { children: unknown }) => children,
}));

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { MarketStatus } from "@kassandra-market/markets";
import { describe, expect, it } from "vitest";

import { LiquidityPanel } from "../src/components/markets/actions/LiquidityPanel";
import type { MarketDetail } from "../src/market/data/markets";

const ORACLE = "Orac1e1111111111111111111111111111111111111";

function summary(outcomeIndex: number, status: MarketStatus, feeCollected = false) {
  return {
    pubkey: `Market${outcomeIndex}1111111111111111111111111111111111`,
    market: {
      oracle: { toString: () => ORACLE },
      outcomeIndex,
      status,
      feeCollected,
      lpMint: { toString: () => `LpMint${outcomeIndex}111111111111111111111111111111111` },
    },
    reserves: null,
    oracleOptionsCount: 3,
  } as unknown;
}

function detailOf(outcomeIndex: number, status: MarketStatus, feeCollected = false, walletUnclaimed = false): MarketDetail {
  const s = summary(outcomeIndex, status, feeCollected) as { pubkey: string; market: unknown };
  return {
    pubkey: s.pubkey,
    market: s.market,
    reserves: null,
    oracle: null,
    contributions: walletUnclaimed
      ? [{ pubkey: "Contrib1111111111111111111111111111111111111", contribution: { contributor: { toString: () => WALLET }, claimed: false } }]
      : [],
  } as unknown as MarketDetail;
}

const render = (detail: MarketDetail) => renderToStaticMarkup(<LiquidityPanel detail={detail} />);

describe("LiquidityPanel", () => {
  it("renders bulk provide-liquidity for a categorical group in funding", () => {
    state.siblings = [
      summary(0, MarketStatus.Funding),
      summary(1, MarketStatus.Funding),
      summary(2, MarketStatus.Funding),
    ];
    const html = render(detailOf(0, MarketStatus.Funding));
    expect(html).toContain("Provide liquidity");
    expect(html).toContain("all 3 outcomes");
    expect(html).toContain("Provide liquidity to 3 outcomes");
    expect(html).toMatch(/Split uniformly across 3 funding outcomes/);
  });

  it("renders on a LONE funding market (no self-hide) with a single provide button", () => {
    state.siblings = []; // list not loaded / not listed → falls back to the current market
    const html = render(detailOf(0, MarketStatus.Funding));
    expect(html).toContain("Provide liquidity");
    expect(html).not.toContain("outcomes of this market"); // not a group
    expect(html).toContain(">Provide liquidity<"); // single-market button label
  });

  it("counts only funding outcomes as depositable", () => {
    state.siblings = [
      summary(0, MarketStatus.Funding),
      summary(1, MarketStatus.Active),
      summary(2, MarketStatus.Funding),
    ];
    const html = render(detailOf(0, MarketStatus.Funding));
    expect(html).toContain("all 3 outcomes");
    expect(html).toContain("Provide liquidity to 2 outcomes");
  });

  it("shows funding-closed on a live single market and no withdraw without a position", () => {
    state.siblings = [];
    const html = render(detailOf(0, MarketStatus.Active));
    expect(html).toContain("Funding is closed");
    expect(html).not.toContain("Withdraw");
  });

  it("offers withdraw only when the wallet holds an unclaimed, fee-collected position", () => {
    state.siblings = [];
    const withPos = render(detailOf(0, MarketStatus.Resolved, true, true));
    expect(withPos).toContain("Withdraw liquidity");
    // fee collected but the wallet has no position → no withdraw
    const noPos = render(detailOf(0, MarketStatus.Resolved, true, false));
    expect(noPos).not.toContain("Withdraw liquidity");
  });
});

/**
 * End-to-end render coverage for a Funding-phase, GROUPED market's Liquidity tab:
 * the top-of-tab overview shows the GROUP's cumulative raised/floor (not just this
 * one outcome's own numbers), there is exactly ONE progress bar on the page, and
 * the per-outcome contribute form is replaced by a pointer to the group panel
 * below — closing the "fund each option independently" gap end-to-end (not just
 * per-component, which the other test files already cover in isolation).
 */
import { vi } from "vitest";
import { MarketStatus } from "@kassandra-market/markets";
import { Phase } from "@kassandra-market/oracles";

const PUB1 = "Market11111111111111111111111111111111111111"; // outcome 1 of the group
const ORACLE = "Orac1eGroup111111111111111111111111111111111";

const detail = {
  pubkey: PUB1,
  market: {
    status: MarketStatus.Funding,
    outcomeIndex: 1,
    settled: false,
    openContributions: 1,
    totalContributed: 80_000_000_000n, // THIS outcome alone raised 80 KASS
    minLiquidity: 500_000_000_000n, // THIS outcome's own floor: 500 KASS
    feeBps: 250,
    feeCollected: false,
    oracle: { toString: () => ORACLE },
  },
  oracle: { optionsCount: 3, phase: Phase.Proposal, resolvedOption: 0 },
  reserves: null,
  contributions: [],
};

// The group: 3 Funding outcomes, raised 5k + 80k + 120k = 205,000, floor 500k×3 = 1,500,000.
const groupSiblings = [
  { pubkey: "Cat0", market: { outcomeIndex: 0, status: MarketStatus.Funding, totalContributed: 5_000_000_000n, minLiquidity: 500_000_000_000n }, reserves: null },
  { pubkey: PUB1, market: { outcomeIndex: 1, status: MarketStatus.Funding, totalContributed: 80_000_000_000n, minLiquidity: 500_000_000_000n }, reserves: null },
  { pubkey: "Cat2", market: { outcomeIndex: 2, status: MarketStatus.Funding, totalContributed: 120_000_000_000n, minLiquidity: 500_000_000_000n }, reserves: null },
];

vi.mock("../src/market/hooks/useMarketDetail", () => ({
  useMarketDetail: () => ({ data: detail, loading: false, error: undefined, refetch: () => {}, refetchAfterWrite: () => {} }),
  useConfig: () => ({ data: undefined, loading: false, error: undefined, refetch: () => {} }),
}));
vi.mock("../src/market/hooks/useOracleGroup", () => ({
  useOracleGroup: () => ({
    siblings: groupSiblings,
    isGroup: true,
    funding: groupSiblings,
    active: [],
    claimable: [],
    depositable: groupSiblings,
    loading: false,
    refetch: () => {},
  }),
}));
vi.mock("../src/hooks/useOracleMeta", () => ({ useOracleMeta: () => new Map() }));
vi.mock("@solana/wallet-adapter-react", () => ({ useWallet: () => ({ publicKey: null }) }));
vi.mock("../src/market/hooks/useWriteAction", () => ({
  useWriteAction: () => ({ status: { kind: "idle" }, address: null, connected: false, indexer: {}, run: async () => {} }),
}));
vi.mock("../src/market/hooks/useKassBalance", () => ({
  useKassBalance: () => ({ balance: null, loading: false, refetch: () => {} }),
}));
vi.mock("../src/components/markets/actions/ConnectGate", () => ({
  ConnectGate: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
// The group panel's own internals are covered by groupLiquidityPanel.render.test.tsx —
// stub it here to a marker so this test only asserts it's reached with a real group.
vi.mock("../src/components/markets/actions/GroupLiquidityPanel", () => ({
  GroupLiquidityPanel: () => <div data-testid="group-liquidity-panel-stub" />,
}));
vi.mock("../src/components/markets/actions/TradePanel", () => ({ TradePanel: () => null }));

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { describe, expect, it } from "vitest";

import MarketDetail from "../src/pages/MarketDetail";

function render(): string {
  return renderToStaticMarkup(
    <MemoryRouter initialEntries={[`/markets/${PUB1}`]}>
      <Routes>
        <Route path="/markets/:pubkey" element={<MarketDetail />} />
      </Routes>
    </MemoryRouter>,
  );
}

describe("Liquidity tab — grouped Funding market", () => {
  it("shows the GROUP's cumulative raised/floor, not just this outcome's own numbers", () => {
    const html = render();
    // 5 + 80 + 120 = 205 KASS raised; 500×3 = 1,500 KASS floor (whole-KASS display).
    expect(html).toMatch(/205 KASS/);
    expect(html).toMatch(/1,500 KASS/);
    // The overview's headline "Raised" figure is the group total (205), not this
    // outcome's own lone total (80) — the pool-composition panel below still
    // shows this outcome's own 80/500 (a different, still-per-market section).
    expect(html).toMatch(/Raised<\/span><span[^>]*>205 KASS/);
  });

  it("renders exactly ONE progress bar on the page", () => {
    const html = render();
    expect(html.match(/role="progressbar"/g)?.length).toBe(1);
  });

  it("replaces the per-outcome contribute form with a pointer to the group panel, which is reached with a real group", () => {
    const html = render();
    expect(html).not.toContain("Contribute funding");
    expect(html).toContain("Fund this option as part of the group below");
    expect(html).toContain('data-testid="group-liquidity-panel-stub"');
  });
});

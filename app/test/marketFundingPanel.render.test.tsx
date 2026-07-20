/**
 * Render coverage for the enriched Funding & liquidity panel: the LP provenance
 * split (funding vs independent LPs, from activationLp / grossLpTotal) and the
 * underlying pool's cYES/cNO token composition. Shown on the default Liquidity tab
 * for a non-Active market. Heavy action panels are stubbed.
 */
import { vi } from "vitest";
import { MarketStatus } from "@kassandra-market/markets";
import { Phase } from "@kassandra-market/oracles";

const PUB = "Market11111111111111111111111111111111111111";
const ORACLE = "Orac1e1111111111111111111111111111111111111";

const detail = {
  pubkey: PUB,
  market: {
    status: MarketStatus.Resolved, // isActive === false → default tab is Liquidity
    outcomeIndex: 0,
    settled: true,
    openContributions: 0,
    totalContributed: 3_000_000_000n,
    minLiquidity: 1_000_000_000n,
    feeBps: 100,
    feeCollected: true,
    oracle: { toString: () => ORACLE },
    // LP provenance: 100 funding LP + 50 independent LP = 150 gross.
    activationLp: 100_000_000_000n,
    grossLpTotal: 150_000_000_000n,
    lpTotal: 150_000_000_000n,
    activationContributed: 1_000_000_000n,
  },
  oracle: { optionsCount: 2, phase: Phase.Resolved, resolvedOption: 0 },
  reserves: { base: 640_000_000n, quote: 360_000_000n },
  contributions: [],
};

vi.mock("../src/market/hooks/useMarketDetail", () => ({
  useMarketDetail: () => ({ data: detail, loading: false, error: undefined, refetch: () => {}, refetchAfterWrite: () => {} }),
  useConfig: () => ({ data: undefined, loading: false, error: undefined, refetch: () => {} }),
}));
vi.mock("../src/market/hooks/useOracleGroup", () => ({
  useOracleGroup: () => ({
    siblings: [],
    isGroup: false,
    funding: [],
    active: [],
    claimable: [],
    depositable: [],
    loading: false,
    refetch: () => {},
  }),
}));
vi.mock("../src/hooks/useOracleMeta", () => ({ useOracleMeta: () => new Map() }));
vi.mock("../src/components/markets/actions/MarketActions", () => ({
  MarketLiquidityActions: () => null,
  MarketLifecycleActions: () => null,
}));
vi.mock("../src/components/markets/actions/GroupLiquidityPanel", () => ({
  GroupLiquidityPanel: () => null,
}));
// The redesigned Liquidity tab's overview reads the connected wallet; render it
// disconnected (no WalletProvider in this static render).
vi.mock("@solana/wallet-adapter-react", () => ({
  useWallet: () => ({ publicKey: null }),
}));

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { describe, expect, it } from "vitest";

import MarketDetail from "../src/pages/MarketDetail";

function render(query = ""): string {
  return renderToStaticMarkup(
    <MemoryRouter initialEntries={[`/markets/${PUB}${query}`]}>
      <Routes>
        <Route path="/markets/:pubkey" element={<MarketDetail />} />
      </Routes>
    </MemoryRouter>,
  );
}

describe("Funding & liquidity panel", () => {
  it("splits LP provenance between funding and independent LPs", () => {
    const html = render();
    expect(html).toContain("LP composition");
    // 100 / 150 = 66.7% funding, 50 / 150 = 33.3% independent.
    expect(html).toMatch(/From funding \(66\.7%\)/);
    expect(html).toMatch(/From independent LPs \(33\.3%\)/);
    expect(html).toMatch(/100\s*LP/);
    expect(html).toMatch(/50\s*LP/);
    expect(html).toMatch(/150\s*LP/); // total
  });

  it("details the underlying pool's cYES/cNO token composition", () => {
    const html = render();
    expect(html).toContain("Pool composition");
    expect(html).toContain("cYES (pays 1 KASS on YES)");
    expect(html).toContain("cNO (pays 1 KASS on NO)");
  });

  it("shows the pool's cYES/cNO reserve AMOUNTS at the top of the tab, not a KASS pool value", () => {
    const html = render();
    expect(html).not.toContain("Pool value");
    // The overview stat tiles surface each side's raw reserve amount (base=cYES,
    // quote=cNO from the fixture's `reserves`), not a mark-to-market KASS total.
    expect(html).toMatch(/>cYES<[\s\S]*?0\.64/);
    expect(html).toMatch(/>cNO<[\s\S]*?0\.36/);
  });

  it("does NOT show the implied-probability gauge on the Liquidity tab", () => {
    const html = render();
    // The gauge moved to the Details tab (dormant here).
    expect(html).not.toContain("Implied YES probability");
  });

  it("restores the tab named in ?tab= on load (refresh persistence)", () => {
    // A non-default tab (default here is Liquidity) is opened straight from the URL.
    const html = render("?tab=manage");
    expect(html).toContain('role="tabpanel" id="panel-manage"');
    expect(html).not.toContain('role="tabpanel" id="panel-liquidity"');
  });

  it("falls back to the default tab when ?tab= is unknown", () => {
    const html = render("?tab=bogus");
    expect(html).toContain('role="tabpanel" id="panel-liquidity"');
    expect(html).not.toContain('role="tabpanel" id="panel-manage"');
  });
});

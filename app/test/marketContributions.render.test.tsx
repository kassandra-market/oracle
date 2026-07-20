/**
 * Regression coverage for the redesigned Liquidity tab's contributions ledger:
 * each contribution expands into up to two TAGGED rows — an "Initial funding" row
 * (`amount` KASS) and a "Liquidity" row (`lateLp` LP) — instead of one combined
 * "KASS · LP" line. A pure funder shows only funding; a pure late LP shows only
 * the LP it added (never a misleading "0 KASS"); a both-cohort contributor shows
 * two separate rows. Also covers the top-panel LP overview (cYES/cNO reserve
 * amounts, LP supply, the connected wallet's share).
 */
import { vi } from "vitest";
import { MarketStatus } from "@kassandra-market/markets";
import { Phase } from "@kassandra-market/oracles";

const PUB = "Market11111111111111111111111111111111111111";
const ORACLE = "Orac1e1111111111111111111111111111111111111";

// A Resolved (activated) market — isActive === false → default tab is Liquidity,
// which holds the overview + ledger. Three contributions: a funder, a pure late
// LP, and a both-cohort contributor. `activationContributed` = the funding that
// converted to LP at activation (Fund 1 + Both 2 = 3); `grossLpTotal` = that
// activation LP (3) + all late LP (Both 3 + Late 0.5 = 3.5) = 6.5.
const detail = {
  pubkey: PUB,
  market: {
    status: MarketStatus.Resolved,
    outcomeIndex: 0,
    settled: true,
    openContributions: 3,
    totalContributed: 3_000_000_000n,
    minLiquidity: 1_000_000_000n,
    feeBps: 0,
    feeCollected: true,
    oracle: { toString: () => ORACLE },
    activationLp: 3_000_000_000n,
    activationContributed: 3_000_000_000n,
    grossLpTotal: 6_500_000_000n,
    lpTotal: 6_500_000_000n,
  },
  oracle: { optionsCount: 2, phase: Phase.Resolved, resolvedOption: 0 },
  reserves: { base: 6_000_000_000n, quote: 4_000_000_000n }, // 6 cYES / 4 cNO
  contributions: [
    { pubkey: "C3", slot: 300n, contribution: { contributor: { toString: () => "Both1111" }, amount: 2_000_000_000n, lateLp: 3_000_000_000n, claimed: false } },
    { pubkey: "C1", slot: 200n, contribution: { contributor: { toString: () => "Fund1111" }, amount: 1_000_000_000n, lateLp: 0n, claimed: false } },
    { pubkey: "C2", slot: 100n, contribution: { contributor: { toString: () => "Late1111" }, amount: 0n, lateLp: 500_000_000n, claimed: false } },
  ],
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
// A connected wallet = the both-cohort contributor, so the overview's "Your share"
// resolves (its gross LP = 2/3·3 activation + 3 late = 5 of 6.5 = 76.9%).
vi.mock("@solana/wallet-adapter-react", () => ({
  useWallet: () => ({ publicKey: { toBase58: () => "Both1111" } }),
}));
// Stub the context-heavy action surfaces so only the overview + ledger render.
vi.mock("../src/components/markets/actions/MarketActions", () => ({
  MarketLiquidityActions: () => null,
  MarketLifecycleActions: () => null,
}));
vi.mock("../src/components/markets/actions/GroupLiquidityPanel", () => ({
  GroupLiquidityPanel: () => null,
}));

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { describe, expect, it } from "vitest";

import MarketDetail from "../src/pages/MarketDetail";

function render(): string {
  return renderToStaticMarkup(
    <MemoryRouter initialEntries={[`/markets/${PUB}`]}>
      <Routes>
        <Route path="/markets/:pubkey" element={<MarketDetail />} />
      </Routes>
    </MemoryRouter>,
  );
}

describe("contributions ledger — tagged funding vs liquidity rows", () => {
  it("splits each contribution into its own tagged row (never a combined line)", () => {
    const html = render();
    // The pure late LP surfaces the LP it added — not "0 KASS".
    expect(html).toMatch(/0\.5\s*LP/);
    // The pure funder's KASS stake shows.
    expect(html).toMatch(/1\s*KASS/);
    // The both-cohort contributor now shows TWO separate rows, not "2 KASS · 3 LP".
    expect(html).toMatch(/2\s*KASS/);
    expect(html).toMatch(/3\s*LP/);
    expect(html).not.toMatch(/2\s*KASS\s*·\s*3\s*LP/);
    // Both action tags are present.
    expect(html).toContain("Initial funding");
    expect(html).toContain("Liquidity");
  });

  it("shows the LP overview: cYES/cNO reserves, LP supply, and the wallet's share", () => {
    const html = render();
    expect(html).not.toContain("Pool value");
    expect(html).toMatch(/>cYES<[\s\S]*?>6</); // cYES reserve amount (reserves.base)
    expect(html).toMatch(/>cNO<[\s\S]*?>4</); // cNO reserve amount (reserves.quote)
    expect(html).toMatch(/6\.5\s*shares/); // LP supply (grossLpTotal)
    expect(html).toMatch(/76\.9%/); // your share of the pool
    expect(html).toMatch(/5\s*LP/); // your gross LP
  });
});

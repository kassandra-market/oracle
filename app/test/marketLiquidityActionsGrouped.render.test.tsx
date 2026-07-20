/**
 * Render coverage for `MarketLiquidityActions`'s Funding-phase branch: a LONE
 * (non-grouped) market gets its own `ContributeForm`, but a GROUPED market
 * (`isGrouped`) gets no per-outcome contribute form at all — funding only
 * happens through the group's cumulative deposit action, so there is exactly
 * one way to fund a still-funding categorical group, not one per outcome.
 */
import { vi } from "vitest";

vi.mock("../src/market/hooks/useWriteAction", () => ({
  useWriteAction: () => ({
    status: { kind: "idle" },
    address: "Trader111111111111111111111111111111111",
    connected: true,
    indexer: {},
    run: async () => {},
  }),
}));
vi.mock("../src/market/hooks/useKassBalance", () => ({
  useKassBalance: () => ({ balance: 0n, loading: false, refetch: () => {} }),
}));
vi.mock("../src/components/markets/actions/ConnectGate", () => ({
  ConnectGate: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { MarketStatus } from "@kassandra-market/markets";
import { describe, expect, it } from "vitest";

import { MarketLiquidityActions } from "../src/components/markets/actions/MarketActions";
import type { MarketDetail } from "../src/market/data/markets";

const detail = {
  pubkey: "Market1111111111111111111111111111111111111",
  market: {
    status: MarketStatus.Funding,
    kassMint: { toString: () => "Kass1111111111111111111111111111111111111111" },
  },
  contributions: [],
} as unknown as MarketDetail;

function render(isGrouped: boolean): string {
  return renderToStaticMarkup(
    <MarketLiquidityActions detail={detail} refetch={() => {}} isGrouped={isGrouped} />,
  );
}

describe("MarketLiquidityActions — Funding phase", () => {
  it("renders the per-market ContributeForm for a LONE market", () => {
    const html = render(false);
    expect(html).toContain("Contribute funding");
    expect(html).not.toContain("Fund this option as part of the group below");
  });

  it("renders no per-outcome contribute form for a GROUPED market — only a pointer to the group panel", () => {
    const html = render(true);
    expect(html).not.toContain("Contribute funding");
    expect(html).toContain("Fund this option as part of the group below");
  });
});

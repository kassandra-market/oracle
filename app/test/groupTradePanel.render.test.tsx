/**
 * Headless render coverage for the unified Trade-tab surface: an outcome
 * selector over the existing single-market TradePanel, letting a categorical
 * group's outcomes be traded from one page without navigating away. `TradePanel`
 * is mocked to a stub that prints the props it was handed, so we can assert
 * WHICH outcome's pool the panel is wired to without needing real chain data.
 */
import { vi } from "vitest";

vi.mock("../src/components/markets/actions/TradePanel", () => ({
  TradePanel: ({ pubkey, boundLabel, question }: { pubkey: string; boundLabel?: string | null; question?: string }) => (
    <div data-testid="trade-panel" data-pubkey={pubkey} data-bound-label={boundLabel ?? ""}>
      {question}
    </div>
  ),
}));

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { MarketStatus } from "@kassandra-market/markets";
import { describe, expect, it, vi as vitestVi } from "vitest";

import { GroupTradePanel } from "../src/components/markets/actions/GroupTradePanel";
import type { OracleGroupState } from "../src/market/hooks/useOracleGroup";
import type { MarketDetail as MarketDetailData } from "../src/market/data/markets";

const ORACLE = "Orac1e1111111111111111111111111111111111111";

function summary(outcomeIndex: number, status: MarketStatus, reserves: { base: bigint; quote: bigint } | null = null) {
  return {
    pubkey: `Market${outcomeIndex}1111111111111111111111111111111111`,
    market: { oracle: { toString: () => ORACLE }, outcomeIndex, status },
    reserves,
    oracleOptionsCount: 3,
  } as unknown as { pubkey: string; market: Record<string, unknown>; reserves: unknown; oracleOptionsCount: number };
}

function detail(pubkey: string, outcomeIndex: number, status: MarketStatus, reserves: { base: bigint; quote: bigint } | null): MarketDetailData {
  return {
    pubkey,
    market: { oracle: { toString: () => ORACLE }, outcomeIndex, status } as never,
    contributions: [],
    oracle: null,
    reserves: reserves as never,
  };
}

function group(active: ReturnType<typeof summary>[]): OracleGroupState {
  return {
    siblings: active as never,
    isGroup: active.length > 1,
    funding: [],
    active: active as never,
    claimable: [],
    depositable: [],
    loading: false,
    refetch: vitestVi.fn(),
  };
}

const R = { base: 6_000_000n, quote: 4_000_000n };
const R2 = { base: 3_000_000n, quote: 7_000_000n };

function render(props: Parameters<typeof GroupTradePanel>[0]): string {
  return renderToStaticMarkup(<GroupTradePanel {...props} />);
}

describe("GroupTradePanel", () => {
  it("renders no outcome selector for a lone Active market — just the plain TradePanel", () => {
    // A genuinely lone market has no OracleGroup siblings at all — the current
    // market itself is the only tradable outcome.
    const d = detail("MarketA", 0, MarketStatus.Active, R);
    const html = render({ detail: d, group: group([]), options: [], refetch: () => {} });
    expect(html).not.toContain('role="tablist"');
    expect(html).toContain('data-testid="trade-panel"');
    expect(html).toContain('data-pubkey="MarketA"');
  });

  it("shows an outcome selector for a multi-outcome group, defaulting to the CURRENT market", () => {
    const d = detail("Market1111111111111111111111111111111111111", 1, MarketStatus.Active, R);
    const g = group([
      summary(0, MarketStatus.Active, R2),
      summary(1, MarketStatus.Active, R),
      summary(2, MarketStatus.Active, R),
    ]);
    const html = render({ detail: d, group: g, options: ["Zero", "One", "Two"], refetch: () => {} });
    expect(html).toContain('role="tablist"');
    // All three outcome pills render, labelled + probability-tagged.
    expect(html).toContain("Zero");
    expect(html).toContain("One");
    expect(html).toContain("Two");
    // Defaults to the CURRENT market (outcome 1), not outcome 0 despite sorting first.
    expect(html).toContain('data-pubkey="Market1111111111111111111111111111111111111"');
    expect(html).toContain('data-bound-label="One"');
  });

  it("defaults to the first tradable sibling when the CURRENT market itself isn't Active", () => {
    // Viewing outcome 0's page while it's still Funding, but outcome 2 is Active.
    const d = detail("Market0111111111111111111111111111111111111", 0, MarketStatus.Funding, null);
    const g = group([summary(2, MarketStatus.Active, R)]);
    const html = render({ detail: d, group: g, options: [], refetch: () => {} });
    expect(html).toContain('data-pubkey="Market21111111111111111111111111111');
  });

  it("passes the shared oracle subject as the trade question, and the picked outcome's own bound label", () => {
    const d = detail("Market1111111111111111111111111111111111111", 1, MarketStatus.Active, R);
    const g = group([summary(1, MarketStatus.Active, R)]);
    const html = render({ detail: d, group: g, subject: "Who wins the tournament?", options: ["Zero", "One"], refetch: () => {} });
    expect(html).toContain("Who wins the tournament?");
    expect(html).toContain('data-bound-label="One"');
  });

  it("renders nothing when no outcome in the group is tradable", () => {
    const d = detail("Market0111111111111111111111111111111111111", 0, MarketStatus.Funding, null);
    const g = group([]);
    expect(render({ detail: d, group: g, options: [], refetch: () => {} })).toBe("");
  });
});

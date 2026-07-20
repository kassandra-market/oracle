/**
 * Headless render coverage for the group bulk-liquidity panel. The `group` prop
 * (what `useOracleGroup` would compute) is built directly per test, and we assert
 * (via `renderToStaticMarkup`):
 *   - a group renders the "Group liquidity" panel + the uniform-split deposit UI;
 *   - a lone market renders NOTHING (self-hides — it uses the single-market form);
 *   - while any outcome is still Funding, ONE cumulative progress bar shows the
 *     group's combined raised/floor — not a bar per outcome;
 *   - the deposit affordance targets exactly the funding outcomes;
 *   - a deposit/withdraw's completion refetches the group (via `group.refetch`),
 *     the KASS balance, AND the parent market-detail page — not just the balance
 *     (regression coverage for the missing-refetch bug: a bulk deposit used to
 *     leave both this panel's reserves and the page's pool value/price impact
 *     stuck on pre-deposit data).
 */
import { vi } from "vitest";

// The `useActionSequence` mock captures its `onDone` callback here so tests can
// invoke it directly and assert on the refetch spies below — the mock's `run`
// never actually calls it (no real async sequence executes under SSR).
const captured = vi.hoisted(() => ({ onDone: undefined as (() => void) | undefined }));
const spies = vi.hoisted(() => ({
  refetchMarkets: vi.fn(),
  refetchBalance: vi.fn(),
  onSuccess: vi.fn(),
}));

vi.mock("../src/market/hooks/useMarketDetail", () => ({
  useConfig: () => ({ data: { kassMint: { toString: () => "Kass1111111111111111111111111111111111111111" } }, loading: false, error: undefined, refetch: () => {} }),
}));
vi.mock("../src/market/hooks/useKassBalance", () => ({
  useKassBalance: () => ({ balance: 1_000_000_000_000n, loading: false, refetch: spies.refetchBalance }),
}));
vi.mock("../src/market/hooks/useActionSequence", () => ({
  useActionSequence: (onDone?: () => void) => {
    captured.onDone = onDone;
    return { statuses: [], busy: false, connected: true, address: "Wa11et11111111111111111111111111111111111111", allDone: false, run: async () => {}, reset: () => {} };
  },
}));
vi.mock("../src/market/lib/indexer", () => ({ useIndexer: () => ({}) }));
vi.mock("../src/components/markets/actions/ConnectGate", () => ({
  // Render children directly (connected) so the panel body is inspectable.
  ConnectGate: ({ children }: { children: unknown }) => children,
}));

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { MarketStatus } from "@kassandra-market/markets";
import { describe, expect, it } from "vitest";

import { GroupLiquidityPanel } from "../src/components/markets/actions/GroupLiquidityPanel";
import type { OracleGroupState } from "../src/market/hooks/useOracleGroup";

const ORACLE = "Orac1e1111111111111111111111111111111111111";

function summary(
  outcomeIndex: number,
  status: MarketStatus,
  feeCollected = false,
  reserves: { base: bigint; quote: bigint } | null = null,
  totalContributed = 0n,
  minLiquidity = 0n,
) {
  return {
    pubkey: `Market${outcomeIndex}1111111111111111111111111111111111`,
    market: {
      oracle: { toString: () => ORACLE },
      outcomeIndex,
      status,
      feeCollected,
      totalContributed,
      minLiquidity,
      lpMint: { toString: () => `LpMint${outcomeIndex}111111111111111111111111111111111` },
    },
    reserves,
    oracleOptionsCount: 3,
  } as unknown as { pubkey: string; market: Record<string, unknown>; reserves: unknown };
}

/** Mirrors `useOracleGroup`'s own derivation — builds the same shape from raw siblings. */
function buildGroup(siblings: ReturnType<typeof summary>[]): OracleGroupState {
  const funding = siblings.filter((m) => (m.market as { status: MarketStatus }).status === MarketStatus.Funding);
  const active = siblings.filter(
    (m) => (m.market as { status: MarketStatus }).status === MarketStatus.Active && m.reserves != null,
  );
  const depositable = [...funding, ...active].sort(
    (a, b) =>
      (a.market as { outcomeIndex: number }).outcomeIndex - (b.market as { outcomeIndex: number }).outcomeIndex,
  );
  const claimable = siblings.filter((m) => (m.market as { feeCollected: boolean }).feeCollected);
  return {
    siblings: siblings as never,
    isGroup: siblings.length > 1,
    funding: funding as never,
    active: active as never,
    claimable: claimable as never,
    depositable: depositable as never,
    loading: false,
    refetch: spies.refetchMarkets,
  };
}

function render(siblings: ReturnType<typeof summary>[], onSuccess?: () => void): string {
  return renderToStaticMarkup(<GroupLiquidityPanel group={buildGroup(siblings)} onSuccess={onSuccess} />);
}

describe("GroupLiquidityPanel", () => {
  it("renders the bulk deposit panel for a categorical group in funding", () => {
    const html = render([
      summary(0, MarketStatus.Funding),
      summary(1, MarketStatus.Funding),
      summary(2, MarketStatus.Funding),
    ]);
    expect(html).toContain("Group liquidity");
    expect(html).toContain("all 3 outcomes");
    // Deposit affordance targets the 3 depositable outcomes with a uniform split.
    expect(html).toContain("Deposit into 3 outcomes");
    expect(html).toMatch(/Split uniformly across 3 outcomes/);
  });

  it("never renders its own progress bar — the single cumulative bar lives in LiquidityOverview above it, not duplicated here", () => {
    const html = render([
      summary(0, MarketStatus.Funding, false, null, 100_000_000_000n, 200_000_000_000n),
      summary(1, MarketStatus.Funding, false, null, 50_000_000_000n, 200_000_000_000n),
      summary(2, MarketStatus.Funding, false, null, 0n, 200_000_000_000n),
    ]);
    expect(html).not.toContain('role="progressbar"');
  });

  it("offers bulk add-liquidity for an ACTIVE group (with reserves), skipping reserveless outcomes", () => {
    const r = { base: 1_000_000n, quote: 1_000_000n };
    const html = render([
      summary(0, MarketStatus.Active, false, r),
      summary(1, MarketStatus.Active, false, r),
      summary(2, MarketStatus.Active, false, null), // reserves not loaded → excluded
    ]);
    // Only the 2 outcomes with known reserves are depositable.
    expect(html).toContain("Deposit into 2 outcomes");
    expect(html).toMatch(/Split uniformly across 2 outcomes/);
  });

  it("closes deposits when no outcome can take liquidity", () => {
    const html = render([
      summary(0, MarketStatus.Active, false, null), // Active but reserveless
      summary(1, MarketStatus.Resolved),
    ]);
    expect(html).toContain("deposits are closed for this group");
  });

  it("self-hides for a lone market (not a group)", () => {
    expect(render([summary(0, MarketStatus.Funding)])).toBe("");
  });

  it("only counts funding outcomes as depositable", () => {
    const html = render([
      summary(0, MarketStatus.Funding),
      summary(1, MarketStatus.Active), // active → not depositable
      summary(2, MarketStatus.Funding),
    ]);
    expect(html).toContain("all 3 outcomes"); // still a 3-outcome group
    expect(html).toContain("Deposit into 2 outcomes"); // but only 2 accept funding
  });

  it("offers bulk withdraw when outcomes have collected fees", () => {
    const html = render([
      summary(0, MarketStatus.Resolved, true),
      summary(1, MarketStatus.Resolved, true),
    ]);
    expect(html).toContain("Withdraw from 2 outcomes");
  });

  it("a completed deposit/withdraw sequence refetches the group, the KASS balance, AND the parent page — not just the balance", () => {
    spies.refetchMarkets.mockClear();
    spies.refetchBalance.mockClear();
    spies.onSuccess.mockClear();
    render([summary(0, MarketStatus.Funding), summary(1, MarketStatus.Funding)], spies.onSuccess);
    expect(captured.onDone).toBeTypeOf("function");
    captured.onDone!();
    expect(spies.refetchBalance).toHaveBeenCalledTimes(1);
    expect(spies.refetchMarkets).toHaveBeenCalledTimes(1);
    expect(spies.onSuccess).toHaveBeenCalledTimes(1);
  });

  it("tolerates a missing onSuccess (standalone, non-embedded usage)", () => {
    spies.refetchMarkets.mockClear();
    spies.refetchBalance.mockClear();
    render([summary(0, MarketStatus.Funding), summary(1, MarketStatus.Funding)]); // no onSuccess
    expect(() => captured.onDone!()).not.toThrow();
    expect(spies.refetchBalance).toHaveBeenCalledTimes(1);
    expect(spies.refetchMarkets).toHaveBeenCalledTimes(1);
  });
});

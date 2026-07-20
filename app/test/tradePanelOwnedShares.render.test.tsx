/**
 * Render coverage for TradePanel's "You own" row: the connected wallet's YES/NO
 * share holdings must be visible regardless of buy/sell mode (previously only
 * shown in sell mode via the amount field's balance line).
 */
import { vi } from "vitest";

const YES_MINT = "YesMint111111111111111111111111111111111";
const NO_MINT = "NoMint1111111111111111111111111111111111";
const KASS_MINT = "KassMint11111111111111111111111111111111";

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
  useKassBalance: (mint: string) => {
    if (mint === YES_MINT) return { balance: 42_000_000_000n, loading: false, refetch: () => {} };
    if (mint === NO_MINT) return { balance: 7_000_000_000n, loading: false, refetch: () => {} };
    return { balance: 100_000_000_000n, loading: false, refetch: () => {} };
  },
}));
vi.mock("../src/hooks/useKassUsdcPrice", () => ({
  useKassUsdcPrice: () => null,
}));
vi.mock("../src/components/markets/PriceChart", () => ({
  PriceChart: () => null,
}));
// ConnectGate reaches for the wallet-modal context (absent in a static render) —
// stub it to a pass-through so the connected form body renders.
vi.mock("../src/components/markets/actions/ConnectGate", () => ({
  ConnectGate: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { TradePanel } from "../src/components/markets/actions/TradePanel";

const market = {
  kassMint: { toString: () => KASS_MINT },
  yesMint: { toString: () => YES_MINT },
  noMint: { toString: () => NO_MINT },
} as never;
const reserves = { base: 1_000_000_000n, quote: 1_000_000_000n } as never;

function render(): string {
  return renderToStaticMarkup(
    <TradePanel pubkey="Market1111" market={market} reserves={reserves} onSuccess={() => {}} />,
  );
}

describe("TradePanel — owned-shares row", () => {
  it("shows both YES and NO holdings in buy mode (the default)", () => {
    const html = render();
    expect(html).toContain("You own");
    expect(html).toContain("42 YES");
    expect(html).toContain("7 NO");
  });
});

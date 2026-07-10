/**
 * SD1 headless render coverage for the ONE-CLICK settle control.
 *
 * Renders the two challenge surfaces the detail page mounts in the Challenge
 * phase — `ChallengeControl` (status + CU3 compose→open) and
 * `ChallengeTradeControls` (trade + crank + one-click settle) — under MOCK mode
 * (`VITE_MOCK=1`, a connected mock wallet) with a settle-OPEN market (twapEnd in
 * the past, !settled) via `renderToStaticMarkup`, and asserts:
 *
 *   - the combined challenge UI has EXACTLY ONE "Settle challenge" surface (the
 *     one-click `SettleButton`) — the old JSON-paste settle in ChallengeControl
 *     is gone, so there is no duplicate;
 *   - there is NO JSON `<textarea>` anywhere in the challenge UI;
 *   - the one-click settle button is present + enabled (the proposer authority
 *     resolves off the passed proposers) and keeps the verdict preview.
 *
 * SSR (no effects), so `buildSettleFromMarketIxs` is never invoked — this proves
 * the CONTROL shape (one-click, no paste), not the ix build (that is the unit +
 * E2E tests). Mock mode makes `useWriteAction` skip the real ix-build entirely.
 */
import { vi } from "vitest";
vi.stubEnv("VITE_MOCK", "1");

import React, { StrictMode, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ConnectionContext, WalletContext } from "@solana/wallet-adapter-react";
import type { WalletContextState } from "@solana/wallet-adapter-react";
import type { Market, Oracle, Proposer } from "@kassandra-market/oracles";
import { describe, expect, it } from "vitest";

import { ChallengeControl } from "../src/components/oracles/actions/ChallengeControl.tsx";
import { ChallengeTradeControls } from "../src/components/oracles/actions/ChallengeTradeControls.tsx";
import { ClusterContext } from "../src/lib/cluster.ts";

// Mock addresses are display-only strings (mock mode never feeds them to
// `new Address`); `.toString()` is the only op the render performs on them.
const A = (s: string) => s as unknown as Market["proposer"];

const ORACLE = "Orac1e1111111111111111111111111111111111111";
const PROPOSER = "Propo5er111111111111111111111111111111111111";
const PROPOSER_AUTH = "Auth0r1ty11111111111111111111111111111111111";

function mockMarket(): Market {
  return {
    accountType: 6,
    oracle: A(ORACLE),
    aiClaim: A("AiC1aim1111111111111111111111111111111111111"),
    proposer: A(PROPOSER),
    challenger: A("Cha11enger11111111111111111111111111111111111"),
    question: A("Quest10n111111111111111111111111111111111111"),
    kassVault: A("KassVau1t11111111111111111111111111111111111"),
    usdcVault: A("UsdcVau1t11111111111111111111111111111111111"),
    passAmm: A("PassAmm1111111111111111111111111111111111111"),
    failAmm: A("Fai1Amm1111111111111111111111111111111111111"),
    oraclePassKass: A("OPassKass11111111111111111111111111111111111"),
    oracleFailKass: A("OFai1Kass11111111111111111111111111111111111"),
    challengerUsdcVault: A("Escrow111111111111111111111111111111111111111"),
    // twapEnd in the PAST → settle open.
    twapEnd: BigInt(Math.floor(Date.now() / 1000) - 600),
    challengerUsdc: 500_000n,
    settled: false,
    bump: 255,
  } as unknown as Market;
}

function mockOracle(): Oracle {
  return {
    accountType: 1,
    marketThresholdNum: 1n,
    marketThresholdDen: 10n,
    kassMint: A("Kass1111111111111111111111111111111111111111"),
    usdcMint: A("Usdc1111111111111111111111111111111111111111"),
  } as unknown as Oracle;
}

function mockProposers(): { pubkey: string; proposer: Proposer }[] {
  return [{ pubkey: PROPOSER, proposer: { authority: A(PROPOSER_AUTH) } as unknown as Proposer }];
}

function walletValue(): WalletContextState {
  return {
    autoConnect: false,
    wallets: [],
    wallet: null,
    publicKey: null,
    connecting: false,
    connected: true,
    disconnecting: false,
    select: () => {},
    connect: async () => {},
    disconnect: async () => {},
    sendTransaction: async () => "sig",
    signTransaction: undefined,
    signAllTransactions: undefined,
    signMessage: undefined,
    signIn: undefined,
  } as unknown as WalletContextState;
}

function withProviders(children: ReactNode): string {
  // A stub connection with getSlot (the crank-hint effect probes it; SSR skips
  // effects anyway). `useWriteAction` swaps in the mock connection under mock.
  const connection = { getSlot: async () => 0 } as unknown as never;
  const cluster = {
    cluster: "localnet" as const,
    endpoint: "http://127.0.0.1:8899",
    setCluster: () => {},
    clusters: ["localnet"] as const,
  };
  return renderToStaticMarkup(
    <StrictMode>
      <ClusterContext.Provider value={cluster}>
        <ConnectionContext.Provider value={{ connection }}>
          <WalletContext.Provider value={walletValue()}>{children}</WalletContext.Provider>
        </ConnectionContext.Provider>
      </ClusterContext.Provider>
    </StrictMode>,
  );
}

/** The trade/crank/one-click-settle surface (challenge-market card). */
function renderTradeControls(): string {
  return withProviders(
    <ChallengeTradeControls
      oraclePubkey={ORACLE}
      oracle={mockOracle()}
      market={mockMarket()}
      proposers={mockProposers()}
      refetch={() => {}}
    />,
  );
}

/**
 * BOTH challenge surfaces the detail page mounts in the Challenge phase, rendered
 * together (as the page does) — proves there is exactly ONE settle surface across
 * ChallengeControl + ChallengeTradeControls and no leftover JSON paste.
 */
function renderBothSurfaces(): string {
  const market = { pubkey: "Mkt", market: mockMarket() };
  return withProviders(
    <>
      <ChallengeControl pubkey={ORACLE} oracle={mockOracle()} market={market} refetch={() => {}} />
      <ChallengeTradeControls
        oraclePubkey={ORACLE}
        oracle={mockOracle()}
        market={mockMarket()}
        proposers={mockProposers()}
        refetch={() => {}}
      />
    </>,
  );
}

describe("SD1 one-click settle control (headless render)", () => {
  const html = renderTradeControls();

  it("renders a one-click Settle button (settle open) with no JSON textarea", () => {
    expect(html).toContain("Settle challenge");
    // The old JSON-paste affordance is gone.
    expect(html).not.toContain("<textarea");
    expect(html).not.toContain("composed account payload");
    expect(html).not.toContain("the runner emits it");
  });

  it("keeps the verdict-preview surface + the derived-account copy", () => {
    expect(html).toContain("Pass TWAP");
    expect(html).toContain("Fail TWAP");
    expect(html).toContain("derived from the market");
  });
});

describe("SD1 single settle surface across the detail's challenge UI (headless render)", () => {
  const both = renderBothSurfaces();

  it("has EXACTLY ONE 'Settle challenge' submit button (no duplicate settle surface)", () => {
    // Count the settle SUBMIT BUTTONS (the label sits in a `>Settle challenge
    // </button>` close tag). Exactly one across BOTH surfaces — the old
    // JSON-paste settle button in ChallengeControl is gone.
    const settleButtons = both.match(/>Settle challenge<\/button>/g) ?? [];
    expect(settleButtons).toHaveLength(1);
    // The leftover ChallengeControl paste copy + a settle textarea must be absent.
    expect(both).not.toContain("takes the composed account set as a pasted");
    expect(both).not.toContain("composed account payload");
  });

  it("has NO <textarea> anywhere in the combined challenge UI", () => {
    expect(both).not.toContain("<textarea");
  });

  it("still exposes the one-click Settle button", () => {
    expect(both).toContain("Settle challenge");
    expect(both).toContain("derived from the market");
  });
});

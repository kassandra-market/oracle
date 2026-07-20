/**
 * Regression: the real-wallet provider must expose `signTransaction`.
 *
 * The merged markets write path (`src/market/data/send.ts` → `signAndRelay`) signs
 * a legacy transaction LOCALLY and relays it through the indexer — it needs
 * `WalletContextState.signTransaction`. `StandardWalletProvider` previously
 * implemented only `sendTransaction` (the oracle RPC send path), leaving
 * `signTransaction: undefined`. So the markets `useWriteAction` gate
 * (`if (!connected || !publicKey || !signTransaction) return null`) collapsed to a
 * null sender even with a connected wallet, and every trade fell through to the
 * misleading "Connect a wallet to participate." error.
 *
 * `useWallets` / `getWalletAccountFeature` are mocked so the provider renders with
 * no real browser wallet; we only assert the SHAPE of the `WalletContextState` it
 * publishes to consumers (SSR render, no effects needed).
 */
import { vi } from "vitest";

vi.mock("@wallet-standard/react", () => ({
  useWallets: () => [],
  getWalletAccountFeature: () => ({
    signTransaction: async (...inputs: unknown[]) =>
      inputs.map(() => ({ signedTransaction: new Uint8Array() })),
  }),
}));

import React, { StrictMode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { useWallet } from "@solana/wallet-adapter-react";
import { describe, expect, it } from "vitest";

import { StandardWalletProvider } from "../src/lib/standardWallet.tsx";

/** Reports which signing capabilities the provider hands to `useWallet()` consumers. */
function Probe() {
  const { signTransaction, sendTransaction, signAllTransactions } = useWallet();
  return (
    <>
      <span>{typeof signTransaction === "function" ? "HAS_SIGN" : "NO_SIGN"}</span>
      <span>{typeof sendTransaction === "function" ? "HAS_SEND" : "NO_SEND"}</span>
      <span>{typeof signAllTransactions === "function" ? "HAS_SIGN_ALL" : "NO_SIGN_ALL"}</span>
    </>
  );
}

describe("StandardWalletProvider — capabilities exposed to consumers", () => {
  const html = renderToStaticMarkup(
    <StrictMode>
      <StandardWalletProvider>
        <Probe />
      </StandardWalletProvider>
    </StrictMode>,
  );

  it("exposes signTransaction (the markets write path relays a locally-signed tx)", () => {
    expect(html).toContain("HAS_SIGN");
  });

  it("still exposes sendTransaction (the oracle RPC write path)", () => {
    expect(html).toContain("HAS_SEND");
  });

  it("exposes signAllTransactions (batch-sign path — one wallet approval for several txs)", () => {
    expect(html).toContain("HAS_SIGN_ALL");
  });
});

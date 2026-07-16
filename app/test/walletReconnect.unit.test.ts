/**
 * Unit coverage for the wallet auto-reconnect decision (`resolveReconnect`) — the
 * pure core behind restoring the last-used wallet on refresh. Kept DOM-free so it
 * runs in the node test env like the rest of the suite.
 */
import { describe, expect, it } from "vitest";

import { resolveReconnect } from "../src/lib/standardWallet";

// Minimal UiWallet/UiWalletAccount shapes — resolveReconnect only reads name +
// accounts[].address.
function wallet(name: string, addresses: string[]) {
  return { name, accounts: addresses.map((address) => ({ address })) } as never;
}

const STORED = { name: "Phantom", address: "Addr1111" };

describe("resolveReconnect", () => {
  it("does nothing without a remembered wallet", () => {
    expect(resolveReconnect(null, [wallet("Phantom", ["Addr1111"])], false)).toEqual({
      kind: "none",
    });
  });

  it("does nothing until the remembered wallet has registered", () => {
    expect(resolveReconnect(STORED, [wallet("Solflare", ["X"])], false)).toEqual({ kind: "none" });
  });

  it("adopts the remembered account when the wallet already exposes it", () => {
    const w = wallet("Phantom", ["Other", "Addr1111"]);
    const action = resolveReconnect(STORED, [w], false);
    expect(action.kind).toBe("adopt");
    expect(action.kind === "adopt" && action.account.address).toBe("Addr1111");
  });

  it("requests a single silent connect when the wallet is present but accountless", () => {
    const w = wallet("Phantom", []);
    const action = resolveReconnect(STORED, [w], false);
    expect(action.kind).toBe("silent");
    expect(action.kind === "silent" && action.wallet).toBe(w);
  });

  it("after a silent attempt, adopts the first account the wallet now authorizes", () => {
    // Same address missing (user switched accounts), but the wallet reconnected one.
    const action = resolveReconnect(STORED, [wallet("Phantom", ["Switched"])], true);
    expect(action.kind).toBe("adopt");
    expect(action.kind === "adopt" && action.account.address).toBe("Switched");
  });

  it("stays disconnected if the silent attempt yielded no accounts", () => {
    expect(resolveReconnect(STORED, [wallet("Phantom", [])], true)).toEqual({ kind: "none" });
  });
});

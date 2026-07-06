/**
 * Offline unit tests for the MetaDAO account readers (byte-offset decoders).
 */
import { describe, expect, it } from "vitest";

import {
  AMM_ACCOUNT_DISCRIMINATOR,
  AMM_BASE_AMOUNT_OFFSET,
  AMM_QUOTE_AMOUNT_OFFSET,
  decodeAmmReserves,
} from "../src/metadao/index.js";

/** Build a minimal valid `Amm` buffer with the given reserves. */
function ammBuffer(base: bigint, quote: bigint, disc = AMM_ACCOUNT_DISCRIMINATOR): Uint8Array {
  const data = new Uint8Array(AMM_QUOTE_AMOUNT_OFFSET + 8);
  data.set(disc, 0);
  const dv = new DataView(data.buffer);
  dv.setBigUint64(AMM_BASE_AMOUNT_OFFSET, base, true);
  dv.setBigUint64(AMM_QUOTE_AMOUNT_OFFSET, quote, true);
  return data;
}

describe("metadao/decodeAmmReserves", () => {
  it("reads base@115 / quote@123 (LE u64) from a valid Amm buffer", () => {
    const reserves = decodeAmmReserves(ammBuffer(123_456n, 789_012n));
    expect(reserves).toEqual({ base: 123_456n, quote: 789_012n });
  });

  it("returns null on a wrong account discriminator", () => {
    const bad = ammBuffer(1n, 2n, Uint8Array.of(0, 1, 2, 3, 4, 5, 6, 7));
    expect(decodeAmmReserves(bad)).toBeNull();
  });

  it("returns null on a too-short buffer", () => {
    const short = ammBuffer(1n, 2n).slice(0, AMM_QUOTE_AMOUNT_OFFSET + 7);
    expect(decodeAmmReserves(short)).toBeNull();
  });
});

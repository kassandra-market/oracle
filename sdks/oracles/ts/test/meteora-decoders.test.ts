/**
 * M1 (cont.) — Meteora DAMM v2 decoder round-trips. Split out of meteora.test.ts.
 *
 * The decoders round-trip a hand-built Pool/Position byte blob, asserting
 * sqrt_price/liquidity/reserves land at the computed offsets and the account
 * size == 8 + INIT_SPACE. Shared fixtures live in ./helpers/meteora.ts.
 * Offline (default suite).
 */
import { Address } from "@solana/web3.js";
import { describe, expect, it } from "vitest";

import { meteora } from "../src/index.js";
import {
  CONFIG,
  CREATOR,
  MINT_A,
  MINT_B,
  NFT_MINT,
  PAYER_TOKEN_A,
  PAYER_TOKEN_B,
  POOL_ACCOUNT_DISCRIMINATOR,
  POOL_ACCOUNT_SIZE,
  POSITION_ACCOUNT_DISCRIMINATOR,
  POSITION_ACCOUNT_SIZE,
} from "./helpers/meteora.js";

const setU64 = (buf: Uint8Array, off: number, v: bigint) => new DataView(buf.buffer).setBigUint64(off, v, true);
const setU128 = (buf: Uint8Array, off: number, v: bigint) => {
  const dv = new DataView(buf.buffer);
  dv.setBigUint64(off, v & 0xffffffffffffffffn, true);
  dv.setBigUint64(off + 8, v >> 64n, true);
};
const setPubkey = (buf: Uint8Array, off: number, a: string) => buf.set(new Address(a).toBytes(), off);

describe("decodePool", () => {
  it("reads mints/vaults/liquidity/sqrt_price(@456)/reserves at the computed offsets", () => {
    const buf = new Uint8Array(POOL_ACCOUNT_SIZE);
    buf.set(POOL_ACCOUNT_DISCRIMINATOR, 0);
    setPubkey(buf, 168, MINT_A);
    setPubkey(buf, 200, MINT_B);
    setPubkey(buf, 232, PAYER_TOKEN_A); // stand-in vault A
    setPubkey(buf, 264, PAYER_TOKEN_B); // stand-in vault B
    setU128(buf, 360, 42_000_000_000_000n); // liquidity
    setU64(buf, 392, 111n); // protocol_a_fee
    setU64(buf, 400, 222n); // protocol_b_fee
    setU128(buf, 424, 1000n); // sqrt_min_price
    setU128(buf, 440, 9_000_000n); // sqrt_max_price
    setU128(buf, 456, 18446744073709551616n); // sqrt_price = 1.0 Q64.64
    setPubkey(buf, 648, CREATOR);
    setU64(buf, 680, 5_000_000n); // token_a_amount
    setU64(buf, 688, 7_000_000n); // token_b_amount

    const pool = meteora.decodePool(buf);
    expect(pool.tokenAMint.toString()).toBe(MINT_A);
    expect(pool.tokenBMint.toString()).toBe(MINT_B);
    expect(pool.tokenAVault.toString()).toBe(PAYER_TOKEN_A);
    expect(pool.tokenBVault.toString()).toBe(PAYER_TOKEN_B);
    expect(pool.liquidity).toBe(42_000_000_000_000n);
    expect(pool.protocolAFee).toBe(111n);
    expect(pool.protocolBFee).toBe(222n);
    expect(pool.sqrtMinPrice).toBe(1000n);
    expect(pool.sqrtMaxPrice).toBe(9_000_000n);
    expect(pool.sqrtPrice).toBe(18446744073709551616n);
    expect(pool.creator.toString()).toBe(CREATOR);
    expect(pool.tokenAAmount).toBe(5_000_000n);
    expect(pool.tokenBAmount).toBe(7_000_000n);
  });

  it("rejects a wrong size or discriminator", () => {
    expect(() => meteora.decodePool(new Uint8Array(POOL_ACCOUNT_SIZE - 1))).toThrow(/wrong account size/);
    const bad = new Uint8Array(POOL_ACCOUNT_SIZE); // all-zero disc
    expect(() => meteora.decodePool(bad)).toThrow(/discriminator/);
  });
});

describe("decodePosition", () => {
  it("reads pool/nft_mint/fees/liquidity at the computed offsets", () => {
    const buf = new Uint8Array(POSITION_ACCOUNT_SIZE);
    buf.set(POSITION_ACCOUNT_DISCRIMINATOR, 0);
    setPubkey(buf, 8, CONFIG); // stand-in pool
    setPubkey(buf, 40, NFT_MINT);
    setU64(buf, 136, 314n); // fee_a_pending
    setU64(buf, 144, 271n); // fee_b_pending
    setU128(buf, 152, 12_345n); // unlocked_liquidity
    setU128(buf, 168, 6_789n); // vested_liquidity
    setU128(buf, 184, 42n); // permanent_locked_liquidity

    const pos = meteora.decodePosition(buf);
    expect(pos.pool.toString()).toBe(CONFIG);
    expect(pos.nftMint.toString()).toBe(NFT_MINT);
    expect(pos.feeAPending).toBe(314n);
    expect(pos.feeBPending).toBe(271n);
    expect(pos.unlockedLiquidity).toBe(12_345n);
    expect(pos.vestedLiquidity).toBe(6_789n);
    expect(pos.permanentLockedLiquidity).toBe(42n);
  });

  it("rejects a wrong size", () => {
    expect(() => meteora.decodePosition(new Uint8Array(10))).toThrow(/wrong account size/);
  });
});

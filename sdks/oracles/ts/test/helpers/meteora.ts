/**
 * Shared fixtures + byte/meta helpers for the Meteora DAMM v2 tests.
 * Split out of meteora.test.ts (see meteora.test.ts / meteora-decoders.test.ts).
 */
import { Address } from "@solana/web3.js";
import { expect } from "vitest";

import { meteora } from "../../src/index.js";

export const {
  DISC,
  METEORA_DAMM_V2_ID,
  TOKEN_2022_PROGRAM_ID,
  SEED,
  POOL_ACCOUNT_DISCRIMINATOR,
  POSITION_ACCOUNT_DISCRIMINATOR,
  POOL_ACCOUNT_SIZE,
  POSITION_ACCOUNT_SIZE,
  POOL_INIT_SPACE,
  POSITION_INIT_SPACE,
  pda,
} = meteora;

// Deterministic valid base58 stand-ins.
export const CREATOR = "rqRMW2HFJsi1FE1jb8Rvaz4Qz3xHzNkZDb8am1pqEHE";
export const PAYER = "rqRMW2HFJsi1FE1jb8Rvaz4Qz3xHzNkZDb8am1pqEHE";
export const CONFIG = "6iQKfEyhr3bZMotVkW6beNZz5CPAkiwvgV2CTje9pVSS";
export const MINT_A = "So11111111111111111111111111111111111111112";
export const MINT_B = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
export const NFT_MINT = "8HoQnePLqPj4M7PUDzfw8e3Ymdwgc7NLGnaTUapubyvu";
export const PAYER_TOKEN_A = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
export const PAYER_TOKEN_B = "3n5xdpW6Mx3wgxxBjTA1eXWZ7ByWWKvqSjzHRfM1Y8dY";

export const hex = (b: Uint8Array) => Buffer.from(b).toString("hex");
export const u64 = (v: bigint) => {
  const o = new Uint8Array(8);
  new DataView(o.buffer).setBigUint64(0, v, true);
  return o;
};
export const u128 = (v: bigint) => {
  const o = new Uint8Array(16);
  const dv = new DataView(o.buffer);
  dv.setBigUint64(0, v & 0xffffffffffffffffn, true);
  dv.setBigUint64(8, v >> 64n, true);
  return o;
};
export const cat = (...ps: Uint8Array[]) => {
  const out = new Uint8Array(ps.reduce((n, p) => n + p.length, 0));
  let o = 0;
  for (const p of ps) {
    out.set(p, o);
    o += p.length;
  }
  return out;
};

export type Meta = { pubkey: Address; isSigner: boolean; isWritable: boolean };
export const w = (p: Address | string, s = false): Meta => ({ pubkey: new Address(p as string), isSigner: s, isWritable: true });
export const ro = (p: Address | string, s = false): Meta => ({ pubkey: new Address(p as string), isSigner: s, isWritable: false });
export const metasEq = (got: readonly Meta[], want: Meta[]) => {
  expect(got.length).toBe(want.length);
  got.forEach((m, i) => {
    expect(m.pubkey.toString()).toBe(want[i].pubkey.toString());
    expect(m.isSigner).toBe(want[i].isSigner);
    expect(m.isWritable).toBe(want[i].isWritable);
  });
};

export const poolAddr = async () => (await pda.pool(CONFIG, MINT_A, MINT_B)).address;

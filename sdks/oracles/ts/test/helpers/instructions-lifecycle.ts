/**
 * Shared fixtures + byte helpers for the D3a instruction-builder tests.
 * Split out of instructions-lifecycle.test.ts.
 */
import { Address } from "@solana/web3.js";

import { Ix } from "../../src/constants.js";

export const PROGRAM_ID = "KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY";

// Deterministic stand-in keys (valid 32-byte base58 addresses).
export const ADMIN = "rqRMW2HFJsi1FE1jb8Rvaz4Qz3xHzNkZDb8am1pqEHE";
export const KASS_MINT = "So11111111111111111111111111111111111111112";
export const USDC_MINT = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
export const CREATOR = "84yVtdReAJ8GiR7Erqj7jyxoJurYWzQ6n9eaBGYBDNqM";
export const CREATOR_KASS = "7WCvk98KGRqi2o8D7EWTGrZQuFtikidP8A2D7CDVXwWJ";
export const AUTHORITY = "7bQEwuq9ybNyjjFcbtHBfDPxdH3TuGAsZKVRZdihVN4d";
export const AUTHORITY_KASS = "EScpWtUwYodKnbZx46YYeJbp2Ci2EpqcLAkF2EdZnZrh";
export const ORACLE = "GuBhyNi5GFo9K5YXGKfPMDryWK8GwS5oXe9CJGrzo2sk";
export const KASS_DAO = "FYQFL976rxQv8hygbC1zPVZYMfbnQkVntriESv69KaED";

/** Build the same [disc, ...payload] buffer independently for cross-checking. */
export function bytesOf(disc: Ix, payload: number[] = []): Uint8Array {
  return new Uint8Array([disc, ...payload]);
}

export function leU64(v: bigint): number[] {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigUint64(0, v, true);
  return Array.from(b);
}

export function leI64(v: bigint): number[] {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigInt64(0, v, true);
  return Array.from(b);
}

export function metaTriples(keys: { pubkey: Address; isSigner: boolean; isWritable: boolean }[]) {
  return keys.map((k) => [k.pubkey.toString(), k.isSigner, k.isWritable] as const);
}

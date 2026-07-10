/**
 * Shared fixtures for the builder + decoder unit tests.
 *
 * Extracted verbatim from the original `builders.test.ts` so the instruction
 * builder suite and the decoder suite can share the deterministic placeholder
 * addresses and the meta flag-string helpers without drift.
 */
import { Address } from "@solana/web3.js";
import type { AccountMeta } from "@solana/web3.js";

// Deterministic non-PDA test addresses.
export const A = (n: number): Address => new Address(new Uint8Array(32).fill(n));
export const PAYER = A(1);
export const KASS_MINT = A(2);
export const AUTHORITY = A(3);
export const ORACLE = A(4);
export const CREATOR = A(5);
export const CREATOR_ATA = A(6);
export const CONTRIBUTOR = A(7);
export const CONTRIB_ATA = A(8);
export const LP_ATA = A(9);
export const QUESTION = A(10);
export const FEE_DEST = A(11);

/** Compact "S"/"W"/"w"/"r" flag string per key, in order. */
export function flags(keys: ReadonlyArray<AccountMeta>): string {
  return keys
    .map((k) => (k.isSigner ? (k.isWritable ? "S" : "s") : k.isWritable ? "W" : "r"))
    .join("");
}
export const b58 = (a: Address): string => a.toString();
export const addrsOf = (keys: ReadonlyArray<AccountMeta>): string[] => keys.map((k) => b58(k.pubkey));

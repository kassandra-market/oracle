/**
 * Shared fixtures + byte helpers for the D3b dispute/challenge/settlement
 * builder tests. Split out of instructions-dispute.test.ts.
 */
import { Address } from "@solana/web3.js";

import { Ix } from "../../src/constants.js";

// Deterministic stand-in keys (valid 32-byte base58 addresses).
export const ORACLE = "GuBhyNi5GFo9K5YXGKfPMDryWK8GwS5oXe9CJGrzo2sk";
export const KASS_MINT = "So11111111111111111111111111111111111111112";
export const USDC_MINT = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
export const AUTHORITY = "7bQEwuq9ybNyjjFcbtHBfDPxdH3TuGAsZKVRZdihVN4d";
export const AUTHORITY_KASS = "EScpWtUwYodKnbZx46YYeJbp2Ci2EpqcLAkF2EdZnZrh";
export const PROPOSER = "84yVtdReAJ8GiR7Erqj7jyxoJurYWzQ6n9eaBGYBDNqM";
export const FACT = "FYQFL976rxQv8hygbC1zPVZYMfbnQkVntriESv69KaED";
export const FACT_VOTE = "7WCvk98KGRqi2o8D7EWTGrZQuFtikidP8A2D7CDVXwWJ";
export const CHALLENGER = "rqRMW2HFJsi1FE1jb8Rvaz4Qz3xHzNkZDb8am1pqEHE";
export const DEST_KASS = "9wFFyRfZBsuAha4YcuxcXLKwMxJR43S7fPfQLusDBzvT";
export const RENT_RECIPIENT = "DjVE6JNiYqPL2QXyCUUh8rNjHrbz9hXHNYt99MQ59qw1";
// MetaDAO / external accounts (caller-composed in the real flow).
export const QUESTION = "Gdnq3GYwQK9wMcZ4tNJjJfQbjPR55Mz6Mw59HCWMy2ER";
export const KASS_VAULT = "AeyTjbHr7yEZQ2KZX26ZbVZ4kgYFp5pZ5HfPwT5hLuMz";
export const USDC_VAULT = "HxhWj4WSvm2Qw4bA8K1xRZH7AcsmZ9c7q3bdsoFiY3Cd";
export const PASS_AMM = "5xUNJK9MZJtoSDc1nXFvFvgQ9hpqfHRZdLkVXCWfd9hM";
export const FAIL_AMM = "Cw4Hcuv7Bs4qB1tQR1Z9D6vWG2sCcTwbQGm7Yqsnz3uG";
export const KASS_VAULT_UNDERLYING = "8KhywBoQbBxAdtdAa3hKzZ4u3F8s5cQ7p1Tym9SHnpZ6";
export const PASS_KASS_MINT = "2tFsVQ9hyLT5VuQ7zZ8Zc4PWb5y5GFwXk5KQ6KkKAuW7";
export const FAIL_KASS_MINT = "3vQ8H7m6CqMQy7vQ9hT9b8mGwgKqpRtN5sH9JLmZTpqL";
export const ORACLE_PASS_KASS = "6yLh8Y9bMTcLW2qXg5e8YZ4cWk5GFp5pZ5HfPwT5jKvN";
export const ORACLE_FAIL_KASS = "4qB1tQR1Z9D6vWG2sCcTwbQGm7Yqsnz3uGCw4Hcuv7Bs";
export const CV_EVENT_AUTH = "7p1Tym9SHnpZ68KhywBoQbBxAdtdAa3hKzZ4u3F8s5cQ";
export const KASS_DAO = "B5y5GFwXk5KQ6KkKAuW72tFsVQ9hyLT5VuQ7zZ8Zc4PW";
export const CHALLENGER_USDC_SRC = "C7zZ8Zc4PWb5y5GFwXk5KQ6KkKAuW72tFsVQ9hyLT5Vu";
export const AI_CLAIM = "DqpRtN5sH9JLmZTpqL3vQ8H7m6CqMQy7vQ9hT9b8mGwg";
export const PROPOSER_USDC = "EW72tFsVQ9hyLT5VuQ7zZ8Zc4PWb5y5GFwXk5KQ6KkKA";
export const CHALLENGER_USDC_DEST = "FuQ7zZ8Zc4PWb5y5GFwXk5KQ6KkKAuW72tFsVQ9hyLT5";
export const CHALLENGER_KASS = "GFwXk5KQ6KkKAuW72tFsVQ9hyLT5VuQ7zZ8Zc4PWb5y5";
export const MARKET_ARG = "HkKAuW72tFsVQ9hyLT5VuQ7zZ8Zc4PWb5y5GFwXk5KQ6";

export function bytesOf(disc: Ix, payload: number[] = []): Uint8Array {
  return new Uint8Array([disc, ...payload]);
}

export function leU64(v: bigint): number[] {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigUint64(0, v, true);
  return Array.from(b);
}

export function leU16(v: number): number[] {
  const b = new Uint8Array(2);
  new DataView(b.buffer).setUint16(0, v, true);
  return Array.from(b);
}

export function metaTriples(keys: { pubkey: Address; isSigner: boolean; isWritable: boolean }[]) {
  return keys.map((k) => [k.pubkey.toString(), k.isSigner, k.isWritable] as const);
}

export const enc = new TextEncoder();

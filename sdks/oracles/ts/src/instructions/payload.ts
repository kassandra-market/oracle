/**
 * Little-endian payload-byte helpers for the instruction builders.
 *
 * Every Kassandra instruction's `data` is `[disc_byte, ...payload]` where the
 * payload mirrors the processor's exact byte layout (all integers
 * little-endian, pubkeys as their 32 raw bytes). These helpers each return a
 * `Uint8Array` chunk; {@link concatBytes} joins them, and {@link withDisc}
 * prepends the 1-byte discriminant. The encodings match the `*_at` / `to_le_bytes`
 * reads in the Rust processors.
 */
import { Address } from "@solana/web3.js";
import type { AccountMeta } from "@solana/web3.js";

import { Ix } from "../constants.js";
import type { AddressInput } from "../pda.js";

/** Coerce an `AddressInput` into a web3.js `Address`. */
export function addr(a: AddressInput): Address {
  return a instanceof Address ? a : new Address(a);
}

/** Writable account meta. */
export function w(pubkey: Address, isSigner = false): AccountMeta {
  return { pubkey, isSigner, isWritable: true };
}

/** Read-only account meta. */
export function ro(pubkey: Address, isSigner = false): AccountMeta {
  return { pubkey, isSigner, isWritable: false };
}

/** A single unsigned byte (`u8`). */
export function u8(value: number): Uint8Array {
  return new Uint8Array([value & 0xff]);
}

/** A little-endian `u16` (2 bytes). */
export function u16LE(value: number): Uint8Array {
  const out = new Uint8Array(2);
  new DataView(out.buffer).setUint16(0, value, true);
  return out;
}

/** A little-endian `u64` (8 bytes) from a bigint/number. */
export function u64LE(value: bigint | number): Uint8Array {
  const out = new Uint8Array(8);
  new DataView(out.buffer).setBigUint64(0, BigInt(value), true);
  return out;
}

/** A little-endian signed `i64` (8 bytes) from a bigint/number. */
export function i64LE(value: bigint | number): Uint8Array {
  const out = new Uint8Array(8);
  new DataView(out.buffer).setBigInt64(0, BigInt(value), true);
  return out;
}

/** The 32 raw bytes of a pubkey (the on-wire form of an `[u8; 32]` payload field). */
export function pubkeyBytes(value: AddressInput): Uint8Array {
  return (value instanceof Address ? value : new Address(value)).toBytes();
}

/** A fixed-length `[u8; len]` field; throws if `bytes` is the wrong length. */
export function fixedBytes(bytes: Uint8Array, len: number): Uint8Array {
  if (bytes.length !== len) {
    throw new Error(`expected exactly ${len} bytes, got ${bytes.length}`);
  }
  return bytes;
}

/** Concatenate payload chunks into one buffer. */
export function concatBytes(parts: Array<Uint8Array>): Uint8Array {
  const total = parts.reduce((n, p) => n + p.length, 0);
  const out = new Uint8Array(total);
  let off = 0;
  for (const p of parts) {
    out.set(p, off);
    off += p.length;
  }
  return out;
}

/** Build instruction `data` = `[disc, ...payload]`. */
export function withDisc(disc: Ix, ...payload: Array<Uint8Array>): Uint8Array {
  return concatBytes([u8(disc), ...payload]);
}

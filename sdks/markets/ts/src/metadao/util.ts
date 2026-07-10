/**
 * Shared meta + borsh helpers for the MetaDAO `amm` / `conditional_vault` wire
 * builders. Both builders emit the same account-meta shapes (writable /
 * read-only) and little-endian borsh scalars, so the encodings live here once.
 * The address coercion is the canonical {@link addr} from the instruction
 * builders (a single implementation across the SDK).
 */
import type { AccountMeta } from "@solana/web3.js";

import { addr } from "../instructions/payload.js";
import type { AddressInput } from "../pda.js";

export { addr };

/** Writable account meta (coerces `pubkey` to an `Address`). */
export function w(pubkey: AddressInput, isSigner = false): AccountMeta {
  return { pubkey: addr(pubkey), isSigner, isWritable: true };
}

/** Read-only account meta (coerces `pubkey` to an `Address`). */
export function ro(pubkey: AddressInput, isSigner = false): AccountMeta {
  return { pubkey: addr(pubkey), isSigner, isWritable: false };
}

/** A single unsigned byte (`u8`). */
export function u8b(v: number): Uint8Array {
  return Uint8Array.from([v & 0xff]);
}

/** A little-endian `u32` (4 bytes). */
export function u32le(v: number): Uint8Array {
  const o = new Uint8Array(4);
  new DataView(o.buffer).setUint32(0, v, true);
  return o;
}

/** A little-endian `u64` (8 bytes). */
export function u64le(v: bigint | number): Uint8Array {
  const o = new Uint8Array(8);
  new DataView(o.buffer).setBigUint64(0, BigInt(v), true);
  return o;
}

/** A little-endian `u128` (16 bytes). */
export function u128le(v: bigint | number): Uint8Array {
  const o = new Uint8Array(16);
  const dv = new DataView(o.buffer);
  const x = BigInt(v);
  dv.setBigUint64(0, x & 0xffffffffffffffffn, true);
  dv.setBigUint64(8, x >> 64n, true);
  return o;
}

/** Concatenate borsh chunks into one buffer. */
export function concat(parts: Array<Uint8Array>): Uint8Array {
  const total = parts.reduce((n, p) => n + p.length, 0);
  const out = new Uint8Array(total);
  let off = 0;
  for (const p of parts) {
    out.set(p, off);
    off += p.length;
  }
  return out;
}

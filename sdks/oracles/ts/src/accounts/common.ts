/**
 * Shared low-level readers for the Pod account decoders.
 *
 * Every Kassandra account is a `#[repr(C)]` `bytemuck::Pod` struct read straight
 * out of the account's raw bytes at FIXED, little-endian offsets (the offsets are
 * pinned in `programs/oracles/tests/state_layout.rs`). These helpers wrap a
 * `DataView` over the account data so each decoder reads a field by its exact
 * byte offset:
 *
 *   - `u64`/`i64`  → `bigint`   (`readU64LE` / `readI64LE`)
 *   - `u8`/`u16`/`u32` → `number`
 *   - `[u8;32]` pubkeys → web3.js `Address` (base58) via `readPubkey`
 *   - `[u8;N]` raw byte arrays → `Uint8Array` copy via `readBytes`
 *   - bool-ish `u8` flags → `boolean` via `readBool`
 *
 * Every account starts with `account_type: u8` @0 + `_pad_hdr: [u8;7]`, so real
 * fields begin at offset 8.
 */
import { Address } from "@solana/web3.js";

import { AccountType } from "../constants.js";

/** Offset of the `account_type` discriminator byte (first byte of every Pod account). */
export const ACCOUNT_TYPE_OFFSET = 0;

/** A `DataView` over an account's raw bytes, honoring its byteOffset/length. */
export function view(data: Uint8Array): DataView {
  return new DataView(data.buffer, data.byteOffset, data.byteLength);
}

/** Read an unsigned 8-bit integer at `offset`. */
export function readU8(dv: DataView, offset: number): number {
  return dv.getUint8(offset);
}

/** Read a little-endian unsigned 16-bit integer at `offset`. */
export function readU16LE(dv: DataView, offset: number): number {
  return dv.getUint16(offset, true);
}

/** Read a little-endian unsigned 32-bit integer at `offset`. */
export function readU32LE(dv: DataView, offset: number): number {
  return dv.getUint32(offset, true);
}

/** Read a little-endian unsigned 64-bit integer at `offset` as a `bigint`. */
export function readU64LE(dv: DataView, offset: number): bigint {
  return dv.getBigUint64(offset, true);
}

/** Read a little-endian signed 64-bit integer at `offset` as a `bigint`. */
export function readI64LE(dv: DataView, offset: number): bigint {
  return dv.getBigInt64(offset, true);
}

/** Read a little-endian unsigned 128-bit integer at `offset` as a `bigint` (low 64 ++ high 64). */
export function readU128LE(dv: DataView, offset: number): bigint {
  const lo = dv.getBigUint64(offset, true);
  const hi = dv.getBigUint64(offset + 8, true);
  return (hi << 64n) | lo;
}

/** Read a bool-ish `u8` flag at `offset` (`0` → false, anything else → true). */
export function readBool(dv: DataView, offset: number): boolean {
  return dv.getUint8(offset) !== 0;
}

/** Copy `len` raw bytes starting at `offset` (e.g. a `content_hash`/`prompt_hash`). */
export function readBytes(data: Uint8Array, offset: number, len: number): Uint8Array {
  return data.slice(offset, offset + len);
}

/** Read a 32-byte pubkey at `offset` and return it as a web3.js `Address` (base58). */
export function readPubkey(data: Uint8Array, offset: number): Address {
  return new Address(data.slice(offset, offset + 32));
}

/**
 * Validate an account buffer before decoding: it must be EXACTLY `size` bytes
 * (the pinned on-chain ABI size) and its first byte must be the expected
 * `account_type` tag. Rejecting a wrong tag prevents type-confusion — decoding a
 * `Fact` where an `Oracle` was expected throws instead of silently misreading.
 */
export function assertAccount(
  data: Uint8Array,
  expectedType: AccountType,
  size: number,
  name: string,
): void {
  if (data.length !== size) {
    throw new Error(
      `${name}: wrong account size — expected ${size} bytes, got ${data.length}.`,
    );
  }
  const tag = data[ACCOUNT_TYPE_OFFSET];
  if (tag !== expectedType) {
    throw new Error(
      `${name}: wrong account_type tag — expected ${expectedType} (${AccountType[expectedType]}), got ${tag} (${AccountType[tag as AccountType] ?? "unknown"}).`,
    );
  }
}

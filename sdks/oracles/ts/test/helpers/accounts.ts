/**
 * Shared fixtures + buffer writer for the Pod account decoder tests.
 * Split out of accounts.test.ts (see accounts.test.ts / accounts-litesvm.test.ts).
 */
import { Address } from "@solana/web3.js";

import { AccountType } from "../../src/constants.js";

export const TOKEN_PROGRAM_ID = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
export const SYSTEM_PROGRAM_ID = "11111111111111111111111111111111";
export const PROGRAM_ID = "KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY";

/** A small writer over a fixed-size buffer that mirrors the on-chain layout. */
export class Buf {
  readonly bytes: Uint8Array;
  private readonly dv: DataView;
  constructor(size: number, accountType: AccountType) {
    this.bytes = new Uint8Array(size);
    this.dv = new DataView(this.bytes.buffer);
    this.bytes[0] = accountType; // account_type @0 (+ _pad_hdr[7])
  }
  u8(offset: number, v: number): this {
    this.dv.setUint8(offset, v);
    return this;
  }
  u16(offset: number, v: number): this {
    this.dv.setUint16(offset, v, true);
    return this;
  }
  u64(offset: number, v: bigint): this {
    this.dv.setBigUint64(offset, v, true);
    return this;
  }
  i64(offset: number, v: bigint): this {
    this.dv.setBigInt64(offset, v, true);
    return this;
  }
  raw(offset: number, v: Uint8Array): this {
    this.bytes.set(v, offset);
    return this;
  }
}

/** A deterministic, distinct 32-byte "pubkey" seeded by `n`. */
export function key32(n: number): Uint8Array {
  const b = new Uint8Array(32);
  for (let i = 0; i < 32; i++) b[i] = (n * 31 + i * 7 + 1) & 0xff;
  return b;
}

/** base58 string an `Address` built from the same 32 bytes should produce. */
export function key32Addr(n: number): string {
  return new Address(key32(n)).toString();
}

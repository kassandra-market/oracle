/**
 * Unit tests for the low-level payload byte encoders in
 * `src/instructions/payload.ts`. These are exercised INDIRECTLY by the
 * instruction byte-parity tests, but their edge + ERROR paths (u8 masking, i64
 * negatives, `fixedBytes` length throw, empty concat) are not — pin them here.
 */
import { describe, expect, it } from "vitest";

import {
  concatBytes,
  fixedBytes,
  i64LE,
  u16LE,
  u64LE,
  u8,
} from "../src/instructions/payload.js";

describe("u8", () => {
  it("emits one byte, masking to the low 8 bits", () => {
    expect([...u8(0)]).toEqual([0]);
    expect([...u8(255)]).toEqual([255]);
    expect([...u8(256)]).toEqual([0]); // wraps
    expect([...u8(257)]).toEqual([1]);
  });
});

describe("u16LE / u64LE", () => {
  it("u16LE is little-endian", () => {
    expect([...u16LE(0x0102)]).toEqual([0x02, 0x01]);
    expect([...u16LE(0)]).toEqual([0, 0]);
    expect([...u16LE(65535)]).toEqual([0xff, 0xff]);
  });

  it("u64LE accepts bigint and number, little-endian, 8 bytes", () => {
    expect([...u64LE(0)]).toEqual([0, 0, 0, 0, 0, 0, 0, 0]);
    expect([...u64LE(1n)]).toEqual([1, 0, 0, 0, 0, 0, 0, 0]);
    expect([...u64LE(258)]).toEqual([2, 1, 0, 0, 0, 0, 0, 0]); // 0x0102
    expect([...u64LE(0xffffffffffffffffn)]).toEqual(Array(8).fill(0xff));
  });
});

describe("i64LE", () => {
  it("encodes signed values two's-complement little-endian", () => {
    expect([...i64LE(0)]).toEqual([0, 0, 0, 0, 0, 0, 0, 0]);
    expect([...i64LE(-1)]).toEqual(Array(8).fill(0xff));
    expect([...i64LE(-1n)]).toEqual(Array(8).fill(0xff));
    expect([...i64LE(1_234)]).toEqual([0xd2, 0x04, 0, 0, 0, 0, 0, 0]);
  });
});

describe("fixedBytes", () => {
  it("returns the buffer unchanged when the length matches", () => {
    const b = Uint8Array.from([1, 2, 3, 4]);
    expect(fixedBytes(b, 4)).toBe(b);
  });

  it("THROWS when the length is wrong (the error path builders never hit)", () => {
    expect(() => fixedBytes(new Uint8Array(31), 32)).toThrow(/expected exactly 32 bytes, got 31/);
    expect(() => fixedBytes(new Uint8Array(33), 32)).toThrow(/got 33/);
    expect(() => fixedBytes(new Uint8Array(0), 1)).toThrow(/got 0/);
  });
});

describe("concatBytes", () => {
  it("concatenates in order", () => {
    expect([...concatBytes([Uint8Array.from([1, 2]), Uint8Array.from([3]), Uint8Array.from([4, 5])])]).toEqual(
      [1, 2, 3, 4, 5],
    );
  });

  it("handles empty input and empty chunks", () => {
    expect([...concatBytes([])]).toEqual([]);
    expect([...concatBytes([new Uint8Array(0), Uint8Array.from([7]), new Uint8Array(0)])]).toEqual([7]);
  });
});

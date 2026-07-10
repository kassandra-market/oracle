/**
 * Offline unit tests for the oracle READ data layer (`src/data/oracles.ts`).
 *
 * These run against a MOCK {@link Connection} whose canned responses are built
 * from REAL Pod byte layouts — buffers of the pinned ABI size, with the
 * account_type tag at offset 0 and each field written at the exact little-endian
 * offset the SDK decoders read (mirroring `programs/oracles/tests/state_layout.rs`).
 * So the decoders exercise genuine byte shapes, not hand-waved objects.
 *
 * The mock models a PERMISSIVE RPC: it honours `memcmp` filters (so the
 * parent-oracle scoping in `fetchOracleDetail` is real) but NOT the `dataSize`
 * filter — which lets a wrong-size / wrong-tag blob reach the decoder, proving
 * the data layer's per-account try/catch SKIP guard (a bad blob is excluded, not
 * thrown). No network.
 */
import type { Connection } from "@solana/web3.js";
import { Address } from "@solana/web3.js";
import { ACCOUNT_SIZES, AccountType, Phase } from "@kassandra-market/oracles";
import { describe, expect, it } from "vitest";

import bs58 from "bs58";

import { fetchOracleDetail, fetchOracles, OracleNotFoundError } from "../src/data/oracles";

// --- real Pod byte-layout builders (tag @0, fields at their pinned offsets) --

/** Build an `Oracle` buffer (392 B) with a chosen deadline + phase. */
function encodeOracle(opts: { deadline: bigint; phase: Phase; options?: number }): Uint8Array {
  const buf = new Uint8Array(ACCOUNT_SIZES.Oracle);
  const dv = new DataView(buf.buffer);
  buf[0] = AccountType.Oracle; // account_type tag
  dv.setBigInt64(136, opts.deadline, true); // deadline: i64 @136
  buf[160] = opts.options ?? 2; // options_count: u8 @160
  buf[161] = opts.phase; // phase: u8 @161
  return buf;
}

/** Write a 32-byte oracle pubkey at the child `oracle` field (offset 8). */
function withOracle(buf: Uint8Array, oracle: Address): Uint8Array {
  buf.set(oracle.toBytes(), 8);
  return buf;
}

function encodeFact(oracle: Address): Uint8Array {
  const buf = new Uint8Array(ACCOUNT_SIZES.Fact);
  buf[0] = AccountType.Fact;
  return withOracle(buf, oracle); // uri_len @128 stays 0 → uri === ""
}

function encodeProposer(oracle: Address, option = 1): Uint8Array {
  const buf = new Uint8Array(ACCOUNT_SIZES.Proposer);
  buf[0] = AccountType.Proposer;
  withOracle(buf, oracle);
  buf[80] = option; // original_option: u8 @80
  buf[81] = 0xff; // claim_option: CLAIM_OPTION_NONE @81
  return buf;
}

function encodeAiClaim(oracle: Address, option = 1): Uint8Array {
  const buf = new Uint8Array(ACCOUNT_SIZES.AiClaim);
  buf[0] = AccountType.AiClaim;
  withOracle(buf, oracle);
  buf[168] = option; // option: u8 @168
  return buf;
}

function encodeMarket(oracle: Address): Uint8Array {
  const buf = new Uint8Array(ACCOUNT_SIZES.Market);
  buf[0] = AccountType.Market;
  return withOracle(buf, oracle);
}

// --- mock Connection ---------------------------------------------------------

interface StoredAccount {
  pubkey: string;
  data: Uint8Array;
}

/** True if `data` satisfies a getProgramAccounts memcmp filter (base58 bytes at offset). */
function matchesMemcmp(data: Uint8Array, offset: number, base58: string): boolean {
  const expected = bs58.decode(base58);
  if (offset + expected.length > data.length) return false;
  for (let i = 0; i < expected.length; i++) {
    if (data[offset + i] !== expected[i]) return false;
  }
  return true;
}

/**
 * A permissive mock RPC: honours memcmp filters (real parent-oracle scoping) but
 * ignores dataSize, so undecodable blobs reach the decoder and exercise the skip.
 */
function mockConnection(store: StoredAccount[]): Connection {
  const toAccount = (a: StoredAccount) => ({
    pubkey: new Address(a.pubkey),
    account: {
      data: a.data,
      executable: false,
      lamports: 1,
      owner: new Address("KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY"),
      rentEpoch: 0,
      space: BigInt(a.data.length),
    },
  });
  return {
    getProgramAccounts: async (
      _programId: Address,
      config?: { filters?: { memcmp?: { offset: number; bytes: string } }[] },
    ) => {
      const memcmps = (config?.filters ?? []).filter((f) => "memcmp" in f);
      return store
        .filter((a) =>
          memcmps.every((f) => matchesMemcmp(a.data, f.memcmp!.offset, f.memcmp!.bytes)),
        )
        .map(toAccount);
    },
    getAccountInfo: async (pubkey: Address) => {
      const found = store.find((a) => a.pubkey === pubkey.toString());
      return found ? toAccount(found).account : null;
    },
  } as unknown as Connection;
}

// A few stable, valid base58 pubkeys to use as account addresses.
const ORACLE_A = new Address("BPFLoader2111111111111111111111111111111111");
const ORACLE_B = new Address("Vote111111111111111111111111111111111111111");
const CHILD_1 = new Address("Stake11111111111111111111111111111111111111");
const CHILD_2 = new Address("Config1111111111111111111111111111111111111");
const CHILD_3 = new Address("SysvarC1ock11111111111111111111111111111111");
const CHILD_4 = new Address("SysvarRent111111111111111111111111111111111");

describe("account_type memcmp tag", () => {
  it("encodes the single account_type tag byte to its canonical base58 char", () => {
    // Byte value n (< 58) → the n-th base58 alphabet char. The getProgramAccounts
    // memcmp filter matches on this bs58-encoded tag.
    expect(bs58.encode(Uint8Array.of(AccountType.Oracle))).toBe("2"); // 1
    expect(bs58.encode(Uint8Array.of(AccountType.Fact))).toBe("4"); // 3
  });
});

describe("fetchOracles", () => {
  it("enumerates + decodes every oracle, sorted by deadline desc, skipping bad blobs", async () => {
    const conn = mockConnection([
      { pubkey: ORACLE_A.toString(), data: encodeOracle({ deadline: 1_000n, phase: Phase.Proposal }) },
      { pubkey: ORACLE_B.toString(), data: encodeOracle({ deadline: 5_000n, phase: Phase.Resolved }) },
      // A malformed blob: Oracle tag byte (passes the tag memcmp) but the wrong
      // size (300 != 392). It slips the mock's ignored dataSize filter and
      // reaches decodeOracle, whose size guard throws → the data layer SKIPS it.
      { pubkey: CHILD_1.toString(), data: (() => {
        const b = new Uint8Array(300);
        b[0] = AccountType.Oracle;
        return b;
      })() },
    ]);

    const oracles = await fetchOracles(conn);

    expect(oracles).toHaveLength(2); // the malformed 300-byte blob was skipped
    expect(oracles.map((o) => o.pubkey)).toEqual([ORACLE_B.toString(), ORACLE_A.toString()]); // 5000 before 1000
    expect(oracles[0].oracle.phase).toBe(Phase.Resolved);
    expect(oracles[0].oracle.deadline).toBe(5_000n);
    expect(oracles[1].oracle.phase).toBe(Phase.Proposal);
  });

  it("returns an empty array when the program owns no oracles", async () => {
    expect(await fetchOracles(mockConnection([]))).toEqual([]);
  });
});

describe("fetchOracleDetail", () => {
  it("assembles the oracle + its children, scoping children by the oracle memcmp", async () => {
    const conn = mockConnection([
      { pubkey: ORACLE_A.toString(), data: encodeOracle({ deadline: 2_000n, phase: Phase.AiClaim }) },
      // Children of ORACLE_A.
      { pubkey: CHILD_1.toString(), data: encodeFact(ORACLE_A) },
      { pubkey: CHILD_2.toString(), data: encodeProposer(ORACLE_A, 1) },
      { pubkey: CHILD_3.toString(), data: encodeAiClaim(ORACLE_A, 1) },
      { pubkey: CHILD_4.toString(), data: encodeMarket(ORACLE_A) },
      // A fact belonging to a DIFFERENT oracle — must be excluded by the memcmp.
      { pubkey: ORACLE_B.toString(), data: encodeFact(ORACLE_B) },
    ]);

    const detail = await fetchOracleDetail(conn, ORACLE_A.toString());

    expect(detail.pubkey).toBe(ORACLE_A.toString());
    expect(detail.oracle.phase).toBe(Phase.AiClaim);
    expect(detail.facts).toHaveLength(1);
    expect(detail.facts[0].pubkey).toBe(CHILD_1.toString());
    expect(detail.facts[0].fact.oracle.toString()).toBe(ORACLE_A.toString());
    expect(detail.proposers).toHaveLength(1);
    expect(detail.proposers[0].proposer.originalOption).toBe(1);
    expect(detail.aiClaims).toHaveLength(1);
    expect(detail.aiClaims[0].aiClaim.option).toBe(1);
    expect(detail.market).toBeDefined();
    expect(detail.market?.market.oracle.toString()).toBe(ORACLE_A.toString());
  });

  it("handles a missing market (undefined) and empty child sets", async () => {
    const conn = mockConnection([
      { pubkey: ORACLE_A.toString(), data: encodeOracle({ deadline: 1n, phase: Phase.Proposal }) },
    ]);
    const detail = await fetchOracleDetail(conn, ORACLE_A.toString());
    expect(detail.facts).toEqual([]);
    expect(detail.proposers).toEqual([]);
    expect(detail.aiClaims).toEqual([]);
    expect(detail.market).toBeUndefined();
  });

  it("throws OracleNotFoundError for an absent oracle account", async () => {
    const conn = mockConnection([]);
    await expect(fetchOracleDetail(conn, ORACLE_A.toString())).rejects.toBeInstanceOf(
      OracleNotFoundError,
    );
  });

  it("throws OracleNotFoundError when the account exists but is the wrong type", async () => {
    const conn = mockConnection([
      { pubkey: ORACLE_A.toString(), data: encodeFact(ORACLE_B) }, // a Fact, not an Oracle
    ]);
    await expect(fetchOracleDetail(conn, ORACLE_A.toString())).rejects.toBeInstanceOf(
      OracleNotFoundError,
    );
  });
});

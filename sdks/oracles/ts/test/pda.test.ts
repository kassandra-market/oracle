/**
 * PDA derivation unit tests — deterministic: known inputs -> stable addresses
 * + correct bumps + correct seed-byte encodings (esp. the oracle nonce as a
 * u64 little-endian 8-byte seed).
 *
 * The hardcoded base58 anchors are regression values: they pin the derivation
 * output so a change to a seed list / encoding is caught. They are produced by
 * the same `Address.findProgramAddress` that the live SDK uses (identical to
 * Solana's `Pubkey::find_program_address` the program + Rust harness use).
 */
import { Address } from "@solana/web3.js";
import { describe, expect, it } from "vitest";

import { KASSANDRA_PROGRAM_ID } from "../src/constants.js";
import * as pda from "../src/pda.js";

// A couple of stable, arbitrary addresses to use as pubkey seeds.
const ORACLE = new Address("FZeeaLKvq4Rz4ESQM7EXGMnhNQdB9sfM8db1r8XdBfbb");
const AUTHORITY = new Address("11111111111111111111111111111111");

describe("PDA: singletons", () => {
  it("protocol() is stable", async () => {
    const { address, bump } = await pda.protocol();
    expect(address.toString()).toBe("DUpkpXThaPjDS7TtwwdMJHam7Ki6a8Fg9bmvNf5ggMn6");
    expect(bump).toBe(255);
  });

  it("mintAuthority() is stable", async () => {
    const { address, bump } = await pda.mintAuthority();
    expect(address.toString()).toBe("CyZkoqGggvQFEnUQRmkMcGmm5kpKZy7jbZdUVuSnx5Hk");
    expect(bump).toBe(255);
  });
});

describe("PDA: oracle nonce is encoded as u64 little-endian", () => {
  it("nonce 1 derives a stable address", async () => {
    const { address, bump } = await pda.oracle(1);
    expect(address.toString()).toBe("FZeeaLKvq4Rz4ESQM7EXGMnhNQdB9sfM8db1r8XdBfbb");
    expect(bump).toBe(254);
  });

  it("nonce 256 derives a DIFFERENT, stable address (LE byte layout matters)", async () => {
    const { address, bump } = await pda.oracle(256);
    expect(address.toString()).toBe("CQ54BVydXfCQAT3k3sSkjVJZPvjNecW4wsvtoS8P4soo");
    expect(bump).toBe(253);
  });

  it("nonce 1 != nonce 256 (would collide under big-endian or truncation bugs)", async () => {
    const a = await pda.oracle(1);
    const b = await pda.oracle(256);
    expect(a.address.toString()).not.toBe(b.address.toString());
  });

  it("accepts bigint and number identically", async () => {
    const asNumber = await pda.oracle(1);
    const asBigint = await pda.oracle(1n);
    expect(asNumber.address.toString()).toBe(asBigint.address.toString());
  });

  it("the u64 nonce uses the full 8 bytes (large nonce derives without throwing)", async () => {
    const big = await pda.oracle(0xffffffffffffffffn);
    expect(big.address).toBeInstanceOf(Address);
  });
});

describe("PDA: derivations are deterministic", () => {
  it("same inputs -> same address (twice)", async () => {
    const a = await pda.proposer(ORACLE, AUTHORITY);
    const b = await pda.proposer(ORACLE, AUTHORITY);
    expect(a.address.toString()).toBe(b.address.toString());
    expect(a.bump).toBe(b.bump);
  });

  it("accepts base58 strings and Address objects identically", async () => {
    const fromString = await pda.stakeVault(ORACLE.toString());
    const fromAddress = await pda.stakeVault(ORACLE);
    expect(fromString.address.toString()).toBe(fromAddress.address.toString());
  });
});

describe("PDA: seed ORDER and IDENTITY matter (no accidental symmetry)", () => {
  it("proposer(oracle, authority) != proposer(authority, oracle)", async () => {
    const ab = await pda.proposer(ORACLE, AUTHORITY);
    const ba = await pda.proposer(AUTHORITY, ORACLE);
    expect(ab.address.toString()).not.toBe(ba.address.toString());
  });

  it("fact() respects the 32-byte content_hash seed", async () => {
    const h0 = new Uint8Array(32).fill(0);
    const h1 = new Uint8Array(32).fill(1);
    const a = await pda.fact(ORACLE, h0);
    const b = await pda.fact(ORACLE, h1);
    expect(a.address.toString()).not.toBe(b.address.toString());
  });

  it("all PDA kinds for the same pubkey seed differ (distinct literal seeds)", async () => {
    const aiClaimPda = await pda.aiClaim(ORACLE, AUTHORITY);
    const proposerPda = await pda.proposer(ORACLE, AUTHORITY);
    const factVotePda = await pda.factVote(ORACLE, AUTHORITY);
    const set = new Set([
      aiClaimPda.address.toString(),
      proposerPda.address.toString(),
      factVotePda.address.toString(),
    ]);
    expect(set.size).toBe(3);
  });
});

describe("PDA: market + escrow chain", () => {
  it("market(aiClaim) then challengeUsdcVault(market) derive distinct stable PDAs", async () => {
    const aiClaimPda = await pda.aiClaim(ORACLE, AUTHORITY);
    const marketPda = await pda.market(aiClaimPda.address);
    const escrowPda = await pda.challengeUsdcVault(marketPda.address);
    expect(marketPda.address.toString()).not.toBe(escrowPda.address.toString());
    // Determinism re-check through the chain.
    const marketPda2 = await pda.market(aiClaimPda.address);
    expect(marketPda2.address.toString()).toBe(marketPda.address.toString());
  });
});

describe("PDA: a custom programId is honored", () => {
  const OTHER_PROGRAM = new Address("11111111111111111111111111111111");

  it("default vs explicit Kassandra id are equal; a different id differs", async () => {
    const def = await pda.protocol();
    const explicit = await pda.protocol(KASSANDRA_PROGRAM_ID);
    const other = await pda.protocol(OTHER_PROGRAM);
    expect(explicit.address.toString()).toBe(def.address.toString());
    expect(other.address.toString()).not.toBe(def.address.toString());
  });
});

/**
 * WF1 offline unit tests for the write action layer (default suite — no network).
 *
 * A mock {@link Connection} returns `null` (ATA absent) or a token-account blob
 * (ATA present) from `getAccountInfo`. We assert that each builder:
 *   - prepends the idempotent create-ATA ix ONLY when the ATA is absent;
 *   - emits an action ix whose `data` + `keys` byte-for-byte match the SDK
 *     builder for the SAME inputs (derived ATA passed as the token account);
 *   - rejects bad input (bond/stake 0, uri > 200 bytes, bad vote kind, option
 *     out of range) with a typed `ValidationError`.
 */
import { Keypair, type Connection } from "@solana/web3.js";
import {
  ATA_PROGRAM_ID,
  VOTE_APPROVE,
  VOTE_DUPLICATE,
  associatedTokenAccount,
  propose,
  submitFact,
  voteFact,
} from "@kassandra-market/oracles";
import { describe, expect, it } from "vitest";

import {
  ValidationError,
  buildProposeIxs,
  buildSubmitFactIxs,
  buildVoteFactIxs,
  hashToContentHash,
} from "../src/data/actions.ts";
import type { TransactionInstruction } from "@solana/web3.js";

/** A mock Connection whose `getAccountInfo` reports the ATA present or absent. */
function mockConnection(ataPresent: boolean): Connection {
  return {
    getAccountInfo: async () => (ataPresent ? { data: new Uint8Array(165), owner: null } : null),
  } as unknown as Connection;
}

/** Compare an ix's keys by stringified pubkey + roles (Address identity is brittle under toEqual). */
function keyShape(ix: TransactionInstruction) {
  return ix.keys.map((k) => ({
    pubkey: k.pubkey.toString(),
    isSigner: k.isSigner,
    isWritable: k.isWritable,
  }));
}

async function fixture() {
  const oracle = (await Keypair.generate()).publicKey;
  const kassMint = (await Keypair.generate()).publicKey;
  const authority = (await Keypair.generate()).publicKey;
  const fact = (await Keypair.generate()).publicKey;
  const ata = (await associatedTokenAccount(authority, kassMint)).address;
  return { oracle, kassMint, authority, fact, ata };
}

describe("buildProposeIxs", () => {
  it("emits only the propose ix when the ATA exists, matching the SDK builder", async () => {
    const { oracle, kassMint, authority, ata } = await fixture();
    const ixs = await buildProposeIxs({
      connection: mockConnection(true),
      oracle,
      kassMint,
      authority,
      option: 1,
      bond: 5_000n,
    });
    expect(ixs.length).toBe(1);
    const expected = await propose({
      oracle,
      authority,
      authorityKass: ata,
      option: 1,
      bond: 5_000n,
    });
    expect(ixs[0].programId.toString()).toBe(expected.programId.toString());
    expect(Array.from(ixs[0].data)).toEqual(Array.from(expected.data));
    expect(keyShape(ixs[0])).toEqual(keyShape(expected));
  });

  it("prepends an idempotent create-ATA ix when the ATA is absent", async () => {
    const { oracle, kassMint, authority, ata } = await fixture();
    const ixs = await buildProposeIxs({
      connection: mockConnection(false),
      oracle,
      kassMint,
      authority,
      option: 0,
      bond: 1n,
    });
    expect(ixs.length).toBe(2);
    const [create, action] = ixs;
    expect(create.programId.toString()).toBe(ATA_PROGRAM_ID.toString());
    expect(Array.from(create.data)).toEqual([1]); // CreateIdempotent
    // ATA order: payer(w,signer), ata(w), owner(ro), mint(ro), system(ro), token(ro).
    expect(create.keys[0].pubkey.toString()).toBe(authority.toString());
    expect(create.keys[0].isSigner).toBe(true);
    expect(create.keys[1].pubkey.toString()).toBe(ata.toString());
    expect(create.keys[2].pubkey.toString()).toBe(authority.toString());
    expect(create.keys[3].pubkey.toString()).toBe(kassMint.toString());
    const expected = await propose({ oracle, authority, authorityKass: ata, option: 0, bond: 1n });
    expect(keyShape(action)).toEqual(keyShape(expected));
  });

  it("rejects a non-positive bond", async () => {
    const { oracle, kassMint, authority } = await fixture();
    await expect(
      buildProposeIxs({ connection: mockConnection(true), oracle, kassMint, authority, option: 0, bond: 0n }),
    ).rejects.toBeInstanceOf(ValidationError);
  });

  it("rejects an option outside optionsCount", async () => {
    const { oracle, kassMint, authority } = await fixture();
    await expect(
      buildProposeIxs({
        connection: mockConnection(true),
        oracle,
        kassMint,
        authority,
        option: 3,
        bond: 1n,
        optionsCount: 2,
      }),
    ).rejects.toBeInstanceOf(ValidationError);
  });
});

describe("buildSubmitFactIxs", () => {
  it("matches the SDK submitFact ix when the ATA exists", async () => {
    const { oracle, kassMint, authority, ata } = await fixture();
    const contentHash = new Uint8Array(32).fill(0x07);
    const ixs = await buildSubmitFactIxs({
      connection: mockConnection(true),
      oracle,
      kassMint,
      submitter: authority,
      contentHash,
      stake: 100n,
      uri: "ipfs://fact",
    });
    expect(ixs.length).toBe(1);
    const expected = await submitFact({
      oracle,
      submitter: authority,
      submitterKass: ata,
      contentHash,
      stake: 100n,
      uri: "ipfs://fact",
    });
    expect(Array.from(ixs[0].data)).toEqual(Array.from(expected.data));
    expect(keyShape(ixs[0])).toEqual(keyShape(expected));
  });

  it("prepends create-ATA when absent", async () => {
    const { oracle, kassMint, authority } = await fixture();
    const ixs = await buildSubmitFactIxs({
      connection: mockConnection(false),
      oracle,
      kassMint,
      submitter: authority,
      contentHash: new Uint8Array(32).fill(1),
      stake: 1n,
      uri: "x",
    });
    expect(ixs.length).toBe(2);
    expect(ixs[0].programId.toString()).toBe(ATA_PROGRAM_ID.toString());
  });

  it("rejects a uri over 200 bytes", async () => {
    const { oracle, kassMint, authority } = await fixture();
    await expect(
      buildSubmitFactIxs({
        connection: mockConnection(true),
        oracle,
        kassMint,
        submitter: authority,
        contentHash: new Uint8Array(32),
        stake: 1n,
        uri: "a".repeat(201),
      }),
    ).rejects.toBeInstanceOf(ValidationError);
  });

  it("rejects a zero stake", async () => {
    const { oracle, kassMint, authority } = await fixture();
    await expect(
      buildSubmitFactIxs({
        connection: mockConnection(true),
        oracle,
        kassMint,
        submitter: authority,
        contentHash: new Uint8Array(32),
        stake: 0n,
        uri: "x",
      }),
    ).rejects.toBeInstanceOf(ValidationError);
  });
});

describe("buildVoteFactIxs", () => {
  it("matches the SDK voteFact ix when the ATA exists", async () => {
    const { oracle, kassMint, authority, fact, ata } = await fixture();
    const ixs = await buildVoteFactIxs({
      connection: mockConnection(true),
      oracle,
      kassMint,
      fact,
      voter: authority,
      kind: VOTE_APPROVE,
      stake: 2_000n,
    });
    expect(ixs.length).toBe(1);
    const expected = await voteFact({
      oracle,
      fact,
      voter: authority,
      voterKass: ata,
      kind: VOTE_APPROVE,
      stake: 2_000n,
    });
    expect(Array.from(ixs[0].data)).toEqual(Array.from(expected.data));
    expect(keyShape(ixs[0])).toEqual(keyShape(expected));
  });

  it("prepends create-ATA when absent and accepts VOTE_DUPLICATE", async () => {
    const { oracle, kassMint, authority, fact } = await fixture();
    const ixs = await buildVoteFactIxs({
      connection: mockConnection(false),
      oracle,
      kassMint,
      fact,
      voter: authority,
      kind: VOTE_DUPLICATE,
      stake: 1n,
    });
    expect(ixs.length).toBe(2);
    expect(ixs[0].programId.toString()).toBe(ATA_PROGRAM_ID.toString());
  });

  it("rejects an invalid vote kind", async () => {
    const { oracle, kassMint, authority, fact } = await fixture();
    await expect(
      buildVoteFactIxs({
        connection: mockConnection(true),
        oracle,
        kassMint,
        fact,
        voter: authority,
        kind: 7,
        stake: 1n,
      }),
    ).rejects.toBeInstanceOf(ValidationError);
  });
});

describe("hashToContentHash", () => {
  it("produces a deterministic 32-byte SHA-256 digest", async () => {
    const a = await hashToContentHash("hello");
    const b = await hashToContentHash(new TextEncoder().encode("hello"));
    expect(a.length).toBe(32);
    expect(Array.from(a)).toEqual(Array.from(b));
  });
});

/**
 * RF2 offline unit tests for the claim / close / sweep settlement action layer
 * (default suite — no network). For each builder we assert its ix `data` + `keys`
 * byte-for-byte match the SDK settlement builder for the SAME inputs, that the
 * claims derive `destKass == ATA(authority, kassMint)` + `rentRecipient ==
 * authority` and prepend a create-ATA ONLY when the ATA is absent, that the
 * sweep's `dao_treasury == ATA(daoAuthority, kassMint)`, and that a missing /
 * invalid nonce is rejected with a typed `ValidationError`. Fully offline (a mock
 * Connection reports the ATA present/absent; every nonce is passed explicitly).
 */
import { Keypair, type Connection, type TransactionInstruction } from "@solana/web3.js";
import {
  ATA_PROGRAM_ID,
  associatedTokenAccount,
  claimFact,
  claimFactVote,
  claimProposer,
  closeAiClaim,
  closeMarket,
  pda,
  sweepOracle,
} from "@kassandra-market/oracles";
import { describe, expect, it } from "vitest";

import { ValidationError } from "../src/data/actions.ts";
import {
  buildClaimFactIxs,
  buildClaimFactVoteIxs,
  buildClaimProposerIxs,
  buildCloseAiClaimIxs,
  buildCloseMarketIxs,
  buildSweepOracleIxs,
} from "../src/data/actions/claims.ts";

/** A mock Connection whose `getAccountInfo` reports the ATA present or absent. */
function mockConnection(ataPresent: boolean): Connection {
  return {
    getAccountInfo: async () => (ataPresent ? { data: new Uint8Array(165), owner: null } : null),
  } as unknown as Connection;
}

/** Compare an ix's keys by stringified pubkey + roles. */
function keyShape(ix: TransactionInstruction) {
  return ix.keys.map((k) => ({
    pubkey: k.pubkey.toString(),
    isSigner: k.isSigner,
    isWritable: k.isWritable,
  }));
}

function expectIxMatches(actual: TransactionInstruction, expected: TransactionInstruction) {
  expect(actual.programId.toString()).toBe(expected.programId.toString());
  expect(Array.from(actual.data)).toEqual(Array.from(expected.data));
  expect(keyShape(actual)).toEqual(keyShape(expected));
}

async function key() {
  return (await Keypair.generate()).publicKey;
}

describe("buildClaimProposerIxs", () => {
  const nonce = 3n;

  it("matches the SDK claimProposer ix (destKass = ATA(authority), rent = authority)", async () => {
    const [proposer, authority, kassMint] = await Promise.all([key(), key(), key()]);
    const ata = (await associatedTokenAccount(authority, kassMint)).address;
    const ixs = await buildClaimProposerIxs({
      connection: mockConnection(true),
      oracleNonce: nonce,
      proposer,
      authority,
      kassMint,
    });
    expect(ixs.length).toBe(1);
    expectIxMatches(
      ixs[0],
      await claimProposer({ nonce, proposer, destKass: ata, rentRecipient: authority }),
    );
  });

  it("prepends a create-ATA ix when the authority's KASS ATA is absent", async () => {
    const [proposer, authority, kassMint] = await Promise.all([key(), key(), key()]);
    const ata = (await associatedTokenAccount(authority, kassMint)).address;
    const ixs = await buildClaimProposerIxs({
      connection: mockConnection(false),
      oracleNonce: nonce,
      proposer,
      authority,
      kassMint,
    });
    expect(ixs.length).toBe(2);
    const [create, claim] = ixs;
    expect(create.programId.toString()).toBe(ATA_PROGRAM_ID.toString());
    expect(Array.from(create.data)).toEqual([1]); // CreateIdempotent
    expect(create.keys[0].pubkey.toString()).toBe(authority.toString()); // payer(w,signer)
    expect(create.keys[0].isSigner).toBe(true);
    expect(create.keys[1].pubkey.toString()).toBe(ata.toString());
    expect(create.keys[2].pubkey.toString()).toBe(authority.toString()); // owner
    expect(create.keys[3].pubkey.toString()).toBe(kassMint.toString());
    expectIxMatches(
      claim,
      await claimProposer({ nonce, proposer, destKass: ata, rentRecipient: authority }),
    );
  });

  it("rejects a missing nonce with a ValidationError", async () => {
    const [proposer, authority, kassMint] = await Promise.all([key(), key(), key()]);
    await expect(
      buildClaimProposerIxs({
        connection: mockConnection(true),
        // @ts-expect-error deliberately omitting the required nonce
        oracleNonce: undefined,
        proposer,
        authority,
        kassMint,
      }),
    ).rejects.toBeInstanceOf(ValidationError);
  });
});

describe("buildClaimFactIxs", () => {
  const nonce = 5n;

  it("matches the SDK claimFact ix (destKass = ATA(authority), rent = authority)", async () => {
    const [fact, authority, kassMint] = await Promise.all([key(), key(), key()]);
    const ata = (await associatedTokenAccount(authority, kassMint)).address;
    const ixs = await buildClaimFactIxs({
      connection: mockConnection(true),
      oracleNonce: nonce,
      fact,
      authority,
      kassMint,
    });
    expect(ixs.length).toBe(1);
    expectIxMatches(
      ixs[0],
      await claimFact({ nonce, fact, destKass: ata, rentRecipient: authority }),
    );
  });

  it("prepends a create-ATA ix when absent", async () => {
    const [fact, authority, kassMint] = await Promise.all([key(), key(), key()]);
    const ixs = await buildClaimFactIxs({
      connection: mockConnection(false),
      oracleNonce: nonce,
      fact,
      authority,
      kassMint,
    });
    expect(ixs.length).toBe(2);
    expect(ixs[0].programId.toString()).toBe(ATA_PROGRAM_ID.toString());
  });
});

describe("buildClaimFactVoteIxs", () => {
  const nonce = 7n;

  it("matches the SDK claimFactVote ix (destKass = ATA(voter), rent = voter, fact threaded)", async () => {
    const [factVote, fact, voter, kassMint] = await Promise.all([key(), key(), key(), key()]);
    const ata = (await associatedTokenAccount(voter, kassMint)).address;
    const ixs = await buildClaimFactVoteIxs({
      connection: mockConnection(true),
      oracleNonce: nonce,
      factVote,
      fact,
      voter,
      kassMint,
    });
    expect(ixs.length).toBe(1);
    expectIxMatches(
      ixs[0],
      await claimFactVote({ nonce, factVote, fact, destKass: ata, rentRecipient: voter }),
    );
  });

  it("prepends a create-ATA ix when absent", async () => {
    const [factVote, fact, voter, kassMint] = await Promise.all([key(), key(), key(), key()]);
    const ixs = await buildClaimFactVoteIxs({
      connection: mockConnection(false),
      oracleNonce: nonce,
      factVote,
      fact,
      voter,
      kassMint,
    });
    expect(ixs.length).toBe(2);
    expect(ixs[0].programId.toString()).toBe(ATA_PROGRAM_ID.toString());
  });
});

describe("buildCloseAiClaimIxs", () => {
  it("matches the SDK closeAiClaim ix (no nonce, no ATA prep)", async () => {
    const [oracle, aiClaim, rentRecipient] = await Promise.all([key(), key(), key()]);
    const ixs = await buildCloseAiClaimIxs({ oracle, aiClaim, rentRecipient });
    expect(ixs.length).toBe(1);
    expectIxMatches(ixs[0], await closeAiClaim({ oracle, aiClaim, rentRecipient }));
  });
});

describe("buildCloseMarketIxs", () => {
  it("matches the SDK closeMarket ix for the explicit nonce", async () => {
    const nonce = 11n;
    const [market, rentRecipient] = await Promise.all([key(), key()]);
    const ixs = await buildCloseMarketIxs({ oracleNonce: nonce, market, rentRecipient });
    expect(ixs.length).toBe(1);
    expectIxMatches(ixs[0], await closeMarket({ nonce, market, rentRecipient }));
  });

  it("rejects a missing nonce", async () => {
    const [market, rentRecipient] = await Promise.all([key(), key()]);
    await expect(
      // @ts-expect-error deliberately omitting the required nonce
      buildCloseMarketIxs({ market, rentRecipient }),
    ).rejects.toBeInstanceOf(ValidationError);
  });
});

describe("buildSweepOracleIxs", () => {
  it("matches the SDK sweepOracle ix; dao_treasury == ATA(daoAuthority, kassMint)", async () => {
    const nonce = 13n;
    const [kassMint, daoAuthority, creator] = await Promise.all([key(), key(), key()]);
    const ixs = await buildSweepOracleIxs({ oracleNonce: nonce, kassMint, daoAuthority, creator });
    expect(ixs.length).toBe(1);
    const expected = await sweepOracle({ nonce, kassMint, daoAuthority, creator });
    expectIxMatches(ixs[0], expected);

    // The DAO treasury account (index 3) is the canonical ATA(daoAuthority, kassMint).
    const treasury = (await associatedTokenAccount(daoAuthority, kassMint)).address;
    expect(ixs[0].keys[3].pubkey.toString()).toBe(treasury.toString());
    expect(ixs[0].keys[3].isWritable).toBe(true);
    // Account 0 is the oracle PDA derived from the nonce (writable, closed).
    expect(ixs[0].keys[0].pubkey.toString()).toBe((await pda.oracle(nonce)).address.toString());
  });

  it("rejects a missing nonce", async () => {
    const [kassMint, daoAuthority, creator] = await Promise.all([key(), key(), key()]);
    await expect(
      // @ts-expect-error deliberately omitting the required nonce
      buildSweepOracleIxs({ kassMint, daoAuthority, creator }),
    ).rejects.toBeInstanceOf(ValidationError);
  });
});

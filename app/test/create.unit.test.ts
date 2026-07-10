/**
 * RF3 offline unit tests for the create-oracle action (default suite — no network).
 *
 * A mock {@link Connection} reports the creator's KASS ATA absent or present.
 * We assert `buildCreateOracleIxs`:
 *   - emits a `createOracle` ix whose `data` + `keys` byte-for-byte match the SDK
 *     builder for the SAME inputs (the derived creator ATA as `creatorKassToken`
 *     and the derived Oracle PDA), and appends `writeOracleMeta` when options are
 *     given;
 *   - prepends the idempotent create-ATA ix ONLY when the ATA is absent;
 *   - returns the resolved nonce + Oracle PDA + extended metadata;
 *   - rejects bad input (optionsCount 1, past deadline, empty question) with a
 *     typed `ValidationError`.
 */
import { Keypair, type Connection } from "@solana/web3.js";
import {
  ATA_PROGRAM_ID,
  associatedTokenAccount,
  createOracle,
  pda,
} from "@kassandra-market/oracles";
import { describe, expect, it } from "vitest";

import { ValidationError } from "../src/data/actions.ts";
import { buildCreateOracleIxs } from "../src/data/actions/create.ts";
import type { TransactionInstruction } from "@solana/web3.js";

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

async function fixture() {
  const creator = (await Keypair.generate()).publicKey;
  const kassMint = (await Keypair.generate()).publicKey;
  const usdcMint = (await Keypair.generate()).publicKey;
  const ata = (await associatedTokenAccount(creator, kassMint)).address;
  return { creator, kassMint, usdcMint, ata };
}

const FUTURE = BigInt(Math.floor(Date.now() / 1000) + 7 * 24 * 3600);

describe("buildCreateOracleIxs", () => {
  it("emits only the createOracle ix when the ATA exists, matching the SDK builder", async () => {
    const { creator, kassMint, usdcMint, ata } = await fixture();
    const nonce = 42n;
    const question = "Did the SpaceX Starship reach orbit before 2027?";
    const built = await buildCreateOracleIxs({
      connection: mockConnection(true),
      nonce,
      question,
      optionsCount: 3,
      deadline: FUTURE,
      creator,
      kassMint,
      usdcMint,
    });

    expect(built.ixs.length).toBe(1);
    // No `options` → no metadata written; returned Oracle PDA == pda.oracle(nonce).
    expect(built.metadata).toBeUndefined();
    expect(built.oracle.toString()).toBe((await pda.oracle(nonce)).address.toString());
    expect(built.nonce).toBe(nonce);

    const expected = await createOracle({
      nonce,
      optionsCount: 3,
      deadline: FUTURE,
      twapWindow: 3600n,
      creator,
      creatorKassToken: ata,
      kassMint,
      usdcMint,
    });
    expect(built.ixs[0].programId.toString()).toBe(expected.programId.toString());
    expect(Array.from(built.ixs[0].data)).toEqual(Array.from(expected.data));
    expect(keyShape(built.ixs[0])).toEqual(keyShape(expected));
  });

  it("appends the writeOracleMeta ix and derives options_count from the labels", async () => {
    const { creator, kassMint, usdcMint } = await fixture();
    const question = "Which team wins?";
    const options = ["Red", "Blue", "Draw"];
    const built = await buildCreateOracleIxs({
      connection: mockConnection(true),
      nonce: 9n,
      question,
      options,
      deadline: FUTURE,
      creator,
      kassMint,
      usdcMint,
      appOrigin: "https://app.test",
    });

    // createOracle ix + the writeOracleMeta ix (ATA already present).
    expect(built.ixs.length).toBe(2);
    const meta = built.ixs[1];
    // WriteOracleMeta discriminant (23); accounts: creator(signer), oracle, meta, system.
    expect(meta.data[0]).toBe(23);
    expect(meta.keys.length).toBe(4);
    expect(meta.keys[0].isSigner).toBe(true);

    // The extended JSON returned for hosting carries the subject/options + a uri
    // pointing at the app origin, bound by a 32-byte uri_hash.
    expect(built.metadata?.json.subject).toBe(question);
    expect(built.metadata?.json.options).toEqual(options);
    expect(built.metadata?.uri).toBe(
      `https://app.test/api/oracle/${built.oracle.toString()}/metadata.json`,
    );
    expect(built.metadata?.uriHash.length).toBe(32);

    // options_count on the createOracle payload is derived from labels.length (3).
    const expected = await createOracle({
      nonce: 9n,
      optionsCount: options.length,
      deadline: FUTURE,
      twapWindow: 3600n,
      creator,
      creatorKassToken: (await associatedTokenAccount(creator, kassMint)).address,
      kassMint,
      usdcMint,
    });
    expect(Array.from(built.ixs[0].data)).toEqual(Array.from(expected.data));
  });

  it("prepends an idempotent create-ATA ix when the creator's ATA is absent", async () => {
    const { creator, kassMint, usdcMint, ata } = await fixture();
    const built = await buildCreateOracleIxs({
      connection: mockConnection(false),
      nonce: 7n,
      question: "Q?",
      optionsCount: 2,
      deadline: FUTURE,
      creator,
      kassMint,
      usdcMint,
    });
    expect(built.ixs.length).toBe(2);
    const [create, action] = built.ixs;
    expect(create.programId.toString()).toBe(ATA_PROGRAM_ID.toString());
    expect(Array.from(create.data)).toEqual([1]); // CreateIdempotent
    // ATA order: payer(w,signer), ata(w), owner(ro), mint(ro), system(ro), token(ro).
    expect(create.keys[0].pubkey.toString()).toBe(creator.toString());
    expect(create.keys[0].isSigner).toBe(true);
    expect(create.keys[1].pubkey.toString()).toBe(ata.toString());
    expect(create.keys[2].pubkey.toString()).toBe(creator.toString());
    expect(create.keys[3].pubkey.toString()).toBe(kassMint.toString());

    const expected = await createOracle({
      nonce: 7n,
      optionsCount: 2,
      deadline: FUTURE,
      twapWindow: 3600n,
      creator,
      creatorKassToken: ata,
      kassMint,
      usdcMint,
    });
    expect(keyShape(action)).toEqual(keyShape(expected));
  });

  it("generates a random nonce when none is supplied (Oracle PDA matches it)", async () => {
    const { creator, kassMint, usdcMint } = await fixture();
    const built = await buildCreateOracleIxs({
      connection: mockConnection(true),
      question: "Q?",
      optionsCount: 2,
      deadline: FUTURE,
      creator,
      kassMint,
      usdcMint,
    });
    expect(built.nonce).toBeGreaterThanOrEqual(0n);
    expect(built.oracle.toString()).toBe((await pda.oracle(built.nonce)).address.toString());
  });

  it("rejects an options count below 2", async () => {
    const { creator, kassMint, usdcMint } = await fixture();
    await expect(
      buildCreateOracleIxs({
        connection: mockConnection(true),
        question: "Q?",
        optionsCount: 1,
        deadline: FUTURE,
        creator,
        kassMint,
        usdcMint,
      }),
    ).rejects.toBeInstanceOf(ValidationError);
  });

  it("rejects a past deadline", async () => {
    const { creator, kassMint, usdcMint } = await fixture();
    await expect(
      buildCreateOracleIxs({
        connection: mockConnection(true),
        question: "Q?",
        optionsCount: 2,
        deadline: BigInt(Math.floor(Date.now() / 1000) - 100),
        creator,
        kassMint,
        usdcMint,
      }),
    ).rejects.toBeInstanceOf(ValidationError);
  });

  it("rejects an empty question", async () => {
    const { creator, kassMint, usdcMint } = await fixture();
    await expect(
      buildCreateOracleIxs({
        connection: mockConnection(true),
        question: "   ",
        optionsCount: 2,
        deadline: FUTURE,
        creator,
        kassMint,
        usdcMint,
      }),
    ).rejects.toBeInstanceOf(ValidationError);
  });
});

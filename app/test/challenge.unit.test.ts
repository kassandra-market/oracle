/**
 * RF4 offline unit tests for the challenge (open / settle) + submit-ai-claim
 * action layer (default suite — no network). For each builder we assert its ix
 * `data` + `keys` byte-for-byte match the SDK challenge/dispute builder for the
 * SAME inputs (the exact MetaDAO market account wiring), that the submit-ai-claim
 * payload is `model_id ++ params_hash ++ io_hash ++ option`, and that validation
 * rejects a missing/invalid nonce, a non-32-byte hash, and an out-of-range option
 * with a typed `ValidationError`. Fully offline.
 */
import { Keypair, type TransactionInstruction } from "@solana/web3.js";
import {
  openChallenge,
  settleChallenge,
  submitAiClaim,
} from "@kassandra-market/oracles";
import { describe, expect, it } from "vitest";

import { ValidationError } from "../src/data/actions.ts";
import {
  buildOpenChallengeIxs,
  buildSettleChallengeIxs,
  buildSubmitAiClaimIxs,
} from "../src/data/actions/challenge.ts";

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

/** Fabricate the full externally-composed MetaDAO market account set. */
async function marketAccounts() {
  const [
    proposer,
    challenger,
    question,
    kassVault,
    usdcVault,
    passAmm,
    failAmm,
    kassVaultUnderlying,
    passKassMint,
    failKassMint,
    oraclePassKass,
    oracleFailKass,
    cvEventAuthority,
    kassDao,
    usdcMint,
    challengerUsdcSrc,
    aiClaim,
    proposerUsdc,
    challengerUsdcDest,
    challengerKass,
  ] = await Promise.all(Array.from({ length: 20 }, () => key()));
  return {
    proposer,
    challenger,
    question,
    kassVault,
    usdcVault,
    passAmm,
    failAmm,
    kassVaultUnderlying,
    passKassMint,
    failKassMint,
    oraclePassKass,
    oracleFailKass,
    cvEventAuthority,
    kassDao,
    usdcMint,
    challengerUsdcSrc,
    aiClaim,
    proposerUsdc,
    challengerUsdcDest,
    challengerKass,
  };
}

describe("buildOpenChallengeIxs", () => {
  const nonce = 100n;

  it("matches the SDK openChallenge ix (all 25 accounts + nonce payload)", async () => {
    const a = await marketAccounts();
    const ixs = await buildOpenChallengeIxs({ oracleNonce: nonce, ...a });
    expect(ixs.length).toBe(1);
    expectIxMatches(ixs[0], await openChallenge({ nonce, ...a }));
  });

  it("rejects a missing nonce with a ValidationError", async () => {
    const a = await marketAccounts();
    await expect(
      // @ts-expect-error deliberately omitting the required nonce
      buildOpenChallengeIxs({ ...a }),
    ).rejects.toBeInstanceOf(ValidationError);
  });

  it("rejects an unparseable account with a ValidationError", async () => {
    const a = await marketAccounts();
    await expect(
      buildOpenChallengeIxs({ oracleNonce: nonce, ...a, question: "not-base58!!!" }),
    ).rejects.toBeInstanceOf(ValidationError);
  });
});

describe("buildSettleChallengeIxs", () => {
  const nonce = 200n;

  it("matches the SDK settleChallenge ix (all 21 accounts + nonce payload)", async () => {
    const a = await marketAccounts();
    const ixs = await buildSettleChallengeIxs({
      oracleNonce: nonce,
      aiClaim: a.aiClaim,
      proposer: a.proposer,
      question: a.question,
      passAmm: a.passAmm,
      failAmm: a.failAmm,
      cvEventAuthority: a.cvEventAuthority,
      kassVault: a.kassVault,
      kassVaultUnderlying: a.kassVaultUnderlying,
      passKassMint: a.passKassMint,
      failKassMint: a.failKassMint,
      oraclePassKass: a.oraclePassKass,
      oracleFailKass: a.oracleFailKass,
      proposerUsdc: a.proposerUsdc,
      challengerUsdcDest: a.challengerUsdcDest,
      challengerKass: a.challengerKass,
    });
    expect(ixs.length).toBe(1);
    expectIxMatches(
      ixs[0],
      await settleChallenge({
        nonce,
        aiClaim: a.aiClaim,
        proposer: a.proposer,
        question: a.question,
        passAmm: a.passAmm,
        failAmm: a.failAmm,
        cvEventAuthority: a.cvEventAuthority,
        kassVault: a.kassVault,
        kassVaultUnderlying: a.kassVaultUnderlying,
        passKassMint: a.passKassMint,
        failKassMint: a.failKassMint,
        oraclePassKass: a.oraclePassKass,
        oracleFailKass: a.oracleFailKass,
        proposerUsdc: a.proposerUsdc,
        challengerUsdcDest: a.challengerUsdcDest,
        challengerKass: a.challengerKass,
      }),
    );
  });

  it("rejects a missing nonce with a ValidationError", async () => {
    const a = await marketAccounts();
    await expect(
      buildSettleChallengeIxs({
        // @ts-expect-error deliberately omitting the required nonce
        aiClaim: a.aiClaim,
        proposer: a.proposer,
        question: a.question,
        passAmm: a.passAmm,
        failAmm: a.failAmm,
        cvEventAuthority: a.cvEventAuthority,
        kassVault: a.kassVault,
        kassVaultUnderlying: a.kassVaultUnderlying,
        passKassMint: a.passKassMint,
        failKassMint: a.failKassMint,
        oraclePassKass: a.oraclePassKass,
        oracleFailKass: a.oracleFailKass,
        proposerUsdc: a.proposerUsdc,
        challengerUsdcDest: a.challengerUsdcDest,
        challengerKass: a.challengerKass,
      }),
    ).rejects.toBeInstanceOf(ValidationError);
  });
});

describe("buildSubmitAiClaimIxs", () => {
  const modelId = new Uint8Array(32).fill(0xa1);
  const paramsHash = new Uint8Array(32).fill(0xb2);
  const ioHash = new Uint8Array(32).fill(0xc3);

  it("matches the SDK submitAiClaim ix; payload == model_id ++ params_hash ++ io_hash ++ option", async () => {
    const [oracle, proposer, submitter] = await Promise.all([key(), key(), key()]);
    const option = 1;
    const ixs = await buildSubmitAiClaimIxs({
      oracle,
      proposer,
      submitter,
      modelId,
      paramsHash,
      ioHash,
      option,
    });
    expect(ixs.length).toBe(1);
    expectIxMatches(
      ixs[0],
      await submitAiClaim({ oracle, proposer, authority: submitter, modelId, paramsHash, ioHash, option }),
    );

    // Byte-exact payload: 1-byte disc + 32 + 32 + 32 + 1-byte option.
    const data = ixs[0].data;
    expect(data.length).toBe(1 + 32 + 32 + 32 + 1);
    expect(Array.from(data.slice(1, 33))).toEqual(Array.from(modelId));
    expect(Array.from(data.slice(33, 65))).toEqual(Array.from(paramsHash));
    expect(Array.from(data.slice(65, 97))).toEqual(Array.from(ioHash));
    expect(data[97]).toBe(option);
  });

  it("rejects a non-32-byte hash with a ValidationError", async () => {
    const [oracle, proposer, submitter] = await Promise.all([key(), key(), key()]);
    await expect(
      buildSubmitAiClaimIxs({
        oracle,
        proposer,
        submitter,
        modelId: new Uint8Array(31).fill(1),
        paramsHash,
        ioHash,
        option: 0,
      }),
    ).rejects.toBeInstanceOf(ValidationError);
  });

  it("rejects an out-of-range option against optionsCount", async () => {
    const [oracle, proposer, submitter] = await Promise.all([key(), key(), key()]);
    await expect(
      buildSubmitAiClaimIxs({
        oracle,
        proposer,
        submitter,
        modelId,
        paramsHash,
        ioHash,
        option: 3,
        optionsCount: 2,
      }),
    ).rejects.toBeInstanceOf(ValidationError);
  });

  it("rejects a negative option", async () => {
    const [oracle, proposer, submitter] = await Promise.all([key(), key(), key()]);
    await expect(
      buildSubmitAiClaimIxs({ oracle, proposer, submitter, modelId, paramsHash, ioHash, option: -1 }),
    ).rejects.toBeInstanceOf(ValidationError);
  });
});

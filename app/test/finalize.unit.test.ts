/**
 * RF1 offline unit tests for the finalize / crank action layer (default suite —
 * no network). For each builder we assert its ix `data` + `keys` byte-for-byte
 * match the SDK finalize builder for the SAME inputs, the child tail is threaded
 * correctly, and the near-cap → `needsAlt` decision flips at exactly
 * {@link MAX_LEGACY_TAIL}. Also covers the pure {@link resolveOracleNonce} scan
 * and the tail/empty validation. Fully offline (the nonce is passed explicitly
 * where a builder needs it, so no RPC and no scan).
 */
import { Keypair, type Address, type TransactionInstruction } from "@solana/web3.js";
import {
  advancePhase,
  finalizeAiClaims,
  finalizeFacts,
  finalizeOracle,
  finalizeProposals,
  pda,
} from "@kassandra-market/oracles";
import { describe, expect, it } from "vitest";

import {
  MAX_LEGACY_TAIL,
  OracleNonceUnresolvedError,
  buildAdvancePhaseIxs,
  buildFinalizeAiClaimsIxs,
  buildFinalizeFactsIxs,
  buildFinalizeOracleIxs,
  buildFinalizeProposalsIxs,
  resolveOracleNonce,
} from "../src/data/actions/finalize.ts";
import { ValidationError } from "../src/data/actions.ts";

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

async function keys(n: number): Promise<Address[]> {
  const out: Address[] = [];
  for (let i = 0; i < n; i++) out.push((await Keypair.generate()).publicKey);
  return out;
}

describe("buildFinalizeProposalsIxs", () => {
  it("matches the SDK finalizeProposals ix and threads the full proposer tail", async () => {
    const oracle = (await Keypair.generate()).publicKey;
    const proposers = await keys(3);
    const { ixs, needsAlt, altAddresses } = await buildFinalizeProposalsIxs({ oracle, proposers });
    expect(ixs.length).toBe(1);
    expectIxMatches(ixs[0], await finalizeProposals({ oracle, proposers }));
    // The full proposer set is present as a read-only tail after the oracle.
    expect(ixs[0].keys.slice(1).map((k) => k.pubkey.toString())).toEqual(
      proposers.map((p) => p.toString()),
    );
    expect(needsAlt).toBe(false);
    expect(altAddresses).toEqual([]);
  });

  it("flips needsAlt at MAX_LEGACY_TAIL (near-cap → v0/ALT)", async () => {
    const oracle = (await Keypair.generate()).publicKey;
    const atLimit = await keys(MAX_LEGACY_TAIL);
    const over = await keys(MAX_LEGACY_TAIL + 1);

    const a = await buildFinalizeProposalsIxs({ oracle, proposers: atLimit });
    expect(a.needsAlt).toBe(false);
    expect(a.altAddresses).toEqual([]);

    const b = await buildFinalizeProposalsIxs({ oracle, proposers: over });
    expect(b.needsAlt).toBe(true);
    expect(b.altAddresses.map((x) => x.toString())).toEqual(over.map((x) => x.toString()));
  });

  it("rejects an empty proposer tail", async () => {
    const oracle = (await Keypair.generate()).publicKey;
    await expect(buildFinalizeProposalsIxs({ oracle, proposers: [] })).rejects.toBeInstanceOf(
      ValidationError,
    );
  });
});

describe("buildAdvancePhaseIxs", () => {
  it("matches the SDK advancePhase ix and never needs an ALT", async () => {
    const oracle = (await Keypair.generate()).publicKey;
    const { ixs, needsAlt, altAddresses } = await buildAdvancePhaseIxs({ oracle });
    expect(ixs.length).toBe(1);
    expectIxMatches(ixs[0], await advancePhase({ oracle }));
    expect(needsAlt).toBe(false);
    expect(altAddresses).toEqual([]);
  });
});

describe("buildFinalizeFactsIxs", () => {
  it("matches the SDK finalizeFacts ix with the explicit nonce + fact tail", async () => {
    const nonce = 3n;
    const oracle = (await pda.oracle(nonce)).address;
    const kassMint = (await Keypair.generate()).publicKey;
    const facts = await keys(2);
    const { ixs, needsAlt } = await buildFinalizeFactsIxs({
      oracle,
      kassMint,
      facts,
      oracleNonce: nonce,
    });
    expect(ixs.length).toBe(1);
    expectIxMatches(ixs[0], await finalizeFacts({ nonce, kassMint, tail: facts }));
    // The subset-capable finalize is chunked, never ALT-routed.
    expect(needsAlt).toBe(false);
  });

  it("resolves the nonce from the oracle pubkey when omitted (pure scan)", async () => {
    const nonce = 5n;
    const oracle = (await pda.oracle(nonce)).address;
    const kassMint = (await Keypair.generate()).publicKey;
    const facts = await keys(1);
    const { ixs } = await buildFinalizeFactsIxs({ oracle, kassMint, facts });
    // Byte-identical to the explicit-nonce build ⇒ the scan recovered nonce 5.
    expectIxMatches(ixs[0], await finalizeFacts({ nonce, kassMint, tail: facts }));
  });

  it("rejects an empty fact tail", async () => {
    const oracle = (await pda.oracle(1n)).address;
    const kassMint = (await Keypair.generate()).publicKey;
    await expect(
      buildFinalizeFactsIxs({ oracle, kassMint, facts: [], oracleNonce: 1n }),
    ).rejects.toBeInstanceOf(ValidationError);
  });
});

describe("buildFinalizeAiClaimsIxs", () => {
  it("matches the SDK finalizeAiClaims ix with a writable proposer subset", async () => {
    const oracle = (await Keypair.generate()).publicKey;
    const proposers = await keys(2);
    const { ixs, needsAlt } = await buildFinalizeAiClaimsIxs({ oracle, proposers });
    expect(ixs.length).toBe(1);
    expectIxMatches(ixs[0], await finalizeAiClaims({ oracle, proposers }));
    expect(needsAlt).toBe(false);
  });

  it("rejects an empty proposer tail", async () => {
    const oracle = (await Keypair.generate()).publicKey;
    await expect(buildFinalizeAiClaimsIxs({ oracle, proposers: [] })).rejects.toBeInstanceOf(
      ValidationError,
    );
  });
});

describe("buildFinalizeOracleIxs", () => {
  it("matches the SDK finalizeOracle ix with the explicit nonce + full ro tail", async () => {
    const nonce = 7n;
    const oracle = (await pda.oracle(nonce)).address;
    const kassMint = (await Keypair.generate()).publicKey;
    const proposers = await keys(2);
    const { ixs, needsAlt } = await buildFinalizeOracleIxs({
      oracle,
      kassMint,
      proposers,
      oracleNonce: nonce,
    });
    expect(ixs.length).toBe(1);
    expectIxMatches(ixs[0], await finalizeOracle({ nonce, kassMint, proposers }));
    expect(needsAlt).toBe(false);
  });

  it("flips needsAlt at MAX_LEGACY_TAIL", async () => {
    const nonce = 9n;
    const oracle = (await pda.oracle(nonce)).address;
    const kassMint = (await Keypair.generate()).publicKey;
    const over = await keys(MAX_LEGACY_TAIL + 1);
    const { needsAlt, altAddresses } = await buildFinalizeOracleIxs({
      oracle,
      kassMint,
      proposers: over,
      oracleNonce: nonce,
    });
    expect(needsAlt).toBe(true);
    expect(altAddresses.length).toBe(MAX_LEGACY_TAIL + 1);
  });
});

describe("resolveOracleNonce", () => {
  it("recovers a small nonce by re-deriving the oracle PDA", async () => {
    const oracle = (await pda.oracle(11n)).address;
    expect(await resolveOracleNonce(oracle)).toBe(11n);
  });

  it("throws OracleNonceUnresolvedError past the scan bound", async () => {
    const oracle = (await pda.oracle(11n)).address;
    await expect(resolveOracleNonce(oracle, { maxNonce: 5 })).rejects.toBeInstanceOf(
      OracleNonceUnresolvedError,
    );
  });
});

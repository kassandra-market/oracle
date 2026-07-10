/**
 * D3b — instruction-builder byte/meta tests for the dispute builders
 * (submit/vote facts, AI claim, finalize).
 *
 * For each builder we assert:
 *   - `data == [disc, ...payload_LE]`, with the payload buffer constructed
 *     INDEPENDENTLY here (so an encoder regression is caught);
 *   - `keys == ` the processor's documented account order, each with the right
 *     `isSigner`/`isWritable` role, PDAs in the correct slots — cross-checked
 *     against the `*_ix` helpers in `programs/oracles/tests/{common/mod,
 *     settlement_e2e,challenge_e2e}.rs`.
 *
 * The challenge + settlement builders live in instructions-challenge.test.ts;
 * shared fixtures/helpers live in ./helpers/instructions-dispute.ts.
 */
import { describe, expect, it } from "vitest";

import { Ix, SYSTEM_PROGRAM_ID, TOKEN_PROGRAM_ID } from "../src/constants.js";
import * as pda from "../src/pda.js";
import {
  finalizeAiClaims,
  finalizeFacts,
  finalizeOracle,
  submitAiClaim,
  submitFact,
  voteFact,
} from "../src/instructions/index.js";
import {
  AUTHORITY,
  AUTHORITY_KASS,
  FACT,
  KASS_MINT,
  ORACLE,
  PROPOSER,
  bytesOf,
  enc,
  leU16,
  leU64,
  metaTriples,
} from "./helpers/instructions-dispute.js";

describe("D3b dispute builders — submit/vote facts, AI claim, finalize", () => {
  it("submitFact: content_hash[32] ++ stake u64 ++ uri_len u16 ++ uri", async () => {
    const contentHash = new Uint8Array(32).fill(0x07);
    const stake = 300n;
    const uri = "ipfs://fact";
    const uriBytes = enc.encode(uri);

    const ix = await submitFact({
      oracle: ORACLE,
      submitter: AUTHORITY,
      submitterKass: AUTHORITY_KASS,
      contentHash,
      stake,
      uri,
    });

    const expected = bytesOf(Ix.SubmitFact, [
      ...Array.from(contentHash),
      ...leU64(stake),
      ...leU16(uriBytes.length),
      ...Array.from(uriBytes),
    ]);
    expect(ix.data).toEqual(expected);

    const fact = await pda.fact(ORACLE, contentHash);
    const stakeVault = await pda.stakeVault(ORACLE);
    expect(metaTriples(ix.keys)).toEqual([
      [ORACLE, false, true],
      [fact.address.toString(), false, true],
      [AUTHORITY, true, true],
      [AUTHORITY_KASS, false, true],
      [stakeVault.address.toString(), false, true],
      [TOKEN_PROGRAM_ID.toString(), false, false],
      [SYSTEM_PROGRAM_ID.toString(), false, false],
    ]);
  });

  it("submitFact: accepts a raw Uint8Array uri", async () => {
    const contentHash = new Uint8Array(32).fill(1);
    const uri = new Uint8Array([1, 2, 3, 4, 5]);
    const ix = await submitFact({
      oracle: ORACLE,
      submitter: AUTHORITY,
      submitterKass: AUTHORITY_KASS,
      contentHash,
      stake: 1n,
      uri,
    });
    expect(ix.data).toEqual(
      bytesOf(Ix.SubmitFact, [
        ...Array.from(contentHash),
        ...leU64(1n),
        ...leU16(5),
        1, 2, 3, 4, 5,
      ]),
    );
  });

  it("voteFact: kind u8 ++ stake u64 + 8 accounts", async () => {
    const kind = 1;
    const stake = 250n;
    const ix = await voteFact({
      oracle: ORACLE,
      fact: FACT,
      voter: AUTHORITY,
      voterKass: AUTHORITY_KASS,
      kind,
      stake,
    });

    expect(ix.data).toEqual(bytesOf(Ix.VoteFact, [kind, ...leU64(stake)]));

    const factVote = await pda.factVote(FACT, AUTHORITY);
    const stakeVault = await pda.stakeVault(ORACLE);
    expect(metaTriples(ix.keys)).toEqual([
      [ORACLE, false, true],
      [FACT, false, true],
      [factVote.address.toString(), false, true],
      [AUTHORITY, true, true],
      [AUTHORITY_KASS, false, true],
      [stakeVault.address.toString(), false, true],
      [TOKEN_PROGRAM_ID.toString(), false, false],
      [SYSTEM_PROGRAM_ID.toString(), false, false],
    ]);
  });

  it("finalizeFacts: nonce u64 payload, oracle/kass_mint/stake_vault/token + writable tail", async () => {
    const nonce = 7n;
    const tail = [FACT, PROPOSER, AUTHORITY];
    const ix = await finalizeFacts({ nonce, kassMint: KASS_MINT, tail });
    expect(ix.data).toEqual(bytesOf(Ix.FinalizeFacts, leU64(nonce)));

    const oracle = await pda.oracle(nonce);
    const stakeVault = await pda.stakeVault(oracle.address);
    expect(metaTriples(ix.keys)).toEqual([
      [oracle.address.toString(), false, true],
      [KASS_MINT, false, true],
      [stakeVault.address.toString(), false, true],
      [TOKEN_PROGRAM_ID.toString(), false, false],
      [FACT, false, true],
      [PROPOSER, false, true],
      [AUTHORITY, false, true],
    ]);
  });

  it("submitAiClaim: model_id[32] ++ params_hash[32] ++ io_hash[32] ++ option u8", async () => {
    const modelId = new Uint8Array(32).fill(0xaa);
    const paramsHash = new Uint8Array(32).fill(0xbb);
    const ioHash = new Uint8Array(32).fill(0xcc);
    const option = 1;

    const ix = await submitAiClaim({
      oracle: ORACLE,
      proposer: PROPOSER,
      authority: AUTHORITY,
      modelId,
      paramsHash,
      ioHash,
      option,
    });

    const expected = bytesOf(Ix.SubmitAiClaim, [
      ...Array.from(modelId),
      ...Array.from(paramsHash),
      ...Array.from(ioHash),
      option,
    ]);
    expect(ix.data).toEqual(expected);
    expect(ix.data.length).toBe(1 + 97);

    const aiClaim = await pda.aiClaim(ORACLE, PROPOSER);
    expect(metaTriples(ix.keys)).toEqual([
      [ORACLE, false, true],
      [PROPOSER, false, true],
      [aiClaim.address.toString(), false, true],
      [AUTHORITY, true, true],
      [SYSTEM_PROGRAM_ID.toString(), false, false],
    ]);
  });

  it("finalizeAiClaims: empty payload, oracle(w) + writable proposer tail", async () => {
    const proposers = [PROPOSER, AUTHORITY];
    const ix = await finalizeAiClaims({ oracle: ORACLE, proposers });
    expect(ix.data).toEqual(bytesOf(Ix.FinalizeAiClaims));
    expect(metaTriples(ix.keys)).toEqual([
      [ORACLE, false, true],
      [PROPOSER, false, true],
      [AUTHORITY, false, true],
    ]);
  });

  it("finalizeOracle: nonce u64 payload, oracle/kass_mint/stake_vault/token + ro proposer tail", async () => {
    const nonce = 7n;
    const proposers = [PROPOSER, AUTHORITY];
    const ix = await finalizeOracle({ nonce, kassMint: KASS_MINT, proposers });

    expect(ix.data).toEqual(bytesOf(Ix.FinalizeOracle, leU64(nonce)));

    const oracle = await pda.oracle(nonce);
    const stakeVault = await pda.stakeVault(oracle.address);
    expect(metaTriples(ix.keys)).toEqual([
      [oracle.address.toString(), false, true],
      [KASS_MINT, false, true],
      [stakeVault.address.toString(), false, true],
      [TOKEN_PROGRAM_ID.toString(), false, false],
      [PROPOSER, false, false],
      [AUTHORITY, false, false],
    ]);
  });
});

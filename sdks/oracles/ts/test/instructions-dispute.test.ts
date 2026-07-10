/**
 * D3b — instruction-builder byte/meta tests for the dispute + challenge +
 * settlement builders (the remaining 13 instructions).
 *
 * For each builder we assert:
 *   - `data == [disc, ...payload_LE]`, with the payload buffer constructed
 *     INDEPENDENTLY here (so an encoder regression is caught);
 *   - `keys == ` the processor's documented account order, each with the right
 *     `isSigner`/`isWritable` role, PDAs in the correct slots — cross-checked
 *     against the `*_ix` helpers in `programs/oracles/tests/{common/mod,
 *     settlement_e2e,challenge_e2e}.rs`.
 *
 * The long OpenChallenge/SettleChallenge lists are asserted slot-by-slot.
 */
import { Address } from "@solana/web3.js";
import { describe, expect, it } from "vitest";

import {
  EXTERNAL_PROGRAM_IDS,
  Ix,
  SYSTEM_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "../src/constants.js";
import * as pda from "../src/pda.js";
import {
  claimFact,
  claimFactVote,
  claimProposer,
  closeAiClaim,
  closeMarket,
  sweepOracle,
  finalizeAiClaims,
  finalizeFacts,
  finalizeOracle,
  openChallenge,
  settleChallenge,
  submitAiClaim,
  submitFact,
  voteFact,
} from "../src/instructions/index.js";

// Deterministic stand-in keys (valid 32-byte base58 addresses).
const ORACLE = "GuBhyNi5GFo9K5YXGKfPMDryWK8GwS5oXe9CJGrzo2sk";
const KASS_MINT = "So11111111111111111111111111111111111111112";
const USDC_MINT = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const AUTHORITY = "7bQEwuq9ybNyjjFcbtHBfDPxdH3TuGAsZKVRZdihVN4d";
const AUTHORITY_KASS = "EScpWtUwYodKnbZx46YYeJbp2Ci2EpqcLAkF2EdZnZrh";
const PROPOSER = "84yVtdReAJ8GiR7Erqj7jyxoJurYWzQ6n9eaBGYBDNqM";
const FACT = "FYQFL976rxQv8hygbC1zPVZYMfbnQkVntriESv69KaED";
const FACT_VOTE = "7WCvk98KGRqi2o8D7EWTGrZQuFtikidP8A2D7CDVXwWJ";
const CHALLENGER = "rqRMW2HFJsi1FE1jb8Rvaz4Qz3xHzNkZDb8am1pqEHE";
const DEST_KASS = "9wFFyRfZBsuAha4YcuxcXLKwMxJR43S7fPfQLusDBzvT";
const RENT_RECIPIENT = "DjVE6JNiYqPL2QXyCUUh8rNjHrbz9hXHNYt99MQ59qw1";
// MetaDAO / external accounts (caller-composed in the real flow).
const QUESTION = "Gdnq3GYwQK9wMcZ4tNJjJfQbjPR55Mz6Mw59HCWMy2ER";
const KASS_VAULT = "AeyTjbHr7yEZQ2KZX26ZbVZ4kgYFp5pZ5HfPwT5hLuMz";
const USDC_VAULT = "HxhWj4WSvm2Qw4bA8K1xRZH7AcsmZ9c7q3bdsoFiY3Cd";
const PASS_AMM = "5xUNJK9MZJtoSDc1nXFvFvgQ9hpqfHRZdLkVXCWfd9hM";
const FAIL_AMM = "Cw4Hcuv7Bs4qB1tQR1Z9D6vWG2sCcTwbQGm7Yqsnz3uG";
const KASS_VAULT_UNDERLYING = "8KhywBoQbBxAdtdAa3hKzZ4u3F8s5cQ7p1Tym9SHnpZ6";
const PASS_KASS_MINT = "2tFsVQ9hyLT5VuQ7zZ8Zc4PWb5y5GFwXk5KQ6KkKAuW7";
const FAIL_KASS_MINT = "3vQ8H7m6CqMQy7vQ9hT9b8mGwgKqpRtN5sH9JLmZTpqL";
const ORACLE_PASS_KASS = "6yLh8Y9bMTcLW2qXg5e8YZ4cWk5GFp5pZ5HfPwT5jKvN";
const ORACLE_FAIL_KASS = "4qB1tQR1Z9D6vWG2sCcTwbQGm7Yqsnz3uGCw4Hcuv7Bs";
const CV_EVENT_AUTH = "7p1Tym9SHnpZ68KhywBoQbBxAdtdAa3hKzZ4u3F8s5cQ";
const KASS_DAO = "B5y5GFwXk5KQ6KkKAuW72tFsVQ9hyLT5VuQ7zZ8Zc4PW";
const CHALLENGER_USDC_SRC = "C7zZ8Zc4PWb5y5GFwXk5KQ6KkKAuW72tFsVQ9hyLT5Vu";
const AI_CLAIM = "DqpRtN5sH9JLmZTpqL3vQ8H7m6CqMQy7vQ9hT9b8mGwg";
const PROPOSER_USDC = "EW72tFsVQ9hyLT5VuQ7zZ8Zc4PWb5y5GFwXk5KQ6KkKA";
const CHALLENGER_USDC_DEST = "FuQ7zZ8Zc4PWb5y5GFwXk5KQ6KkKAuW72tFsVQ9hyLT5";
const CHALLENGER_KASS = "GFwXk5KQ6KkKAuW72tFsVQ9hyLT5VuQ7zZ8Zc4PWb5y5";
const MARKET_ARG = "HkKAuW72tFsVQ9hyLT5VuQ7zZ8Zc4PWb5y5GFwXk5KQ6";

function bytesOf(disc: Ix, payload: number[] = []): Uint8Array {
  return new Uint8Array([disc, ...payload]);
}

function leU64(v: bigint): number[] {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigUint64(0, v, true);
  return Array.from(b);
}

function leU16(v: number): number[] {
  const b = new Uint8Array(2);
  new DataView(b.buffer).setUint16(0, v, true);
  return Array.from(b);
}

function metaTriples(keys: { pubkey: Address; isSigner: boolean; isWritable: boolean }[]) {
  return keys.map((k) => [k.pubkey.toString(), k.isSigner, k.isWritable] as const);
}

const enc = new TextEncoder();

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

describe("D3b challenge builders — open_challenge / settle_challenge", () => {
  it("openChallenge: nonce u64 payload, 25 accounts slot-by-slot", async () => {
    const nonce = 7n;
    const ix = await openChallenge({
      nonce,
      proposer: PROPOSER,
      challenger: CHALLENGER,
      question: QUESTION,
      kassVault: KASS_VAULT,
      usdcVault: USDC_VAULT,
      passAmm: PASS_AMM,
      failAmm: FAIL_AMM,
      kassVaultUnderlying: KASS_VAULT_UNDERLYING,
      passKassMint: PASS_KASS_MINT,
      failKassMint: FAIL_KASS_MINT,
      oraclePassKass: ORACLE_PASS_KASS,
      oracleFailKass: ORACLE_FAIL_KASS,
      cvEventAuthority: CV_EVENT_AUTH,
      kassDao: KASS_DAO,
      usdcMint: USDC_MINT,
      challengerUsdcSrc: CHALLENGER_USDC_SRC,
    });

    expect(ix.data).toEqual(bytesOf(Ix.OpenChallenge, leU64(nonce)));

    const oracle = await pda.oracle(nonce);
    const aiClaim = await pda.aiClaim(oracle.address, PROPOSER);
    const market = await pda.market(aiClaim.address);
    const stakeVault = await pda.stakeVault(oracle.address);
    const protocol = await pda.protocol();
    const escrow = await pda.challengeUsdcVault(market.address);

    expect(metaTriples(ix.keys)).toEqual([
      [oracle.address.toString(), false, true], // 0
      [aiClaim.address.toString(), false, true], // 1
      [PROPOSER, false, true], // 2
      [market.address.toString(), false, true], // 3
      [CHALLENGER, true, true], // 4
      [QUESTION, false, false], // 5
      [KASS_VAULT, false, true], // 6
      [USDC_VAULT, false, false], // 7
      [PASS_AMM, false, false], // 8
      [FAIL_AMM, false, false], // 9
      [stakeVault.address.toString(), false, true], // 10
      [KASS_VAULT_UNDERLYING, false, true], // 11
      [PASS_KASS_MINT, false, true], // 12
      [FAIL_KASS_MINT, false, true], // 13
      [ORACLE_PASS_KASS, false, true], // 14
      [ORACLE_FAIL_KASS, false, true], // 15
      [EXTERNAL_PROGRAM_IDS.conditionalVault.toString(), false, false], // 16
      [TOKEN_PROGRAM_ID.toString(), false, false], // 17
      [SYSTEM_PROGRAM_ID.toString(), false, false], // 18
      [CV_EVENT_AUTH, false, false], // 19
      [protocol.address.toString(), false, false], // 20
      [KASS_DAO, false, false], // 21
      [USDC_MINT, false, false], // 22
      [CHALLENGER_USDC_SRC, false, true], // 23
      [escrow.address.toString(), false, true], // 24
    ]);
    expect(ix.keys.length).toBe(25);
  });

  it("settleChallenge: nonce u64 payload, 21 accounts slot-by-slot", async () => {
    const nonce = 7n;
    const ix = await settleChallenge({
      nonce,
      aiClaim: AI_CLAIM,
      proposer: PROPOSER,
      question: QUESTION,
      passAmm: PASS_AMM,
      failAmm: FAIL_AMM,
      cvEventAuthority: CV_EVENT_AUTH,
      kassVault: KASS_VAULT,
      kassVaultUnderlying: KASS_VAULT_UNDERLYING,
      passKassMint: PASS_KASS_MINT,
      failKassMint: FAIL_KASS_MINT,
      oraclePassKass: ORACLE_PASS_KASS,
      oracleFailKass: ORACLE_FAIL_KASS,
      proposerUsdc: PROPOSER_USDC,
      challengerUsdcDest: CHALLENGER_USDC_DEST,
      challengerKass: CHALLENGER_KASS,
    });

    expect(ix.data).toEqual(bytesOf(Ix.SettleChallenge, leU64(nonce)));

    const oracle = await pda.oracle(nonce);
    const market = await pda.market(AI_CLAIM);
    const stakeVault = await pda.stakeVault(oracle.address);
    const escrow = await pda.challengeUsdcVault(market.address);

    expect(metaTriples(ix.keys)).toEqual([
      [oracle.address.toString(), false, true], // 0
      [market.address.toString(), false, true], // 1
      [AI_CLAIM, false, false], // 2
      [PROPOSER, false, true], // 3
      [QUESTION, false, true], // 4
      [PASS_AMM, false, false], // 5
      [FAIL_AMM, false, false], // 6
      [EXTERNAL_PROGRAM_IDS.conditionalVault.toString(), false, false], // 7
      [CV_EVENT_AUTH, false, false], // 8
      [TOKEN_PROGRAM_ID.toString(), false, false], // 9
      [stakeVault.address.toString(), false, true], // 10
      [KASS_VAULT, false, true], // 11
      [KASS_VAULT_UNDERLYING, false, true], // 12
      [PASS_KASS_MINT, false, true], // 13
      [FAIL_KASS_MINT, false, true], // 14
      [ORACLE_PASS_KASS, false, true], // 15
      [ORACLE_FAIL_KASS, false, true], // 16
      [escrow.address.toString(), false, true], // 17
      [PROPOSER_USDC, false, true], // 18
      [CHALLENGER_USDC_DEST, false, true], // 19
      [CHALLENGER_KASS, false, true], // 20
    ]);
    expect(ix.keys.length).toBe(21);
  });
});

describe("D3b settlement builders — claims + closes", () => {
  it("claimProposer: nonce u64 payload, 6 accounts", async () => {
    const nonce = 7n;
    const ix = await claimProposer({
      nonce,
      proposer: PROPOSER,
      destKass: DEST_KASS,
      rentRecipient: RENT_RECIPIENT,
    });

    expect(ix.data).toEqual(bytesOf(Ix.ClaimProposer, leU64(nonce)));

    const oracle = await pda.oracle(nonce);
    const stakeVault = await pda.stakeVault(oracle.address);
    expect(metaTriples(ix.keys)).toEqual([
      [oracle.address.toString(), false, false],
      [PROPOSER, false, true],
      [DEST_KASS, false, true],
      [stakeVault.address.toString(), false, true],
      [RENT_RECIPIENT, false, true],
      [TOKEN_PROGRAM_ID.toString(), false, false],
    ]);
  });

  it("claimFact: nonce u64 payload, fact at index 1", async () => {
    const nonce = 7n;
    const ix = await claimFact({
      nonce,
      fact: FACT,
      destKass: DEST_KASS,
      rentRecipient: RENT_RECIPIENT,
    });

    expect(ix.data).toEqual(bytesOf(Ix.ClaimFact, leU64(nonce)));

    const oracle = await pda.oracle(nonce);
    const stakeVault = await pda.stakeVault(oracle.address);
    expect(metaTriples(ix.keys)).toEqual([
      [oracle.address.toString(), false, false],
      [FACT, false, true],
      [DEST_KASS, false, true],
      [stakeVault.address.toString(), false, true],
      [RENT_RECIPIENT, false, true],
      [TOKEN_PROGRAM_ID.toString(), false, false],
    ]);
  });

  it("claimFactVote: nonce u64 payload, fact_vote(1) + fact(2)", async () => {
    const nonce = 7n;
    const ix = await claimFactVote({
      nonce,
      factVote: FACT_VOTE,
      fact: FACT,
      destKass: DEST_KASS,
      rentRecipient: RENT_RECIPIENT,
    });

    expect(ix.data).toEqual(bytesOf(Ix.ClaimFactVote, leU64(nonce)));

    const oracle = await pda.oracle(nonce);
    const stakeVault = await pda.stakeVault(oracle.address);
    expect(metaTriples(ix.keys)).toEqual([
      [oracle.address.toString(), false, false],
      [FACT_VOTE, false, true],
      [FACT, false, true],
      [DEST_KASS, false, true],
      [stakeVault.address.toString(), false, true],
      [RENT_RECIPIENT, false, true],
      [TOKEN_PROGRAM_ID.toString(), false, false],
    ]);
  });

  it("closeAiClaim: empty payload, oracle(ro) + ai_claim(w) + rent_recipient(w)", async () => {
    const ix = await closeAiClaim({
      oracle: ORACLE,
      aiClaim: AI_CLAIM,
      rentRecipient: RENT_RECIPIENT,
    });
    expect(ix.data).toEqual(bytesOf(Ix.CloseAiClaim));
    expect(metaTriples(ix.keys)).toEqual([
      [ORACLE, false, false],
      [AI_CLAIM, false, true],
      [RENT_RECIPIENT, false, true],
    ]);
  });

  it("closeMarket: nonce u64 payload, oracle/market/escrow/rent/token", async () => {
    const nonce = 7n;
    const ix = await closeMarket({
      nonce,
      market: MARKET_ARG,
      rentRecipient: RENT_RECIPIENT,
    });

    expect(ix.data).toEqual(bytesOf(Ix.CloseMarket, leU64(nonce)));

    const oracle = await pda.oracle(nonce);
    const escrow = await pda.challengeUsdcVault(MARKET_ARG);
    expect(metaTriples(ix.keys)).toEqual([
      [oracle.address.toString(), false, false],
      [MARKET_ARG, false, true],
      [escrow.address.toString(), false, true],
      [RENT_RECIPIENT, false, true],
      [TOKEN_PROGRAM_ID.toString(), false, false],
    ]);
  });

  it("sweepOracle: nonce u64 payload, oracle/vault/protocol/treasury/creator/token", async () => {
    const nonce = 7n;
    // daoAuthority = KASS_DAO stand-in, creator = AUTHORITY, mint = KASS_MINT.
    const ix = await sweepOracle({
      nonce,
      kassMint: KASS_MINT,
      daoAuthority: KASS_DAO,
      creator: AUTHORITY,
    });

    expect(ix.data).toEqual(bytesOf(Ix.SweepOracle, leU64(nonce)));

    const oracle = await pda.oracle(nonce);
    const stakeVault = await pda.stakeVault(oracle.address);
    const protocol = await pda.protocol();
    const daoTreasury = await pda.associatedTokenAccount(KASS_DAO, KASS_MINT);
    // Treasury is the KASS ATA of dao_authority under the ATA program.
    const [expectedAta] = await Address.findProgramAddress(
      [new Address(KASS_DAO).toBytes(), TOKEN_PROGRAM_ID.toBytes(), new Address(KASS_MINT).toBytes()],
      new Address("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"),
    );
    expect(daoTreasury.address.toString()).toBe(expectedAta.toString());

    expect(metaTriples(ix.keys)).toEqual([
      [oracle.address.toString(), false, true],
      [stakeVault.address.toString(), false, true],
      [protocol.address.toString(), false, false],
      [daoTreasury.address.toString(), false, true],
      [AUTHORITY, false, true],
      [TOKEN_PROGRAM_ID.toString(), false, false],
    ]);
  });

  it("respects a programId override", async () => {
    const other = new Address("Vote111111111111111111111111111111111111111");
    const ix = await closeAiClaim({
      oracle: ORACLE,
      aiClaim: AI_CLAIM,
      rentRecipient: RENT_RECIPIENT,
      programId: other,
    });
    expect(ix.programId.toString()).toBe(other.toString());
  });
});

/**
 * D3b (cont.) — instruction-builder byte/meta tests for the challenge
 * (open/settle) + settlement (claims + closes) builders. Split out of
 * instructions-dispute.test.ts; shared fixtures/helpers live in
 * ./helpers/instructions-dispute.ts.
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
  openChallenge,
  settleChallenge,
  sweepOracle,
} from "../src/instructions/index.js";
import {
  AI_CLAIM,
  AUTHORITY,
  CHALLENGER,
  CHALLENGER_KASS,
  CHALLENGER_USDC_DEST,
  CHALLENGER_USDC_SRC,
  CV_EVENT_AUTH,
  DEST_KASS,
  FACT,
  FAIL_AMM,
  FAIL_KASS_MINT,
  FACT_VOTE,
  KASS_DAO,
  KASS_MINT,
  KASS_VAULT,
  KASS_VAULT_UNDERLYING,
  MARKET_ARG,
  ORACLE,
  ORACLE_FAIL_KASS,
  ORACLE_PASS_KASS,
  PASS_AMM,
  PASS_KASS_MINT,
  PROPOSER,
  PROPOSER_USDC,
  QUESTION,
  RENT_RECIPIENT,
  USDC_MINT,
  USDC_VAULT,
  bytesOf,
  leU64,
  metaTriples,
} from "./helpers/instructions-dispute.js";

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

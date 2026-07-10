/**
 * SD1 offline unit tests for the DERIVE-FROM-MARKET one-click settle.
 *
 * `buildSettleFromMarketIxs` derives the full 15-account settle set client-side
 * from a DECODED {@link Market} + {@link Oracle} (+ the proposer authority) and
 * calls RF4's `buildSettleChallengeIxs`. These tests assert, fully offline:
 *
 *   - the settle ix `data` + `keys` BYTE-MATCH `buildSettleChallengeIxs` called
 *     with the HAND-SUPPLIED equivalent accounts — i.e. the derivation lands the
 *     SAME 21-account set the SDK builds;
 *   - each derived account == its expected PDA/ATA (passKassMint ==
 *     conditionalTokenMint(kassVault,0), proposerUsdc == ATA(proposerAuthority,
 *     usdcMint), challengerUsdcDest == ATA(challenger, usdcMint) [NOT the escrow],
 *     cvEventAuthority == vaultEventAuthority(), kassVaultUnderlying ==
 *     ATA(kassVault, kassMint));
 *   - the optional idempotent payout-ATA creates (connection + payer) prepend
 *     exactly 3 create-ATA ixs;
 *   - validation: a settled market / a missing market / a missing proposer
 *     authority reject.
 */
import { Address, Keypair, TransactionInstruction, type Connection } from "@solana/web3.js";
import {
  AccountType,
  ACCOUNT_SIZES,
  associatedTokenAccount,
  decodeMarket,
  decodeOracle,
  futarchy,
  type Market,
  type Oracle,
} from "@kassandra-market/oracles";
import { describe, expect, it } from "vitest";

import { ValidationError } from "../src/data/actions.ts";
import { conditionalTokenMint } from "../src/data/actions/challengeTrade.ts";
import { buildSettleChallengeIxs } from "../src/data/actions/challenge.ts";
import { buildSettleFromMarketIxs } from "../src/data/actions/challengeSettle.ts";

const ATA_PROGRAM_ID = new Address("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

function keyShape(ix: TransactionInstruction) {
  return ix.keys.map((k) => ({
    pubkey: k.pubkey.toString(),
    isSigner: k.isSigner,
    isWritable: k.isWritable,
  }));
}

/** A connection whose ATA-existence check always reports absent. */
function fakeConnection(): Connection {
  return { getAccountInfo: async () => null } as unknown as Connection;
}

/**
 * Build a genuine decoded-shaped {@link Market} by writing a byte buffer at the
 * pinned field offsets and DECODING it (proves the derive path off real decode).
 */
function makeMarket(fields: {
  oracle: Address;
  aiClaim: Address;
  proposer: Address;
  challenger: Address;
  question: Address;
  kassVault: Address;
  usdcVault: Address;
  passAmm: Address;
  failAmm: Address;
  oraclePassKass: Address;
  oracleFailKass: Address;
  challengerUsdcVault: Address;
  settled?: boolean;
}): Market {
  const data = new Uint8Array(ACCOUNT_SIZES.Market);
  data[0] = AccountType.Market;
  const put = (off: number, a: Address) => data.set(a.toBytes(), off);
  put(8, fields.oracle);
  put(40, fields.aiClaim);
  put(72, fields.proposer);
  put(104, fields.challenger);
  put(136, fields.question);
  put(168, fields.kassVault);
  put(200, fields.usdcVault);
  put(232, fields.passAmm);
  put(264, fields.failAmm);
  put(296, fields.oraclePassKass);
  put(328, fields.oracleFailKass);
  put(360, fields.challengerUsdcVault);
  const dv = new DataView(data.buffer);
  dv.setBigInt64(392, 1_000n, true); // twapEnd
  dv.setBigUint64(400, 500_000n, true); // challengerUsdc
  data[408] = fields.settled ? 1 : 0;
  data[409] = 255; // bump
  return decodeMarket(data);
}

/** Build a genuine decoded-shaped {@link Oracle} (only kassMint/usdcMint matter). */
function makeOracle(kassMint: Address, usdcMint: Address, stakeVault: Address): Oracle {
  const data = new Uint8Array(ACCOUNT_SIZES.Oracle);
  data[0] = AccountType.Oracle;
  data.set(kassMint.toBytes(), 40);
  data.set(usdcMint.toBytes(), 72);
  data.set(stakeVault.toBytes(), 104);
  return decodeOracle(data);
}

async function fixture() {
  const g = async () => (await Keypair.generate()).publicKey;
  const oracle = await g();
  const aiClaim = await g();
  const proposer = await g();
  const proposerAuthority = await g();
  const challenger = await g();
  const question = await g();
  const kassVault = await g();
  const usdcVault = await g();
  const passAmm = await g();
  const failAmm = await g();
  const oraclePassKass = await g();
  const oracleFailKass = await g();
  const challengerUsdcVault = await g();
  const kassMint = await g();
  const usdcMint = await g();
  const stakeVault = await g();

  const market = makeMarket({
    oracle,
    aiClaim,
    proposer,
    challenger,
    question,
    kassVault,
    usdcVault,
    passAmm,
    failAmm,
    oraclePassKass,
    oracleFailKass,
    challengerUsdcVault,
  });
  const oracleAcct = makeOracle(kassMint, usdcMint, stakeVault);
  return {
    market,
    oracleAcct,
    proposerAuthority,
    kassMint,
    usdcMint,
    challenger,
    proposer,
    challengerUsdcVault,
  };
}

const NONCE = 200n;

describe("buildSettleFromMarketIxs — derive-from-Market one-click settle", () => {
  it("byte-matches buildSettleChallengeIxs with the hand-supplied equivalent 21 accounts", async () => {
    const f = await fixture();
    const m = f.market;

    // Hand-derive the 15 caller accounts (the equivalent hand-supplied set).
    const [passKassMint, failKassMint, kassVaultUnderlying, cvEventAuthority] = await Promise.all([
      conditionalTokenMint(m.kassVault, 0),
      conditionalTokenMint(m.kassVault, 1),
      associatedTokenAccount(m.kassVault, f.kassMint).then((p) => p.address),
      futarchy.pda.vaultEventAuthority().then((p) => p.address),
    ]);
    const [proposerUsdc, challengerUsdcDest, challengerKass] = await Promise.all([
      associatedTokenAccount(f.proposerAuthority, f.usdcMint).then((p) => p.address),
      associatedTokenAccount(f.challenger, f.usdcMint).then((p) => p.address),
      associatedTokenAccount(f.challenger, f.kassMint).then((p) => p.address),
    ]);

    const [expected] = await buildSettleChallengeIxs({
      oracleNonce: NONCE,
      aiClaim: m.aiClaim,
      proposer: m.proposer,
      question: m.question,
      passAmm: m.passAmm,
      failAmm: m.failAmm,
      cvEventAuthority,
      kassVault: m.kassVault,
      kassVaultUnderlying,
      passKassMint,
      failKassMint,
      oraclePassKass: m.oraclePassKass,
      oracleFailKass: m.oracleFailKass,
      proposerUsdc,
      challengerUsdcDest,
      challengerKass,
    });

    const derived = await buildSettleFromMarketIxs({
      oracleNonce: NONCE,
      market: m,
      oracle: f.oracleAcct,
      proposerAuthority: f.proposerAuthority,
    });

    // Exactly one ix (no ATA creates without a connection), byte-for-byte equal.
    expect(derived).toHaveLength(1);
    expect(derived[0].programId.toString()).toBe(expected.programId.toString());
    expect(Array.from(derived[0].data)).toEqual(Array.from(expected.data));
    expect(keyShape(derived[0])).toEqual(keyShape(expected));
    // 21 accounts total.
    expect(derived[0].keys).toHaveLength(21);
  });

  it("derives each caller account to its expected PDA/ATA", async () => {
    const f = await fixture();
    const m = f.market;

    const [ix] = await buildSettleFromMarketIxs({
      oracleNonce: NONCE,
      market: m,
      oracle: f.oracleAcct,
      proposerAuthority: f.proposerAuthority,
    });
    const keys = ix.keys.map((k) => k.pubkey.toString());

    // Settle account slot map (see sdk settleChallenge): 13 pass_kass_mint,
    // 14 fail_kass_mint, 12 kass_vault_underlying, 8 cv_event_authority,
    // 18 proposer_usdc, 19 challenger_usdc_dest, 20 challenger_kass.
    const passKassMint = await conditionalTokenMint(m.kassVault, 0);
    const failKassMint = await conditionalTokenMint(m.kassVault, 1);
    const kassVaultUnderlying = (await associatedTokenAccount(m.kassVault, f.kassMint)).address;
    const cvEventAuthority = (await futarchy.pda.vaultEventAuthority()).address;
    const proposerUsdc = (await associatedTokenAccount(f.proposerAuthority, f.usdcMint)).address;
    const challengerUsdcDest = (await associatedTokenAccount(f.challenger, f.usdcMint)).address;
    const challengerKass = (await associatedTokenAccount(f.challenger, f.kassMint)).address;

    expect(keys[13]).toBe(passKassMint.toString());
    expect(keys[14]).toBe(failKassMint.toString());
    expect(keys[12]).toBe(kassVaultUnderlying.toString());
    expect(keys[8]).toBe(cvEventAuthority.toString());
    expect(keys[18]).toBe(proposerUsdc.toString());
    expect(keys[19]).toBe(challengerUsdcDest.toString());
    expect(keys[20]).toBe(challengerKass.toString());

    // The Market-direct accounts thread straight through.
    expect(keys[2]).toBe(m.aiClaim.toString()); // ai_claim
    expect(keys[3]).toBe(m.proposer.toString()); // proposer
    expect(keys[4]).toBe(m.question.toString()); // question
    expect(keys[5]).toBe(m.passAmm.toString()); // pass_amm
    expect(keys[6]).toBe(m.failAmm.toString()); // fail_amm
    expect(keys[11]).toBe(m.kassVault.toString()); // kass_vault
    expect(keys[15]).toBe(m.oraclePassKass.toString());
    expect(keys[16]).toBe(m.oracleFailKass.toString());
  });

  it("challengerUsdcDest is the challenger ATA, NOT the market escrow (account 17 vs 19)", async () => {
    const f = await fixture();
    const [ix] = await buildSettleFromMarketIxs({
      oracleNonce: NONCE,
      market: f.market,
      oracle: f.oracleAcct,
      proposerAuthority: f.proposerAuthority,
    });
    const keys = ix.keys.map((k) => k.pubkey.toString());
    const challengerUsdcDest = (await associatedTokenAccount(f.challenger, f.usdcMint)).address;
    // Account 19 (dest) is the challenger's own USDC ATA.
    expect(keys[19]).toBe(challengerUsdcDest.toString());
    // The escrow (market.challengerUsdcVault) is a DIFFERENT account (slot 17,
    // SDK-derived from the market PDA — here it is NOT the challenger ATA).
    expect(keys[19]).not.toBe(f.challengerUsdcVault.toString());
    expect(f.challengerUsdcVault.toString()).not.toBe(challengerUsdcDest.toString());
  });

  it("prepends 3 idempotent payout-ATA creates when a connection + payer are supplied", async () => {
    const f = await fixture();
    const payer = (await Keypair.generate()).publicKey;
    const ixs = await buildSettleFromMarketIxs({
      connection: fakeConnection(),
      oracleNonce: NONCE,
      market: f.market,
      oracle: f.oracleAcct,
      proposerAuthority: f.proposerAuthority,
      payer,
    });
    expect(ixs).toHaveLength(4); // 3 create-ATA + 1 settle
    for (const create of ixs.slice(0, 3)) {
      expect(create.programId.toString()).toBe(ATA_PROGRAM_ID.toString());
      expect(Array.from(create.data)).toEqual([1]); // createIdempotent discriminant
      expect(create.keys[0].pubkey.toString()).toBe(payer.toString());
      expect(create.keys[0].isSigner).toBe(true);
    }
  });

  it("rejects a settled market / a missing market / a missing proposer authority", async () => {
    const f = await fixture();
    const settled = makeMarket({
      oracle: f.market.oracle,
      aiClaim: f.market.aiClaim,
      proposer: f.market.proposer,
      challenger: f.challenger,
      question: f.market.question,
      kassVault: f.market.kassVault,
      usdcVault: f.market.usdcVault,
      passAmm: f.market.passAmm,
      failAmm: f.market.failAmm,
      oraclePassKass: f.market.oraclePassKass,
      oracleFailKass: f.market.oracleFailKass,
      challengerUsdcVault: f.challengerUsdcVault,
      settled: true,
    });
    await expect(
      buildSettleFromMarketIxs({
        oracleNonce: NONCE,
        market: settled,
        oracle: f.oracleAcct,
        proposerAuthority: f.proposerAuthority,
      }),
    ).rejects.toBeInstanceOf(ValidationError);

    await expect(
      buildSettleFromMarketIxs({
        oracleNonce: NONCE,
        market: undefined as unknown as Market,
        oracle: f.oracleAcct,
        proposerAuthority: f.proposerAuthority,
      }),
    ).rejects.toBeInstanceOf(ValidationError);

    await expect(
      buildSettleFromMarketIxs({
        oracleNonce: NONCE,
        market: f.market,
        oracle: f.oracleAcct,
        proposerAuthority: undefined as unknown as Address,
      }),
    ).rejects.toBeInstanceOf(ValidationError);
  });
});

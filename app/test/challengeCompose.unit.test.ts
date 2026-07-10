/**
 * CU3 offline unit tests for the CLIENT-SIDE challenge-market composition.
 *
 * `buildComposeAndOpenChallengeIxs` returns an ORDERED sequence of single-tx
 * {@link ComposeStep}s (question → 2 vaults → fund+split → 2 pool seeds → open).
 * These tests assert, fully offline against a fake `Connection`:
 *
 *   - the STEP order + ids + grouping (7 steps in the exact choreography order);
 *   - the key ixs byte-for-byte (`data` + `keys`) against the SDK builders for the
 *     SAME derived inputs — initializeQuestion / initializeConditionalVault (KASS
 *     + USDC) / splitTokens (KASS + USDC) / createAmm / addLiquidity / openChallenge;
 *   - the PDA / ATA derivations (oracle / question / vaults / conditional mints /
 *     amm / oracle-owned holder ATAs / challenger conditional ATAs);
 *   - the twap_initial_observation + seed-liquidity math (mirror buildPool);
 *   - validation (bad nonce / bad address / non-positive reserves / bad question id).
 */
import { Address, Keypair, TransactionInstruction, type Connection } from "@solana/web3.js";
import {
  EXTERNAL_PROGRAM_IDS,
  ammV04,
  associatedTokenAccount,
  futarchy,
  pda,
} from "@kassandra-market/oracles";
import { describe, expect, it } from "vitest";

import { ValidationError } from "../src/data/actions.ts";
import {
  DEFAULT_BASE_RESERVE,
  DEFAULT_QUESTION_ID,
  DEFAULT_QUOTE_RESERVE,
  MAX_OBSERVATION_CHANGE,
  PRICE_SCALE,
  buildComposeAndOpenChallengeIxs,
  twapInitialObservation,
} from "../src/data/actions/challengeCompose.ts";
import { buildOpenChallengeIxs } from "../src/data/actions/challenge.ts";

const enc = new TextEncoder();
const VLTX = EXTERNAL_PROGRAM_IDS.conditionalVault;

/** A connection whose ATA-existence check always reports absent (create fires). */
function fakeConnection(): Connection {
  return { getAccountInfo: async () => null } as unknown as Connection;
}

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

/** A deterministic fixture: the nonce, proposer, challenger, mints, dao. */
async function fixture() {
  const nonce = 100n;
  const proposer = (await Keypair.generate()).publicKey;
  const challenger = (await Keypair.generate()).publicKey;
  const kassMint = (await Keypair.generate()).publicKey;
  const usdcMint = (await Keypair.generate()).publicKey;
  const kassDao = (await Keypair.generate()).publicKey;
  return { nonce, proposer, challenger, kassMint, usdcMint, kassDao };
}

async function build(over: Partial<Parameters<typeof buildComposeAndOpenChallengeIxs>[0]> = {}) {
  const f = await fixture();
  return buildComposeAndOpenChallengeIxs({
    connection: fakeConnection(),
    oracleNonce: f.nonce,
    proposer: f.proposer,
    challenger: f.challenger,
    kassMint: f.kassMint,
    usdcMint: f.usdcMint,
    kassDao: f.kassDao,
    ...over,
  });
}

describe("compose — the step sequence + grouping", () => {
  it("returns the 7 ordered steps in the exact choreography order", async () => {
    const { steps } = await build();
    expect(steps.map((s) => s.id)).toEqual([
      "question",
      "kass-vault",
      "usdc-vault",
      "fund-split",
      "pass-pool",
      "fail-pool",
      "open",
    ]);
    // Each labelled + non-empty, single-purpose groups.
    for (const s of steps) {
      expect(s.label.length).toBeGreaterThan(0);
      expect(s.ixs.length).toBeGreaterThan(0);
    }
    // The fund+split step bundles 6 ATA-creates + 2 splits = 8 ixs.
    expect(steps.find((s) => s.id === "fund-split")!.ixs.length).toBe(8);
    // Each pool step = createAmm + create-LP-ATA + addLiquidity.
    expect(steps.find((s) => s.id === "pass-pool")!.ixs.length).toBe(3);
    expect(steps.find((s) => s.id === "fail-pool")!.ixs.length).toBe(3);
    // open = a single open_challenge ix.
    expect(steps.find((s) => s.id === "open")!.ixs.length).toBe(1);
  });
});

describe("compose — PDA / ATA derivations", () => {
  it("derives oracle / question / vaults / conditional mints / amms / holders", async () => {
    const f = await fixture();
    const { composed } = await build({
      oracleNonce: f.nonce,
      proposer: f.proposer,
      challenger: f.challenger,
      kassMint: f.kassMint,
      usdcMint: f.usdcMint,
      kassDao: f.kassDao,
    });

    const oracle = (await pda.oracle(f.nonce)).address;
    expect(composed.oracle.toString()).toBe(oracle.toString());

    const question = (await futarchy.pda.question(DEFAULT_QUESTION_ID, oracle, 2)).address;
    expect(composed.question.toString()).toBe(question.toString());

    const kassVault = (await futarchy.pda.conditionalVault(question, f.kassMint)).address;
    const usdcVault = (await futarchy.pda.conditionalVault(question, f.usdcMint)).address;
    expect(composed.kassVault.toString()).toBe(kassVault.toString());
    expect(composed.usdcVault.toString()).toBe(usdcVault.toString());

    // conditional-token mints [b"conditional_token", vault, index].
    for (const [vault, idx, got] of [
      [kassVault, 0, composed.passKassMint],
      [kassVault, 1, composed.failKassMint],
      [usdcVault, 0, composed.passUsdcMint],
      [usdcVault, 1, composed.failUsdcMint],
    ] as const) {
      const [exp] = await Address.findProgramAddress(
        [enc.encode("conditional_token"), vault.toBytes(), Uint8Array.of(idx)],
        VLTX,
      );
      expect(got.toString()).toBe(exp.toString());
    }

    // pool PDAs = amm(condKass, condUsdc) for each side.
    const passAmm = (await ammV04.pda.amm(composed.passKassMint, composed.passUsdcMint)).address;
    const failAmm = (await ammV04.pda.amm(composed.failKassMint, composed.failUsdcMint)).address;
    expect(composed.passAmm.toString()).toBe(passAmm.toString());
    expect(composed.failAmm.toString()).toBe(failAmm.toString());

    // oracle-owned pass/fail KASS holders = ATA(oracle, condKassMint).
    expect(composed.oraclePassKass.toString()).toBe(
      (await associatedTokenAccount(oracle, composed.passKassMint)).address.toString(),
    );
    expect(composed.oracleFailKass.toString()).toBe(
      (await associatedTokenAccount(oracle, composed.failKassMint)).address.toString(),
    );
    // challenger USDC source = ATA(challenger, usdcMint).
    expect(composed.challengerUsdcSrc.toString()).toBe(
      (await associatedTokenAccount(f.challenger, f.usdcMint)).address.toString(),
    );
  });
});

describe("compose — the key ixs byte-match the SDK builders", () => {
  it("step 1 == futarchy.initializeQuestion(binary, resolver == oracle)", async () => {
    const f = await fixture();
    const { steps, composed } = await build({
      oracleNonce: f.nonce,
      proposer: f.proposer,
      challenger: f.challenger,
      kassMint: f.kassMint,
      usdcMint: f.usdcMint,
      kassDao: f.kassDao,
    });
    const expected = await futarchy.initializeQuestion({
      questionId: DEFAULT_QUESTION_ID,
      oracle: composed.oracle,
      numOutcomes: 2,
      payer: f.challenger,
    });
    expectIxMatches(steps[0].ixs[0], expected);
  });

  it("steps 2/3 == futarchy.initializeConditionalVault(KASS) / (USDC)", async () => {
    const f = await fixture();
    const { steps } = await build({
      oracleNonce: f.nonce,
      proposer: f.proposer,
      challenger: f.challenger,
      kassMint: f.kassMint,
      usdcMint: f.usdcMint,
      kassDao: f.kassDao,
    });
    const oracle = (await pda.oracle(f.nonce)).address;
    const question = (await futarchy.pda.question(DEFAULT_QUESTION_ID, oracle, 2)).address;
    const expKass = await futarchy.initializeConditionalVault({
      question,
      underlyingMint: f.kassMint,
      payer: f.challenger,
      numOutcomes: 2,
    });
    const expUsdc = await futarchy.initializeConditionalVault({
      question,
      underlyingMint: f.usdcMint,
      payer: f.challenger,
      numOutcomes: 2,
    });
    expectIxMatches(steps[1].ixs[0], expKass);
    expectIxMatches(steps[2].ixs[0], expUsdc);
  });

  it("step 4 splits == futarchy.splitTokens(KASS baseReserve) / (USDC quoteReserve)", async () => {
    const f = await fixture();
    const { steps, composed } = await build({
      oracleNonce: f.nonce,
      proposer: f.proposer,
      challenger: f.challenger,
      kassMint: f.kassMint,
      usdcMint: f.usdcMint,
      kassDao: f.kassDao,
    });
    const fundSplit = steps.find((s) => s.id === "fund-split")!;
    // The two split ixs are the last two of the group.
    const [splitKass, splitUsdc] = fundSplit.ixs.slice(-2);

    const challengerKass = (await associatedTokenAccount(f.challenger, f.kassMint)).address;
    const challengerPassKass = (await associatedTokenAccount(f.challenger, composed.passKassMint)).address;
    const challengerFailKass = (await associatedTokenAccount(f.challenger, composed.failKassMint)).address;
    const expSplitKass = await futarchy.splitTokens({
      question: composed.question,
      vault: composed.kassVault,
      vaultUnderlying: composed.kassVaultUnderlying,
      authority: f.challenger,
      userUnderlying: challengerKass,
      conditionalMints: [composed.passKassMint, composed.failKassMint],
      userConditionalAccounts: [challengerPassKass, challengerFailKass],
      amount: DEFAULT_BASE_RESERVE,
    });
    expectIxMatches(splitKass, expSplitKass);

    const challengerPassUsdc = (await associatedTokenAccount(f.challenger, composed.passUsdcMint)).address;
    const challengerFailUsdc = (await associatedTokenAccount(f.challenger, composed.failUsdcMint)).address;
    const expSplitUsdc = await futarchy.splitTokens({
      question: composed.question,
      vault: composed.usdcVault,
      vaultUnderlying: composed.usdcVaultUnderlying,
      authority: f.challenger,
      userUnderlying: composed.challengerUsdcSrc,
      conditionalMints: [composed.passUsdcMint, composed.failUsdcMint],
      userConditionalAccounts: [challengerPassUsdc, challengerFailUsdc],
      amount: DEFAULT_QUOTE_RESERVE,
    });
    expectIxMatches(splitUsdc, expSplitUsdc);
  });

  it("steps 5/6 == ammV04.createAmm + addLiquidity with the buildPool math", async () => {
    const f = await fixture();
    const { steps, composed } = await build({
      oracleNonce: f.nonce,
      proposer: f.proposer,
      challenger: f.challenger,
      kassMint: f.kassMint,
      usdcMint: f.usdcMint,
      kassDao: f.kassDao,
    });
    const initialObs = twapInitialObservation(DEFAULT_BASE_RESERVE, DEFAULT_QUOTE_RESERVE);
    // buildPool: initialObs = quote·1e12/base.
    expect(initialObs).toBe((DEFAULT_QUOTE_RESERVE * PRICE_SCALE) / DEFAULT_BASE_RESERVE);

    for (const [side, base, quote, poolIxs] of [
      ["pass", composed.passKassMint, composed.passUsdcMint, steps.find((s) => s.id === "pass-pool")!.ixs],
      ["fail", composed.failKassMint, composed.failUsdcMint, steps.find((s) => s.id === "fail-pool")!.ixs],
    ] as const) {
      const expCreate = await ammV04.createAmm({
        payer: f.challenger,
        baseMint: base,
        quoteMint: quote,
        twapInitialObservation: initialObs,
        twapMaxObservationChangePerUpdate: MAX_OBSERVATION_CHANGE,
        twapStartDelaySlots: 0n,
      });
      const expAdd = await ammV04.addLiquidity({
        payer: f.challenger,
        baseMint: base,
        quoteMint: quote,
        quoteAmount: DEFAULT_QUOTE_RESERVE,
        maxBaseAmount: DEFAULT_BASE_RESERVE,
        minLpTokens: 0n,
      });
      expectIxMatches(poolIxs[0], expCreate);
      // poolIxs[1] is the idempotent LP-ATA create (disc 1); addLiquidity is [2].
      expect(Array.from(poolIxs[1].data)).toEqual([1]);
      expectIxMatches(poolIxs[2], expAdd);
      expect(side).toBeDefined();
    }
  });

  it("step 7 == buildOpenChallengeIxs fed the composed account set", async () => {
    const f = await fixture();
    const { steps, composed } = await build({
      oracleNonce: f.nonce,
      proposer: f.proposer,
      challenger: f.challenger,
      kassMint: f.kassMint,
      usdcMint: f.usdcMint,
      kassDao: f.kassDao,
    });
    const cvEventAuthority = (await futarchy.pda.vaultEventAuthority()).address;
    const [expOpen] = await buildOpenChallengeIxs({
      oracleNonce: f.nonce,
      proposer: f.proposer,
      challenger: f.challenger,
      question: composed.question,
      kassVault: composed.kassVault,
      usdcVault: composed.usdcVault,
      passAmm: composed.passAmm,
      failAmm: composed.failAmm,
      kassVaultUnderlying: composed.kassVaultUnderlying,
      passKassMint: composed.passKassMint,
      failKassMint: composed.failKassMint,
      oraclePassKass: composed.oraclePassKass,
      oracleFailKass: composed.oracleFailKass,
      cvEventAuthority,
      kassDao: f.kassDao,
      usdcMint: f.usdcMint,
      challengerUsdcSrc: composed.challengerUsdcSrc,
    });
    expectIxMatches(steps.find((s) => s.id === "open")!.ixs[0], expOpen);
  });
});

describe("compose — the fund+split ATA-creates", () => {
  it("prepends idempotent ATA-creates for the oracle holders + challenger conditional accounts", async () => {
    const { steps, composed } = await build();
    const creates = steps.find((s) => s.id === "fund-split")!.ixs.slice(0, 6);
    // All 6 are ATA-program create-idempotent ixs (disc 1).
    for (const ix of creates) {
      expect(ix.programId.toString()).toBe(
        "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
      );
      expect(Array.from(ix.data)).toEqual([1]);
    }
    // The oracle-holder creates carry owner == oracle (key[2]).
    expect(creates[0].keys[2].pubkey.toString()).toBe(composed.oracle.toString());
    expect(creates[0].keys[1].pubkey.toString()).toBe(composed.oraclePassKass.toString());
    expect(creates[1].keys[1].pubkey.toString()).toBe(composed.oracleFailKass.toString());
  });
});

describe("compose — the seed math", () => {
  it("twapInitialObservation scales quote by PRICE_SCALE over base", () => {
    expect(twapInitialObservation(100_000_000_000n, 100_000_000n)).toBe(1_000_000_000n);
    expect(twapInitialObservation(200n, 400n)).toBe((400n * PRICE_SCALE) / 200n);
  });

  it("MAX_OBSERVATION_CHANGE == (2^64-1)·1e12", () => {
    expect(MAX_OBSERVATION_CHANGE).toBe(((1n << 64n) - 1n) * PRICE_SCALE);
  });

  it("custom reserves flow through to the createAmm observation + addLiquidity", async () => {
    const f = await fixture();
    const baseReserve = 50_000_000_000n;
    const quoteReserve = 75_000_000n;
    const { steps, composed } = await build({
      oracleNonce: f.nonce,
      proposer: f.proposer,
      challenger: f.challenger,
      kassMint: f.kassMint,
      usdcMint: f.usdcMint,
      kassDao: f.kassDao,
      baseReserve,
      quoteReserve,
    });
    const expCreate = await ammV04.createAmm({
      payer: f.challenger,
      baseMint: composed.passKassMint,
      quoteMint: composed.passUsdcMint,
      twapInitialObservation: twapInitialObservation(baseReserve, quoteReserve),
      twapMaxObservationChangePerUpdate: MAX_OBSERVATION_CHANGE,
      twapStartDelaySlots: 0n,
    });
    expectIxMatches(steps.find((s) => s.id === "pass-pool")!.ixs[0], expCreate);
  });
});

describe("compose — validation", () => {
  const bad = async (over: Record<string, unknown>) => {
    try {
      await build(over as never);
      return null;
    } catch (e) {
      return e;
    }
  };

  it("rejects a missing/negative oracle nonce", async () => {
    expect(await bad({ oracleNonce: -1 })).toBeInstanceOf(ValidationError);
  });
  it("rejects a bad challenger address", async () => {
    expect(await bad({ challenger: "not-base58!!!" })).toBeInstanceOf(ValidationError);
  });
  it("rejects non-positive reserves", async () => {
    expect(await bad({ baseReserve: 0n })).toBeInstanceOf(ValidationError);
    expect(await bad({ quoteReserve: 0n })).toBeInstanceOf(ValidationError);
  });
  it("rejects a wrong-length question id", async () => {
    expect(await bad({ questionId: new Uint8Array(16) })).toBeInstanceOf(ValidationError);
  });
});

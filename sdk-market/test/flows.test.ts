/**
 * Offline unit tests for the high-level flows — no chain, no network.
 * Covers: compose twap constants, buy/sell swap direction + instruction shape,
 * the Jupiter request shape, and the `composeWithEntry` combiner ordering.
 */
import { describe, expect, it } from "vitest";

import { Address, TransactionInstruction } from "@solana/web3.js";

import { EXTERNAL_PROGRAM_IDS } from "../src/constants.js";
import { DISC, SwapType } from "../src/metadao/index.js";
import * as pda from "../src/pda.js";
import {
  JUPITER_V6_BASE_URL,
  TWAP_INITIAL_OBSERVATION,
  TWAP_MAX_OBSERVATION_CHANGE_PER_UPDATE,
  TWAP_START_DELAY_SLOTS,
  buildJupiterEntryRequest,
  buyInstructions,
  composeMarketInstructions,
  createAllOutcomeMarkets,
  composeWithEntry,
  ensureConditionalAtasInstructions,
  redeemInstructions,
  sellInstructions,
  type MarketRefs,
} from "../src/flows/index.js";
import { ATA_PROGRAM_ID } from "../src/constants.js";

const KASS = new Address("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
const ORACLE = new Address("KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY");
const USER = new Address("11111111111111111111111111111112");
const USDC = new Address("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt2w");

async function fakeRefs(): Promise<MarketRefs> {
  const market = (await pda.market(ORACLE, 0)).address;
  const { refs } = await composeMarketInstructions({ market, oracle: ORACLE, kassMint: KASS, payer: USER });
  return refs;
}

/** Read a u128 LE from an ix data buffer. */
function u128le(data: Uint8Array, off: number): bigint {
  const dv = new DataView(data.buffer, data.byteOffset, data.length);
  return dv.getBigUint64(off, true) | (dv.getBigUint64(off + 8, true) << 64n);
}

describe("flows/compose", () => {
  it("pins the Rust-harness twap/observation constants for a valid empty 50/50 seed", () => {
    expect(TWAP_INITIAL_OBSERVATION).toBe(1_000_000_000_000n);
    expect(TWAP_MAX_OBSERVATION_CHANGE_PER_UPDATE).toBe((2n ** 64n - 1n) * 1_000_000_000_000n);
    expect(TWAP_START_DELAY_SLOTS).toBe(0n);
  });

  it("emits initializeQuestion → initializeConditionalVault → createAmm with those twap args", async () => {
    const market = (await pda.market(ORACLE, 0)).address;
    const { instructions, refs } = await composeMarketInstructions({
      market,
      oracle: ORACLE,
      kassMint: KASS,
      payer: USER,
    });
    expect(instructions).toHaveLength(3);
    // ix[2] == create_amm; its data is disc[8] ++ u128 init ++ u128 maxChange ++ u64 delay.
    const ammData = instructions[2].data;
    expect(Array.from(ammData.slice(0, 8))).toEqual(Array.from(DISC.createAmm));
    expect(u128le(ammData, 8)).toBe(TWAP_INITIAL_OBSERVATION);
    expect(u128le(ammData, 24)).toBe(TWAP_MAX_OBSERVATION_CHANGE_PER_UPDATE);
    // refs carry everything activate needs.
    expect(refs.question).toBeDefined();
    expect(refs.marketCyes).toBeDefined();
    expect(refs.cvEventAuthority).toBeDefined();
  });
});

describe("flows/trade", () => {
  it("buy YES = split + swap(cNO→cYES, Buy)", async () => {
    const refs = await fakeRefs();
    const userKassAta = new Address("So11111111111111111111111111111111111111112");
    const { instructions } = await buyInstructions({
      refs,
      user: USER,
      outcome: "yes",
      kassAmount: 1000n,
      userKassAta,
    });
    expect(instructions).toHaveLength(2);
    expect(Array.from(instructions[0].data.slice(0, 8))).toEqual(Array.from(DISC.splitTokens));
    const swap = instructions[1].data;
    expect(Array.from(swap.slice(0, 8))).toEqual(Array.from(DISC.swap));
    expect(swap[8]).toBe(SwapType.Buy); // quote→base = cNO→cYES
  });

  it("threads the SAME conditional-ATA overrides into BOTH split and swap", async () => {
    const refs = await fakeRefs();
    const userKassAta = new Address("So11111111111111111111111111111111111111112");
    // Deliberately NON-ATA override accounts (as a fabricated/test wallet would use).
    const userYesAta = USDC;
    const userNoAta = ORACLE;
    const { instructions } = await buyInstructions({
      refs,
      user: USER,
      outcome: "yes",
      kassAmount: 1000n,
      userKassAta,
      userYesAta,
      userNoAta,
    });
    const [split, swap] = instructions;
    // split's user conditional accounts are its last two keys: [cYES, cNO].
    const splitYes = split.keys[split.keys.length - 2].pubkey.toString();
    const splitNo = split.keys[split.keys.length - 1].pubkey.toString();
    // swap's user base/quote are keys[2]/keys[3] (base == cYES, quote == cNO).
    const swapBase = swap.keys[2].pubkey.toString();
    const swapQuote = swap.keys[3].pubkey.toString();
    expect(splitYes).toBe(userYesAta.toString());
    expect(splitNo).toBe(userNoAta.toString());
    expect(swapBase).toBe(splitYes); // the least-proven property: they MUST agree
    expect(swapQuote).toBe(splitNo);
  });

  it("throws on an invalid outcome", async () => {
    const refs = await fakeRefs();
    const userKassAta = new Address("So11111111111111111111111111111111111111112");
    await expect(
      buyInstructions({
        refs,
        user: USER,
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        outcome: "maybe" as any,
        kassAmount: 1000n,
        userKassAta,
      }),
    ).rejects.toThrow(/outcome must be/);
  });

  it("buy NO = split + swap(cYES→cNO, Sell)", async () => {
    const refs = await fakeRefs();
    const userKassAta = new Address("So11111111111111111111111111111111111111112");
    const { instructions } = await buyInstructions({
      refs,
      user: USER,
      outcome: "no",
      kassAmount: 1000n,
      userKassAta,
    });
    expect(instructions[1].data[8]).toBe(SwapType.Sell); // base→quote = cYES→cNO
  });

  it("sell YES = swap(cYES→cNO, Sell) + merge", async () => {
    const refs = await fakeRefs();
    const userKassAta = new Address("So11111111111111111111111111111111111111112");
    const { instructions } = await sellInstructions({
      refs,
      user: USER,
      outcome: "yes",
      swapAmount: 500n,
      mergeAmount: 400n,
      userKassAta,
    });
    expect(instructions).toHaveLength(2);
    expect(instructions[0].data[8]).toBe(SwapType.Sell);
    expect(Array.from(instructions[1].data.slice(0, 8))).toEqual(Array.from(DISC.mergeTokens));
  });
});

describe("flows/redeem", () => {
  it("emits a single redeem_tokens ix and derives the user conditional ATAs", async () => {
    const refs = await fakeRefs();
    const userKassAta = new Address("So11111111111111111111111111111111111111112");
    const { instructions, userYesAta, userNoAta } = await redeemInstructions({
      refs,
      user: USER,
      userKassAta,
    });
    expect(instructions).toHaveLength(1);
    expect(Array.from(instructions[0].data.slice(0, 8))).toEqual(Array.from(DISC.redeemTokens));
    expect(userYesAta).toBeDefined();
    expect(userNoAta).toBeDefined();
  });
});

describe("flows/atas", () => {
  it("emits idempotent ATA-create ixs (disc 1) for cYES/cNO, and KASS when asked", async () => {
    const refs = await fakeRefs();
    const both = await ensureConditionalAtasInstructions({ refs, user: USER });
    expect(both.instructions).toHaveLength(2);
    for (const ix of both.instructions) {
      expect(ix.programId.toString()).toBe(ATA_PROGRAM_ID.toString());
      expect(Array.from(ix.data)).toEqual([1]); // createAssociatedTokenAccountIdempotent
    }
    expect(both.userKassAta).toBeUndefined();

    const withKass = await ensureConditionalAtasInstructions({ refs, user: USER, includeKass: true });
    expect(withKass.instructions).toHaveLength(3);
    expect(withKass.userKassAta).toBeDefined();
    // The derived cYES ATA matches what redeem/buy derive by default.
    const redeemed = await redeemInstructions({
      refs,
      user: USER,
      userKassAta: new Address("So11111111111111111111111111111111111111112"),
    });
    expect(redeemed.userYesAta.toString()).toBe(withKass.userYesAta.toString());
    expect(redeemed.userNoAta.toString()).toBe(withKass.userNoAta.toString());
  });
});

describe("flows/jupiter", () => {
  it("shapes a v6 quote+swap request (no network) with output = KASS", () => {
    const req = buildJupiterEntryRequest({
      inputMint: USDC,
      outputMint: KASS,
      amount: 1_000_000n,
      slippageBps: 50,
      userPublicKey: USER,
    });
    expect(req.baseUrl).toBe(JUPITER_V6_BASE_URL);
    expect(req.quote.inputMint).toBe(USDC.toString());
    expect(req.quote.outputMint).toBe(KASS.toString());
    expect(req.quote.amount).toBe("1000000");
    expect(req.quote.slippageBps).toBe(50);
    expect(req.quote.swapMode).toBe("ExactIn");
    expect(req.swap.userPublicKey).toBe(USER.toString());
    // The app fills quoteResponse before POSTing.
    expect(req.swap.quoteResponse).toBeUndefined();
  });

  it("composeWithEntry puts the Jupiter swap ix first", () => {
    const jup = new TransactionInstruction({
      programId: EXTERNAL_PROGRAM_IDS.ammV04,
      keys: [],
      data: new Uint8Array([9]),
    });
    const a = new TransactionInstruction({ programId: EXTERNAL_PROGRAM_IDS.ammV04, keys: [], data: new Uint8Array([1]) });
    const b = new TransactionInstruction({ programId: EXTERNAL_PROGRAM_IDS.ammV04, keys: [], data: new Uint8Array([2]) });
    const out = composeWithEntry(jup, [a, b]);
    expect(out).toEqual([jup, a, b]);
  });
});

describe("flows/createAll", () => {
  const CREATOR = new Address("11111111111111111111111111111112");
  const CREATOR_ATA = new Address("So11111111111111111111111111111111111111112");

  it("emits one step per outcome with distinct PDAs matching pda.market(oracle, i)", async () => {
    const { steps } = await createAllOutcomeMarkets({
      oracle: ORACLE,
      optionsCount: 4,
      creator: CREATOR,
      kassMint: KASS,
      creatorKassAta: CREATOR_ATA,
      seedAmount: 1_000n,
    });

    expect(steps).toHaveLength(4);
    // outcomeIndex is 0..3 in order.
    expect(steps.map((s) => s.outcomeIndex)).toEqual([0, 1, 2, 3]);

    // Each step's market equals the canonically-derived (oracle, i) PDA.
    for (const step of steps) {
      const expected = (await pda.market(ORACLE, step.outcomeIndex)).address;
      expect(step.market.toString()).toBe(expected.toString());
    }

    // The four market addresses are all distinct.
    const uniq = new Set(steps.map((s) => s.market.toString()));
    expect(uniq.size).toBe(4);
  });

  it("byte-checks each ix payload: seedAmount@1, outcome_index@9", async () => {
    const seedAmount = 1_000n;
    const { steps } = await createAllOutcomeMarkets({
      oracle: ORACLE,
      optionsCount: 4,
      creator: CREATOR,
      kassMint: KASS,
      creatorKassAta: CREATOR_ATA,
      seedAmount,
    });

    for (const step of steps) {
      const data = step.instruction.data;
      // data = disc(1) ++ seed_amount(u64 LE) ++ outcome_index(u8) = 10 bytes.
      expect(data).toHaveLength(10);
      const dv = new DataView(data.buffer, data.byteOffset, data.length);
      expect(dv.getBigUint64(1, true)).toBe(seedAmount); // seed_amount @ payload offset 0
      expect(data[9]).toBe(step.outcomeIndex); // outcome_index @ payload offset 8
    }
  });

  it("rejects a non-positive optionsCount", async () => {
    await expect(
      createAllOutcomeMarkets({
        oracle: ORACLE,
        optionsCount: 0,
        creator: CREATOR,
        kassMint: KASS,
        creatorKassAta: CREATOR_ATA,
        seedAmount: 1_000n,
      }),
    ).rejects.toThrow();
  });
});

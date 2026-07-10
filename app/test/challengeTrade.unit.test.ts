/**
 * CU2 offline unit tests for the challenge-market TRADE / CRANK action layer.
 *
 * For `buildSwapIxs` / `buildCrankTwapIxs` we assert their ix `data` + `keys`
 * byte-for-byte match the SDK `ammV04.swap` / `ammV04.crankThatTwap` for the
 * SAME derived inputs; that the amm PDA, the conditional-token MINTS
 * (`[b"conditional_token", vault, index]`, 0=pass/1=fail), and the user/vault
 * ATAs derive exactly as the SDK `ammV04.pda` helpers; that the user ATA is
 * idempotently created ONLY when absent; and that validation rejects a bad
 * amount / pool / side. Fully offline (a fake in-memory `Connection`).
 */
import { Address, Keypair, TransactionInstruction, type Connection } from "@solana/web3.js";
import { EXTERNAL_PROGRAM_IDS, ammV04, type Market } from "@kassandra-market/oracles";
import { describe, expect, it } from "vitest";

import { ValidationError } from "../src/data/actions.ts";
import {
  buildCrankTwapIxs,
  buildSwapIxs,
  conditionalTokenMint,
  constantProductOut,
  crankRateLimited,
  minOutFromSlippage,
  poolMints,
  swapEstimate,
} from "../src/data/actions/challengeTrade.ts";
import type { AmmV04 } from "../src/data/ammV04.ts";

const VLTX = EXTERNAL_PROGRAM_IDS.conditionalVault;
const enc = new TextEncoder();

/** A connection whose ATA-existence check is scripted from a present-set. */
function fakeConnection(present: Set<string> = new Set()): Connection {
  return {
    getAccountInfo: async (a: Address) =>
      present.has(a.toString()) ? ({ data: new Uint8Array(165) } as unknown) : null,
  } as unknown as Connection;
}

async function marketFixture(): Promise<Market> {
  const kassVault = (await Keypair.generate()).publicKey;
  const usdcVault = (await Keypair.generate()).publicKey;
  return { kassVault, usdcVault } as unknown as Market;
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

const USER = (await Keypair.generate()).publicKey;

describe("conditional-token mint + amm/ATA derivations", () => {
  it("derives the pass/fail conditional-token mints as [b'conditional_token', vault, index]", async () => {
    const m = await marketFixture();
    for (const [pool, idx] of [
      ["pass", 0],
      ["fail", 1],
    ] as const) {
      const { base, quote } = await poolMints(m, pool);
      const [expBase] = await Address.findProgramAddress(
        [enc.encode("conditional_token"), m.kassVault.toBytes(), Uint8Array.of(idx)],
        VLTX,
      );
      const [expQuote] = await Address.findProgramAddress(
        [enc.encode("conditional_token"), m.usdcVault.toBytes(), Uint8Array.of(idx)],
        VLTX,
      );
      expect(base.toString()).toBe(expBase.toString());
      expect(quote.toString()).toBe(expQuote.toString());
      // conditionalTokenMint() agrees with poolMints().
      expect((await conditionalTokenMint(m.kassVault, idx)).toString()).toBe(expBase.toString());
    }
  });

  it("the swap amm PDA == ammV04.pda.amm(base, quote) for the pool mints", async () => {
    const m = await marketFixture();
    const { base, quote } = await poolMints(m, "fail");
    const conn = fakeConnection();
    const ixs = await buildSwapIxs({
      connection: conn,
      market: m,
      pool: "fail",
      side: "buy",
      amountIn: 1000n,
      user: USER,
    });
    // Last ix is the swap; its amm account (key[1]) is the derived PDA.
    const swapIx = ixs[ixs.length - 1];
    const expAmm = (await ammV04.pda.amm(base, quote)).address;
    expect(swapIx.keys[1].pubkey.toString()).toBe(expAmm.toString());
    // vault base/quote (keys 4,5) are the amm's ATAs.
    expect(swapIx.keys[4].pubkey.toString()).toBe((await ammV04.pda.ata(expAmm, base)).toString());
    expect(swapIx.keys[5].pubkey.toString()).toBe((await ammV04.pda.ata(expAmm, quote)).toString());
  });
});

describe("buildSwapIxs", () => {
  it("byte-matches ammV04.swap (Buy) for the derived mints + creates BOTH user ATAs when absent", async () => {
    const m = await marketFixture();
    const { base, quote } = await poolMints(m, "fail");
    const conn = fakeConnection(); // nothing present → both ATAs created
    const ixs = await buildSwapIxs({
      connection: conn,
      market: m,
      pool: "fail",
      side: "buy",
      amountIn: 5000n,
      minAmountOut: 42n,
      user: USER,
    });
    expect(ixs.length).toBe(3); // create base ATA, create quote ATA, swap
    expectIxMatches(
      ixs[2],
      await ammV04.swap({
        payer: USER,
        baseMint: base,
        quoteMint: quote,
        swapType: ammV04.SwapType.Buy,
        inputAmount: 5000n,
        minOutputAmount: 42n,
      }),
    );
    // The two pre-ixs are idempotent create-ATA (ATA program, disc byte 1).
    const userBase = await ammV04.pda.ata(USER, base);
    const userQuote = await ammV04.pda.ata(USER, quote);
    expect(ixs[0].keys[1].pubkey.toString()).toBe(userBase.toString());
    expect(ixs[1].keys[1].pubkey.toString()).toBe(userQuote.toString());
    expect(Array.from(ixs[0].data)).toEqual([1]);
  });

  it("byte-matches ammV04.swap (Sell) and skips a create-ATA that already exists", async () => {
    const m = await marketFixture();
    const { base, quote } = await poolMints(m, "pass");
    const userBase = await ammV04.pda.ata(USER, base);
    const conn = fakeConnection(new Set([userBase.toString()])); // base present, quote absent
    const ixs = await buildSwapIxs({
      connection: conn,
      market: m,
      pool: "pass",
      side: "sell",
      amountIn: 777n,
      minAmountOut: 0n,
      user: USER,
    });
    expect(ixs.length).toBe(2); // create quote ATA + swap only
    expectIxMatches(
      ixs[1],
      await ammV04.swap({
        payer: USER,
        baseMint: base,
        quoteMint: quote,
        swapType: ammV04.SwapType.Sell,
        inputAmount: 777n,
        minOutputAmount: 0n,
      }),
    );
  });

  it("emits no create-ATA when both accounts exist", async () => {
    const m = await marketFixture();
    const { base, quote } = await poolMints(m, "fail");
    const conn = fakeConnection(
      new Set([
        (await ammV04.pda.ata(USER, base)).toString(),
        (await ammV04.pda.ata(USER, quote)).toString(),
      ]),
    );
    const ixs = await buildSwapIxs({
      connection: conn,
      market: m,
      pool: "fail",
      side: "buy",
      amountIn: 1n,
      user: USER,
    });
    expect(ixs.length).toBe(1);
  });

  it("computes minAmountOut from the reserves + slippage when no explicit floor is given", async () => {
    const m = await marketFixture();
    const amm: AmmV04 = {
      baseMint: new Address("11111111111111111111111111111111"),
      quoteMint: new Address("11111111111111111111111111111111"),
      baseDecimals: 9,
      quoteDecimals: 6,
      baseAmount: 1_000_000n,
      quoteAmount: 1_000_000n,
      createdAtSlot: 0n,
      lastUpdatedSlot: 0n,
      startDelaySlots: 0n,
      aggregator: 0n,
    };
    const conn = fakeConnection(
      new Set([
        (await ammV04.pda.ata(USER, (await poolMints(m, "fail")).base)).toString(),
        (await ammV04.pda.ata(USER, (await poolMints(m, "fail")).quote)).toString(),
      ]),
    );
    const ixs = await buildSwapIxs({
      connection: conn,
      market: m,
      pool: "fail",
      side: "buy",
      amountIn: 100_000n,
      user: USER,
      slippageBps: 100, // 1%
      amm,
    });
    // out = 100000*1000000/(1000000+100000) = 90909; minOut = floor(90909*0.99) = 89999(-ish)
    const est = constantProductOut(100_000n, amm.quoteAmount, amm.baseAmount);
    const expectedMin = minOutFromSlippage(est, 100);
    const data = ixs[0].data; // single swap ix (ATAs present)
    // swap data = disc(8) + u8 type + u64 in + u64 out; read the trailing u64.
    const dv = new DataView(data.buffer, data.byteOffset, data.byteLength);
    const minOut = dv.getBigUint64(data.length - 8, true);
    expect(minOut).toBe(expectedMin);
  });

  it("rejects a zero amount / bad pool / bad side with a ValidationError", async () => {
    const m = await marketFixture();
    const conn = fakeConnection();
    await expect(
      buildSwapIxs({ connection: conn, market: m, pool: "fail", side: "buy", amountIn: 0n, user: USER }),
    ).rejects.toBeInstanceOf(ValidationError);
    await expect(
      // @ts-expect-error bad pool
      buildSwapIxs({ connection: conn, market: m, pool: "middle", side: "buy", amountIn: 1n, user: USER }),
    ).rejects.toBeInstanceOf(ValidationError);
    await expect(
      // @ts-expect-error bad side
      buildSwapIxs({ connection: conn, market: m, pool: "fail", side: "hold", amountIn: 1n, user: USER }),
    ).rejects.toBeInstanceOf(ValidationError);
  });
});

describe("buildCrankTwapIxs", () => {
  it("byte-matches ammV04.crankThatTwap for the derived amm PDA (pass + fail)", async () => {
    const m = await marketFixture();
    for (const pool of ["pass", "fail"] as const) {
      const { base, quote } = await poolMints(m, pool);
      const amm = (await ammV04.pda.amm(base, quote)).address;
      const ixs = await buildCrankTwapIxs({ market: m, pool });
      expect(ixs.length).toBe(1);
      expectIxMatches(ixs[0], await ammV04.crankThatTwap({ amm }));
    }
  });

  it("rejects a bad pool with a ValidationError", async () => {
    const m = await marketFixture();
    // @ts-expect-error bad pool
    await expect(buildCrankTwapIxs({ market: m, pool: "nope" })).rejects.toBeInstanceOf(
      ValidationError,
    );
  });
});

describe("pure preview helpers", () => {
  it("constantProductOut is the fee-less xy=k estimate", () => {
    expect(constantProductOut(100n, 1000n, 1000n)).toBe((100n * 1000n) / 1100n);
    expect(constantProductOut(0n, 1000n, 1000n)).toBe(0n);
    expect(constantProductOut(100n, 0n, 1000n)).toBe(0n);
  });

  it("swapEstimate routes buy=quote→base, sell=base→quote and clamps impact", () => {
    const amm = { baseAmount: 2_000n, quoteAmount: 1_000n } as unknown as AmmV04;
    const buy = swapEstimate(amm, "buy", 100n);
    const sell = swapEstimate(amm, "sell", 100n);
    expect(buy.expectedOut).toBe(constantProductOut(100n, 1_000n, 2_000n));
    expect(sell.expectedOut).toBe(constantProductOut(100n, 2_000n, 1_000n));
    expect(buy.impact).toBeGreaterThanOrEqual(0);
    expect(buy.impact).toBeLessThanOrEqual(1);
    expect(swapEstimate(null, "buy", 100n).expectedOut).toBe(0n);
  });

  it("minOutFromSlippage floors by bps and clamps", () => {
    expect(minOutFromSlippage(1000n, 100)).toBe(990n); // 1%
    expect(minOutFromSlippage(1000n, 0)).toBe(1000n);
    expect(minOutFromSlippage(0n, 100)).toBe(0n);
    expect(minOutFromSlippage(1000n, 20000)).toBe(0n); // clamped to 100%
  });

  it("crankRateLimited flags a crank within 150 slots (else false / unknown)", () => {
    const amm = { lastUpdatedSlot: 1000n } as unknown as AmmV04;
    expect(crankRateLimited(amm, 1100n)).toBe(true); // 100 < 150
    expect(crankRateLimited(amm, 1200n)).toBe(false); // 200 >= 150
    expect(crankRateLimited(amm, null)).toBe(false);
    expect(crankRateLimited(null, 9999n)).toBe(false);
  });
});

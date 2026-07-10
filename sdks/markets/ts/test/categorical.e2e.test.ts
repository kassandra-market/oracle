/**
 * litesvm CATEGORICAL round-trip, driven THROUGH THE SDK — the proof that a
 * categorical Kassandra oracle (`options_count > 2`) is modeled as N independent
 * per-outcome binary sub-markets, each keyed by `(oracle, outcome_index)`.
 *
 * Against the REAL compiled `kassandra_markets_program.so`:
 *   1. initConfig + a fabricated 3-option oracle (Proposal).
 *   2. createMarket at outcome_index 0, 1, 2 → three DISTINCT market PDAs, all
 *      succeed; decode each Market and assert `outcomeIndex` matches.
 *   3. outcome_index == options_count (3) is rejected with InvalidOutcome (19).
 *
 * This exercises the new `pda.market(oracle, outcomeIndex)` seed + the
 * `createMarket` outcome_index payload byte + `decodeMarket.outcomeIndex` end to
 * end (not just self-consistently).
 */
import { describe, it } from "vitest";

import { AccountType, MarketError, MarketStatus } from "../src/constants.js";
import { Phase } from "../src/accounts/oracle.js";
import { createMarket, initConfig } from "../src/instructions/index.js";
import * as pda from "../src/pda.js";

import { MarketTestCtx, customCode, expect } from "./harness.js";

describe("litesvm categorical markets (per-outcome binary sub-markets)", () => {
  it("creates sub-markets at outcome_index 0/1/2 on a 3-option oracle → distinct PDAs; decodes outcomeIndex; rejects out-of-range", async () => {
    const ctx = await MarketTestCtx.new();

    // ── initConfig ──────────────────────────────────────────────────────────
    const kassMint = await ctx.createMint(9);
    const authority = (await ctx.fundedKeypair()).publicKey;
    const minLiquidity = 1_000_000n;
    const feeDestination = await ctx.createTokenAccount(kassMint, authority, 0n);
    await ctx.sendOk(
      await initConfig({
        payer: ctx.payer.publicKey,
        kassMint,
        authority,
        minLiquidity,
        feeBps: 100,
        feeDestination,
      }),
      [],
      "initConfig",
    );

    // ── A single 3-option (categorical) oracle in the Proposal phase. ────────
    const OPTIONS = 3;
    const oracle = await ctx.seedOracle(OPTIONS, Phase.Proposal);
    const seed = 300_000n;

    const marketAddrs: string[] = [];
    for (let outcomeIndex = 0; outcomeIndex < OPTIONS; outcomeIndex++) {
      const creator = await ctx.fundedKeypair();
      const creatorKassAta = await ctx.createTokenAccount(kassMint, creator.publicKey, seed);
      await ctx.sendOk(
        await createMarket({
          creator: creator.publicKey,
          oracle,
          kassMint,
          creatorKassAta,
          seedAmount: seed,
          outcomeIndex,
        }),
        [creator],
        `createMarket(outcome ${outcomeIndex})`,
      );

      const marketPda = (await pda.market(oracle, outcomeIndex)).address;
      const market = ctx.readMarket(marketPda);
      expect(market.accountType).toBe(AccountType.Market);
      expect(market.status).toBe(MarketStatus.Funding);
      expect(market.oracle.toString()).toBe(oracle.toString());
      // The decoded outcome_index (u8 @397) round-trips the one we created with.
      expect(market.outcomeIndex).toBe(outcomeIndex);
      marketAddrs.push(marketPda.toString());
    }

    // All three sub-markets are DISTINCT addresses (PDA keyed by outcome).
    expect(new Set(marketAddrs).size).toBe(OPTIONS);

    // ── outcome_index == options_count (out of range) → InvalidOutcome (19). ──
    const badCreator = await ctx.fundedKeypair();
    const badAta = await ctx.createTokenAccount(kassMint, badCreator.publicKey, seed);
    const badResult = await ctx.send(
      await createMarket({
        creator: badCreator.publicKey,
        oracle,
        kassMint,
        creatorKassAta: badAta,
        seedAmount: seed,
        outcomeIndex: OPTIONS, // == options_count → out of range
      }),
      [badCreator],
    );
    expect(customCode(badResult)).toBe(MarketError.InvalidOutcome);
  });
});

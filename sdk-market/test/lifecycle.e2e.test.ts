/**
 * litesvm END-TO-END round-trip, driven THROUGH THE SDK — the proof the
 * hand-written builders + decoders are wire-correct against the REAL compiled
 * `target/deploy/kassandra_market_program.so` (not just self-consistent).
 *
 * Every instruction below is built by an SDK builder (`src/instructions/*`),
 * bridged into litesvm via `toLiteSvmTransaction`, submitted to the real program,
 * and every resulting account is decoded by an SDK decoder (`src/accounts/*`).
 *
 * This is the Phase-1 path (no MetaDAO needed) and exercises 5 of the 9
 * instructions end-to-end: initConfig, createMarket, contribute, cancel, refund.
 * It mirrors the Rust references `tests/{init_config,create_market,contribute,
 * cancel,refund}.rs`.
 *
 *   1. initConfig            → decode Config, assert authority/kassMint/minLiquidity.
 *   2. seedOracle(2, Proposal) + createMarket(seed)
 *                            → decode Market, assert Funding/totals/oracle/escrow;
 *                              assert escrow token balance == seed.
 *   3. contribute (2nd funded contributor)
 *                            → decode Market totalContributed summed;
 *                              decode Contribution amount.
 *   4. re-seed oracle terminal (Resolved) + cancel → Cancelled;
 *      refund each contributor → KASS ATAs restored, escrow drains to 0,
 *      Contribution.claimed == true.
 */
import { describe, it } from "vitest";

import { AccountType, MarketStatus } from "../src/constants.js";
import { Phase } from "../src/accounts/oracle.js";
import { cancel, contribute, createMarket, initConfig, refund } from "../src/instructions/index.js";
import * as pda from "../src/pda.js";

import { MarketTestCtx, expect } from "./harness.js";

describe("litesvm lifecycle round-trip (Phase-1, no MetaDAO)", () => {
  it("initConfig → createMarket → contribute → cancel → refund, all via SDK", async () => {
    const ctx = await MarketTestCtx.new();

    // --- 1. initConfig -------------------------------------------------------
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

    const configPda = (await pda.config()).address;
    const config = ctx.readConfig(configPda);
    expect(config.accountType).toBe(AccountType.Config);
    expect(config.authority.toString()).toBe(authority.toString());
    expect(config.kassMint.toString()).toBe(kassMint.toString());
    expect(config.minLiquidity).toBe(minLiquidity);

    // --- 2. createMarket -----------------------------------------------------
    const oracle = await ctx.seedOracle(2, Phase.Proposal);
    const seed = 300_000n;

    const creator = await ctx.fundedKeypair();
    const creatorKassAta = await ctx.createTokenAccount(kassMint, creator.publicKey, seed);

    await ctx.sendOk(
      await createMarket({
        creator: creator.publicKey,
        oracle,
        kassMint,
        creatorKassAta,
        seedAmount: seed,
        outcomeIndex: 0,
      }),
      [creator],
      "createMarket",
    );

    const marketPda = (await pda.market(oracle, 0)).address;
    const escrowPda = (await pda.escrow(marketPda)).address;
    let market = ctx.readMarket(marketPda);
    expect(market.accountType).toBe(AccountType.Market);
    expect(market.status).toBe(MarketStatus.Funding);
    expect(market.oracle.toString()).toBe(oracle.toString());
    expect(market.creator.toString()).toBe(creator.publicKey.toString());
    expect(market.kassMint.toString()).toBe(kassMint.toString());
    expect(market.escrowVault.toString()).toBe(escrowPda.toString());
    expect(market.totalContributed).toBe(seed);
    expect(market.minLiquidity).toBe(minLiquidity);
    // Escrow holds exactly the seed; the creator's ATA drained to 0.
    expect(ctx.tokenBalance(escrowPda)).toBe(seed);
    expect(ctx.tokenBalance(creatorKassAta)).toBe(0n);

    // Creator's Contribution recorded the seed.
    const creatorContribPda = (await pda.contribution(marketPda, creator.publicKey)).address;
    const creatorContrib = ctx.readContribution(creatorContribPda);
    expect(creatorContrib.accountType).toBe(AccountType.Contribution);
    expect(creatorContrib.market.toString()).toBe(marketPda.toString());
    expect(creatorContrib.contributor.toString()).toBe(creator.publicKey.toString());
    expect(creatorContrib.amount).toBe(seed);
    expect(creatorContrib.claimed).toBe(false);

    // --- 3. contribute (second contributor) ----------------------------------
    const amount = 200_000n;
    const contributor = await ctx.fundedKeypair();
    const contributorKassAta = await ctx.createTokenAccount(kassMint, contributor.publicKey, amount);

    await ctx.sendOk(
      await contribute({
        contributor: contributor.publicKey,
        market: marketPda,
        contributorKassAta,
        amount,
      }),
      [contributor],
      "contribute",
    );

    market = ctx.readMarket(marketPda);
    expect(market.totalContributed).toBe(seed + amount);
    expect(ctx.tokenBalance(escrowPda)).toBe(seed + amount);
    expect(ctx.tokenBalance(contributorKassAta)).toBe(0n);

    const contributorContribPda = (await pda.contribution(marketPda, contributor.publicKey)).address;
    const contributorContrib = ctx.readContribution(contributorContribPda);
    expect(contributorContrib.amount).toBe(amount);
    expect(contributorContrib.contributor.toString()).toBe(contributor.publicKey.toString());
    expect(contributorContrib.claimed).toBe(false);

    // --- 4. cancel + refund --------------------------------------------------
    // Move the SAME oracle to the terminal Resolved phase, then cancel.
    await ctx.seedOracle(2, Phase.Resolved, 0, oracle);
    await ctx.sendOk(await cancel({ market: marketPda, oracle }), [], "cancel");

    market = ctx.readMarket(marketPda);
    expect(market.status).toBe(MarketStatus.Cancelled);

    // Refund the creator (permissionless; ATA owner must match the contributor).
    // The Contribution is CLOSED, its rent returned to the creator (the contributor),
    // and open_contributions decrements — its absence is the idempotency guard now.
    const creatorRentBefore = ctx.lamportsOf(creator.publicKey);
    await ctx.sendOk(
      await refund({ market: marketPda, contributor: creator.publicKey, contributorKassAta: creatorKassAta }),
      [],
      "refund(creator)",
    );
    expect(ctx.tokenBalance(creatorKassAta)).toBe(seed);
    expect(ctx.exists(creatorContribPda)).toBe(false); // Contribution closed
    expect(ctx.lamportsOf(creator.publicKey)).toBeGreaterThan(creatorRentBefore); // rent returned
    expect(ctx.tokenBalance(escrowPda)).toBe(amount);
    expect(ctx.readMarket(marketPda).openContributions).toBe(1);

    // Refund the second contributor; escrow drains to 0, its Contribution closes.
    const contributorRentBefore = ctx.lamportsOf(contributor.publicKey);
    await ctx.sendOk(
      await refund({
        market: marketPda,
        contributor: contributor.publicKey,
        contributorKassAta: contributorKassAta,
      }),
      [],
      "refund(contributor)",
    );
    expect(ctx.tokenBalance(contributorKassAta)).toBe(amount);
    expect(ctx.exists(contributorContribPda)).toBe(false); // Contribution closed
    expect(ctx.lamportsOf(contributor.publicKey)).toBeGreaterThan(contributorRentBefore); // rent returned
    expect(ctx.tokenBalance(escrowPda)).toBe(0n);
    expect(ctx.readMarket(marketPda).openContributions).toBe(0);

    // --- 5. closeMarket (Cancelled path: closes escrow + Market only) --------
    // Cancelled markets never activated (market.lp_vault == default), so close_market
    // reaps just the escrow + the Market PDA, routing both rents to the creator.
    const creatorBeforeClose = ctx.lamportsOf(creator.publicKey);
    await ctx.closeMarket(marketPda, creator.publicKey, "closeMarket");
    expect(ctx.exists(marketPda)).toBe(false); // Market PDA gone
    expect(ctx.exists(escrowPda)).toBe(false); // escrow token account gone
    expect(ctx.lamportsOf(creator.publicKey)).toBeGreaterThan(creatorBeforeClose); // rent returned
  });
});

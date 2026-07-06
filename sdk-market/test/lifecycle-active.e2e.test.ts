/**
 * Full ACTIVE-market lifecycle, driven THROUGH THE SDK flows — the SDK-level
 * proof that the whole `compose → activate → trade → resolve → collect_fee →
 * claim_lp → redeem` surface is wire-correct against the REAL compiled program +
 * the deployed MetaDAO v0.4 `conditional_vault` + `amm` fixtures in LiteSVM.
 *
 * It mirrors the Rust `programs/kassandra-market/tests/lifecycle_active.rs` +
 * `tests/collect_fee.rs`:
 *   1. initConfig(min_liquidity = 1 KASS, fee_bps = 100)
 *   2. createMarket(seed = SEED_A) + contribute(SEED_B) → funded to exactly MIN_LIQ
 *   3. composeMarketInstructions (question + vault + amm) → activateInstruction
 *      → status Active, escrow drained, pool seeded, lpTotal > 0
 *   4. a REAL swap grows the pool (a swapper splits KASS, sells cYES) so the LP
 *      position accrues genuine earnings; a fresh user `split`s a redeemable
 *      cYES+cNO position (the winner)
 *   5. oracle resolves option 0 (YES) → resolveMarket → status Resolved, settled,
 *      fee NOT yet collected
 *   6. claim_lp BEFORE collect_fee is rejected (FeeNotCollected)
 *   7. collect_fee → fee_destination receives the accrued KASS cut, lpTotal is
 *      reduced, fee_collected == 1
 *   8. claim_lp for BOTH contributors pays pro-rata off the REDUCED lpTotal
 *   9. redeem → the YES holder is paid the winning leg 1:1 in KASS
 *
 * The fee cut is what forces the resolve → collect_fee → claim_lp ordering: with a
 * non-zero fee_bps, claim_lp is gated on collection.
 */
import { describe, it } from "vitest";

import { MarketError, MarketStatus } from "../src/constants.js";
import { Phase } from "../src/accounts/oracle.js";
import { claimLp, contribute, createMarket, initConfig, resolveMarket } from "../src/instructions/index.js";
import * as metadao from "../src/metadao/index.js";
import * as pda from "../src/pda.js";
import {
  activateInstruction,
  collectFeeInstruction,
  composeMarketInstructions,
  redeemInstructions,
} from "../src/flows/index.js";

import { MarketTestCtx, customCode, expect } from "./harness.js";

const MIN_LIQ = 1_000_000_000n; // 1 KASS (9 dp)
const SEED_A = 600_000_000n; // creator's stake
const SEED_B = 400_000_000n; // second contributor's stake (A + B == MIN_LIQ)
const SPLIT_AMT = 2_000_000_000n; // KASS the traded user splits for a position
const SWAP_KASS = 3_000_000_000n; // KASS the swapper splits to grow the pool
const SWAP_IN = 1_500_000_000n; // cYES sold into the pool to accrue swap fees
const FEE_BPS = 100; // 1% protocol fee

/** Floor pro-rata LP share, mirroring the on-chain u128-intermediate helper. */
function expectedShare(lpTotal: bigint, amount: bigint, total: bigint): bigint {
  return (lpTotal * amount) / total;
}

describe("litesvm full active-market lifecycle (compose → activate → trade → resolve → collect_fee → claimLp → redeem)", () => {
  it("stands up, funds, activates, accrues + collects a protocol fee, then distributes reduced LP and pays a YES winner", async () => {
    const ctx = await MarketTestCtx.new();

    // ── Stage 1: initConfig (fee_bps = 100, a KASS fee destination) ─────────
    const kass = await ctx.createMint(9);
    const authority = (await ctx.fundedKeypair()).publicKey;
    const feeDestination = await ctx.createTokenAccount(kass, authority, 0n);
    await ctx.sendOk(
      await initConfig({
        payer: ctx.payer.publicKey,
        kassMint: kass,
        authority,
        minLiquidity: MIN_LIQ,
        feeBps: FEE_BPS,
        feeDestination,
      }),
      [],
      "initConfig",
    );

    // ── Stage 2: createMarket + contribute (fund to exactly MIN_LIQ) ───────
    const oracle = await ctx.seedOracle(2, Phase.Proposal);
    const market = (await pda.market(oracle, 0)).address;

    const creator = await ctx.fundedKeypair();
    const creatorAta = await ctx.createTokenAccount(kass, creator.publicKey, 5_000_000_000n);
    await ctx.sendOk(
      await createMarket({
        creator: creator.publicKey,
        oracle,
        kassMint: kass,
        creatorKassAta: creatorAta,
        seedAmount: SEED_A,
        outcomeIndex: 0,
      }),
      [creator],
      "createMarket",
    );

    const c2 = await ctx.fundedKeypair();
    const c2Ata = await ctx.createTokenAccount(kass, c2.publicKey, 5_000_000_000n);
    await ctx.sendOk(
      await contribute({ contributor: c2.publicKey, market, contributorKassAta: c2Ata, amount: SEED_B }),
      [c2],
      "contribute",
    );

    let m = ctx.readMarket(market);
    expect(m.totalContributed).toBe(MIN_LIQ);
    expect(m.status).toBe(MarketStatus.Funding);
    expect(m.feeBps).toBe(FEE_BPS);
    const escrow = m.escrowVault;
    expect(ctx.tokenBalance(escrow)).toBe(MIN_LIQ);

    // ── Stage 3: compose MetaDAO market + activate ─────────────────────────
    const { instructions: composeIxs, refs } = await composeMarketInstructions({
      market,
      oracle,
      kassMint: kass,
      payer: ctx.payer.publicKey,
    });
    await ctx.sendManyOk([composeIxs[0]], [], "initializeQuestion");
    await ctx.sendManyOk([composeIxs[1]], [], "initializeConditionalVault");
    await ctx.sendManyOk([composeIxs[2]], [], "createAmm");

    await ctx.sendManyOk([await activateInstruction({ refs, payer: ctx.payer.publicKey })], [], "activate");

    m = ctx.readMarket(market);
    expect(m.status).toBe(MarketStatus.Active);
    expect(ctx.tokenBalance(escrow)).toBe(0n);
    expect(m.lpTotal).toBeGreaterThan(0n);
    expect(ctx.tokenBalance(refs.lpVault)).toBe(m.lpTotal);
    expect(m.question.toString()).toBe(refs.question.toString());
    const lpTotalAtActivation = m.lpTotal;

    // ── Stage 4a: a REAL swap grows the pool → the LP position accrues fees ─
    // A fresh swapper splits SWAP_KASS into cYES+cNO, then SELLS SWAP_IN cYES into
    // the pool (base→quote = Sell). The swap fee accrues to the reserves, so a YES
    // resolution realizes genuine LP earnings.
    const swapper = await ctx.fundedKeypair();
    const swKass = await ctx.createTokenAccount(kass, swapper.publicKey, SWAP_KASS);
    const swCyes = await ctx.createTokenAccount(refs.yesMint, swapper.publicKey, 0n);
    const swCno = await ctx.createTokenAccount(refs.noMint, swapper.publicKey, 0n);
    await ctx.sendManyOk(
      [
        await metadao.splitTokens({
          question: refs.question,
          vault: refs.vault,
          vaultUnderlyingAta: refs.vaultUnderlyingAta,
          authority: swapper.publicKey,
          userUnderlyingAta: swKass,
          conditionalMints: [refs.yesMint, refs.noMint],
          userConditionalAtas: [swCyes, swCno],
          amount: SWAP_KASS,
        }),
      ],
      [swapper],
      "swapper split",
    );
    await ctx.sendManyOk(
      [
        await metadao.swap({
          payer: swapper.publicKey,
          baseMint: refs.yesMint,
          quoteMint: refs.noMint,
          swapType: metadao.SwapType.Sell,
          inputAmount: SWAP_IN,
          outputAmountMin: 0n,
          userBase: swCyes,
          userQuote: swCno,
        }),
      ],
      [swapper],
      "swap (sell cYES)",
    );

    // ── Stage 4b: hand a fresh user a net redeemable position (the winner) ──
    const winner = await ctx.fundedKeypair();
    const winKass = await ctx.createTokenAccount(kass, winner.publicKey, SPLIT_AMT);
    const winCyes = await ctx.createTokenAccount(refs.yesMint, winner.publicKey, 0n);
    const winCno = await ctx.createTokenAccount(refs.noMint, winner.publicKey, 0n);
    await ctx.sendManyOk(
      [
        await metadao.splitTokens({
          question: refs.question,
          vault: refs.vault,
          vaultUnderlyingAta: refs.vaultUnderlyingAta,
          authority: winner.publicKey,
          userUnderlyingAta: winKass,
          conditionalMints: [refs.yesMint, refs.noMint],
          userConditionalAtas: [winCyes, winCno],
          amount: SPLIT_AMT,
        }),
      ],
      [winner],
      "winner split",
    );
    expect(ctx.tokenBalance(winCyes)).toBe(SPLIT_AMT);
    expect(ctx.tokenBalance(winCno)).toBe(SPLIT_AMT);

    // ── Stage 5: oracle resolves YES (option 0) + resolveMarket ────────────
    await ctx.seedOracle(2, Phase.Resolved, 0, oracle);
    await ctx.sendManyOk(
      [
        await resolveMarket({
          market,
          oracle,
          question: refs.question,
          cvEventAuthority: refs.cvEventAuthority,
        }),
      ],
      [],
      "resolveMarket",
    );
    m = ctx.readMarket(market);
    expect(m.settled).toBe(true);
    expect(m.status).toBe(MarketStatus.Resolved);
    expect(m.feeCollected).toBe(false); // fee_bps > 0 → crank still pending

    // ── Stage 6: claim_lp BEFORE collect_fee is rejected (FeeNotCollected) ─
    const earlyAta = await ctx.createTokenAccount(refs.lpMint, creator.publicKey, 0n);
    const early = await ctx.send(
      await claimLp({ market, contributor: creator.publicKey, contributorLpAta: earlyAta }),
    );
    expect(customCode(early)).toBe(MarketError.FeeNotCollected);
    expect(ctx.tokenBalance(earlyAta)).toBe(0n); // nothing moved

    // ── Stage 7: collect_fee → the futarchy receives the accrued KASS cut ──
    const lpTotalBeforeCollect = ctx.readMarket(market).lpTotal;
    await ctx.sendManyOk(
      [await collectFeeInstruction({ refs, feeDestination })],
      [],
      "collectFee",
    );
    m = ctx.readMarket(market);
    expect(m.feeCollected).toBe(true);
    const feeKass = ctx.tokenBalance(feeDestination);
    expect(feeKass).toBeGreaterThan(0n); // the futarchy received a real cut
    expect(m.lpTotal).toBeLessThan(lpTotalBeforeCollect); // lp_total reduced by the fee slice
    expect(m.lpTotal).toBeLessThan(lpTotalAtActivation);
    const reducedTotal = m.lpTotal;

    // A second collect_fee is idempotent-rejected.
    const second = await ctx.sendMany([await collectFeeInstruction({ refs, feeDestination })]);
    expect(customCode(second)).toBe(MarketError.AlreadySettled);

    // ── Stage 8: claimLp for BOTH contributors, off the REDUCED lpTotal ────
    // The FIRST claimer (open_contributions == 2) takes the floor pro-rata share;
    // the LAST claimer (open_contributions == 1) sweeps the ENTIRE remaining
    // lp_vault so it ends at exactly 0. Each claim CLOSES its Contribution, with the
    // rent returned to the contributor (the account is gone afterward).
    const aShare = expectedShare(reducedTotal, SEED_A, MIN_LIQ);
    expect(aShare).toBeGreaterThan(0n);

    const creatorContribPda = (await pda.contribution(market, creator.publicKey)).address;
    const c2ContribPda = (await pda.contribution(market, c2.publicKey)).address;
    const creatorRentBefore = ctx.lamportsOf(creator.publicKey);

    // Creator claims first (not the last claimer) → floor pro-rata aShare.
    await ctx.sendOk(
      await claimLp({ market, contributor: creator.publicKey, contributorLpAta: earlyAta }),
      [],
      "claimLp(creator)",
    );
    expect(ctx.tokenBalance(earlyAta)).toBe(aShare);
    // Contribution CLOSED, its rent returned to the creator (the contributor).
    expect(ctx.exists(creatorContribPda)).toBe(false);
    expect(ctx.lamportsOf(creator.publicKey)).toBeGreaterThan(creatorRentBefore);
    expect(ctx.readMarket(market).openContributions).toBe(1);

    // c2 claims last (open_contributions == 1) → sweeps the entire remaining vault.
    const c2RentBefore = ctx.lamportsOf(c2.publicKey);
    const bLpAta = await ctx.createTokenAccount(refs.lpMint, c2.publicKey, 0n);
    const remainingLp = ctx.tokenBalance(refs.lpVault);
    await ctx.sendOk(
      await claimLp({ market, contributor: c2.publicKey, contributorLpAta: bLpAta }),
      [],
      "claimLp(c2)",
    );
    // The last claimer sweeps EVERYTHING left → lp_vault ends at exactly 0.
    expect(ctx.tokenBalance(bLpAta)).toBe(remainingLp);
    expect(ctx.tokenBalance(refs.lpVault)).toBe(0n);
    expect(ctx.exists(c2ContribPda)).toBe(false);
    expect(ctx.lamportsOf(c2.publicKey)).toBeGreaterThan(c2RentBefore);
    expect(ctx.readMarket(market).openContributions).toBe(0);

    // ── Stage 9: redeem — the YES holder is paid the winning leg 1:1 ───────
    const { instructions: redeemIxs } = await redeemInstructions({
      refs,
      user: winner.publicKey,
      userKassAta: winKass,
      userYesAta: winCyes,
      userNoAta: winCno,
    });
    await ctx.sendManyOk(redeemIxs, [winner], "redeem");
    // cYES paid SPLIT_AMT (1:1), cNO paid 0 → an exact round trip.
    expect(ctx.tokenBalance(winKass)).toBe(SPLIT_AMT);

    // ── Stage 10: closeMarket — reclaim ALL remaining rent to the creator ───
    // Resolved + fee_collected + open_contributions == 0 → the permissionless
    // crank SPL-CloseAccounts escrow/cyes/cno/lp_vault + closes the Market PDA,
    // routing every reclaimed rent lamport back to the creator. (The MetaDAO
    // question/vault/mints are NOT ours — never closed, so redeem above is unaffected.)
    const creatorBeforeClose = ctx.lamportsOf(creator.publicKey);
    await ctx.closeMarket(market, creator.publicKey, "closeMarket");
    // The Market PDA + all four market-owned token accounts are gone…
    expect(ctx.exists(market)).toBe(false);
    expect(ctx.exists(escrow)).toBe(false);
    expect(ctx.exists(refs.marketCyes)).toBe(false);
    expect(ctx.exists(refs.marketCno)).toBe(false);
    expect(ctx.exists(refs.lpVault)).toBe(false);
    // …and the creator received all of that rent.
    expect(ctx.lamportsOf(creator.publicKey)).toBeGreaterThan(creatorBeforeClose);
  });
});

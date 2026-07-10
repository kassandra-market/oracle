/**
 * Full ACTIVE-market lifecycle against a surfpool MAINNET FORK — the top
 * validation layer above the LiteSVM tests. Every instruction is BUILT by the SDK
 * and sent as a REAL RPC transaction (signed, blockhash, compute-budgeted,
 * confirmed) against the REAL deployed MetaDAO conditional-vault + AMM v0.4
 * programs (lazily fetched from the fork — no local fixtures). Input state
 * (KASS mint, ATAs, the Kassandra oracle) is `surfnet_setAccount`-fabricated; all
 * OUTCOMES flow through the real forked programs.
 *
 * Mirrors `programs/markets/tests/collect_fee.rs` +
 * `sdks/oracles/ts/test/lifecycle-active.e2e.test.ts`:
 *   1. initConfig(min_liquidity = 1 KASS, fee_bps = 100, a fabricated KASS fee dest)
 *   2. seedOracle(Proposal) + createMarket(SEED_A) + contribute(SEED_B) → funded
 *   3. compose (question/vault/amm) → activate → Active, escrow drained, lpTotal>0
 *   4. a REAL swap on the fork grows the pool so the LP position accrues fees;
 *      split → a cYES-only winner + a cNO-only loser
 *   5. oracle resolves YES (option 0) → resolveMarket → Resolved, fee NOT collected
 *   6. claim_lp BEFORE collect_fee is rejected (the fee gate)
 *   7. **collect_fee** → the futarchy KASS fee_destination receives the accrued
 *      cut, market.feeCollected == 1, lpTotal reduced — proven against the REAL
 *      MetaDAO programs on the fork
 *   8. claim_lp for BOTH contributors pays pro-rata off the REDUCED lpTotal
 *   9. redeem → the cYES winner is paid 1:1 in KASS; the cNO loser gets 0
 * Conservation: escrow drained, total KASS out ≤ total KASS in.
 */
import { Keypair } from "@solana/web3.js";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { decodeConfig, decodeMarket } from "../../src/accounts/index.js";
import { Phase } from "../../src/accounts/oracle.js";
import { MarketStatus } from "../../src/constants.js";
import {
  activateInstruction,
  collectFeeInstruction,
  composeMarketInstructions,
  redeemInstructions,
} from "../../src/flows/index.js";
import {
  claimLp,
  closeMarket,
  contribute,
  createMarket,
  initConfig,
  resolveMarket,
} from "../../src/instructions/index.js";
import * as metadao from "../../src/metadao/index.js";
import * as pda from "../../src/pda.js";

import { MarketSurfpoolHarness, splTransfer, surfpoolReady } from "./harness/index.js";

const ENABLED = process.env.KASSANDRA_MARKET_E2E === "1" && surfpoolReady();
const PORT = 18903;

const MIN_LIQ = 1_000_000_000n; // 1 KASS (9 dp)
const SEED_A = 600_000_000n; // creator's stake
const SEED_B = 400_000_000n; // second contributor's stake (A + B == MIN_LIQ)
const SPLIT_AMT = 2_000_000_000n; // KASS each traded user splits for a position
const SWAP_KASS = 3_000_000_000n; // KASS the swapper splits to grow the pool
const SWAP_IN = 1_500_000_000n; // cYES sold into the pool to accrue swap fees
const FEE_BPS = 100; // 1% protocol fee

// Compute budgets (the MetaDAO composition + activate/swap/collect CPIs exceed the 200k default).
const CU_COMPOSE = 400_000;
const CU_ACTIVATE = 1_400_000;
const CU_INTERACT = 400_000;
const CU_COLLECT = 800_000;

/** `Question` field byte offsets (after the 8-byte Anchor disc). */
const Q_NUM0_OFFSET = 76;
const Q_NUM1_OFFSET = 80;
const Q_DENOMINATOR_OFFSET = 84;

/** Floor pro-rata LP share, mirroring the on-chain u128-intermediate helper. */
function expectedShare(lpTotal: bigint, amount: bigint, total: bigint): bigint {
  return (lpTotal * amount) / total;
}

/** Read a little-endian u32 from `data` at `off`. */
function u32(data: Uint8Array, off: number): number {
  return new DataView(data.buffer, data.byteOffset, data.length).getUint32(off, true);
}

describe.skipIf(!ENABLED)(
  "surfpool lifecycle: compose → activate → swap → resolve → collect_fee → claimLp → redeem",
  () => {
    let h: MarketSurfpoolHarness;

    beforeAll(async () => {
      h = await MarketSurfpoolHarness.start({ port: PORT, fork: "mainnet" });
    }, 90_000);

    afterAll(async () => {
      await h?.teardown();
    });

    it("stands up, funds, activates, accrues + collects a protocol fee to the futarchy, then distributes reduced LP and pays a YES winner", async () => {
      // Funded keypairs.
      const payer = await Keypair.generate();
      const creator = await Keypair.generate();
      const c2 = await Keypair.generate();
      const swapper = await Keypair.generate();
      const winner = await Keypair.generate();
      const loser = await Keypair.generate();
      for (const kp of [payer, creator, c2, swapper, winner, loser]) {
        await h.airdrop(kp.publicKey.toString());
      }

      // ── Stage 1: initConfig (fee_bps = 100, a fabricated KASS fee dest) ────
      // The payer must be the program's upgrade authority for init_config.
      await h.setUpgradeAuthority(payer.publicKey);
      const kass = await h.createMint(9, payer.publicKey);
      const authority = (await Keypair.generate()).publicKey;
      const feeDestination = await h.createTokenAccount(kass, authority, 0n);
      await h.sendIx(payer, [
        await initConfig({
          payer: payer.publicKey,
          kassMint: kass,
          authority,
          minLiquidity: MIN_LIQ,
          feeBps: FEE_BPS,
          feeDestination,
        }),
      ]);
      const cfg = decodeConfig(await h.waitForAccount((await pda.config()).address));
      expect(cfg.kassMint.toString()).toBe(kass.toString());
      expect(cfg.feeBps).toBe(FEE_BPS);
      expect(cfg.feeDestination.toString()).toBe(feeDestination.toString());

      // ── Stage 2: seedOracle(Proposal) + createMarket + contribute ─────────
      const oracle = await h.seedOracle({ optionsCount: 2, phase: Phase.Proposal });
      const market = (await pda.market(oracle, 0)).address;

      const creatorKass = await h.fundTokenAccount(kass, creator.publicKey, 5_000_000_000n);
      await h.sendIx(creator, [
        await createMarket({
          creator: creator.publicKey,
          oracle,
          kassMint: kass,
          creatorKassAta: creatorKass,
          seedAmount: SEED_A,
          outcomeIndex: 0,
        }),
      ]);
      let m = decodeMarket(await h.waitForAccount(market));
      expect(m.status).toBe(MarketStatus.Funding);
      expect(m.feeBps).toBe(FEE_BPS);
      const escrow = m.escrowVault;

      const c2Kass = await h.fundTokenAccount(kass, c2.publicKey, 5_000_000_000n);
      await h.sendIx(c2, [
        await contribute({ contributor: c2.publicKey, market, contributorKassAta: c2Kass, amount: SEED_B }),
      ]);
      m = decodeMarket(await h.getAccountData(market).then((d) => d!));
      expect(m.totalContributed).toBe(MIN_LIQ);
      expect(await h.tokenBalance(escrow)).toBe(MIN_LIQ);

      // ── Stage 3: compose MetaDAO market + activate (the real CPI path) ─────
      const { instructions: composeIxs, refs } = await composeMarketInstructions({
        market,
        oracle,
        kassMint: kass,
        payer: payer.publicKey,
      });
      await h.sendIx(payer, [composeIxs[0]], [], CU_COMPOSE); // initialize_question
      await h.sendIx(payer, [composeIxs[1]], [], CU_COMPOSE); // initialize_conditional_vault
      await h.sendIx(payer, [composeIxs[2]], [], CU_COMPOSE); // create_amm

      await h.sendIx(payer, [await activateInstruction({ refs, payer: payer.publicKey })], [], CU_ACTIVATE);

      m = decodeMarket(await h.getAccountData(market).then((d) => d!));
      expect(m.status).toBe(MarketStatus.Active);
      expect(await h.tokenBalance(escrow)).toBe(0n);
      expect(m.lpTotal).toBeGreaterThan(0n);
      expect(await h.tokenBalance(refs.lpVault)).toBe(m.lpTotal);
      const lpTotalAtActivation = m.lpTotal;

      // ── Stage 4a: a REAL swap on the fork grows the pool → accrue fees ─────
      const swapKass = await h.fundTokenAccount(kass, swapper.publicKey, SWAP_KASS);
      const swYes = await h.createTokenAccount(refs.yesMint, swapper.publicKey, 0n);
      const swNo = await h.createTokenAccount(refs.noMint, swapper.publicKey, 0n);
      await h.sendIx(
        swapper,
        [
          await metadao.splitTokens({
            question: refs.question,
            vault: refs.vault,
            vaultUnderlyingAta: refs.vaultUnderlyingAta,
            authority: swapper.publicKey,
            userUnderlyingAta: swapKass,
            conditionalMints: [refs.yesMint, refs.noMint],
            userConditionalAtas: [swYes, swNo],
            amount: SWAP_KASS,
          }),
        ],
        [],
        CU_INTERACT,
      );
      await h.sendIx(
        swapper,
        [
          await metadao.swap({
            payer: swapper.publicKey,
            baseMint: refs.yesMint,
            quoteMint: refs.noMint,
            swapType: metadao.SwapType.Sell,
            inputAmount: SWAP_IN,
            outputAmountMin: 0n,
            userBase: swYes,
            userQuote: swNo,
          }),
        ],
        [],
        CU_INTERACT,
      );

      // ── Stage 4b: split → a cYES-only winner and a cNO-only loser ──────────
      const setupSingleLeg = async (user: Keypair, drainYes: boolean) => {
        const userKass = await h.fundTokenAccount(kass, user.publicKey, SPLIT_AMT);
        const userYes = await h.createTokenAccount(refs.yesMint, user.publicKey, 0n);
        const userNo = await h.createTokenAccount(refs.noMint, user.publicKey, 0n);
        await h.sendIx(
          user,
          [
            await metadao.splitTokens({
              question: refs.question,
              vault: refs.vault,
              vaultUnderlyingAta: refs.vaultUnderlyingAta,
              authority: user.publicKey,
              userUnderlyingAta: userKass,
              conditionalMints: [refs.yesMint, refs.noMint],
              userConditionalAtas: [userYes, userNo],
              amount: SPLIT_AMT,
            }),
          ],
          [],
          CU_INTERACT,
        );
        const drainFrom = drainYes ? userYes : userNo;
        const drainMint = drainYes ? refs.yesMint : refs.noMint;
        const sink = await h.createTokenAccount(drainMint, (await Keypair.generate()).publicKey, 0n);
        await h.sendIx(user, [splTransfer(drainFrom, sink, user.publicKey, SPLIT_AMT)]);
        expect(await h.tokenBalance(drainFrom)).toBe(0n);
        return { userKass, userYes, userNo };
      };

      const win = await setupSingleLeg(winner, /* drainYes */ false); // holds only cYES
      const lose = await setupSingleLeg(loser, /* drainYes */ true); // holds only cNO

      // ── Stage 5: oracle resolves YES (option 0) + resolveMarket ────────────
      await h.setOracleResolved(oracle, 0);
      await h.sendIx(
        payer,
        [
          await resolveMarket({
            market,
            oracle,
            question: refs.question,
            cvEventAuthority: refs.cvEventAuthority,
          }),
        ],
        [],
        CU_INTERACT,
      );
      m = decodeMarket(await h.getAccountData(market).then((d) => d!));
      expect(m.settled).toBe(true);
      expect(m.status).toBe(MarketStatus.Resolved);
      expect(m.feeCollected).toBe(false); // fee_bps > 0 → crank still pending
      const q = await h.waitForAccount(refs.question);
      expect(u32(q, Q_DENOMINATOR_OFFSET)).not.toBe(0); // question resolved
      expect(u32(q, Q_NUM0_OFFSET)).toBe(1); // YES numerator == 1
      expect(u32(q, Q_NUM1_OFFSET)).toBe(0); // NO numerator == 0

      // ── Stage 6: claim_lp BEFORE collect_fee is rejected (fee gate) ────────
      const earlyLp = await h.fundTokenAccount(refs.lpMint, creator.publicKey, 0n);
      await expect(
        h.sendIx(payer, [await claimLp({ market, contributor: creator.publicKey, contributorLpAta: earlyLp })]),
      ).rejects.toThrow();
      expect(await h.tokenBalance(earlyLp)).toBe(0n); // nothing moved

      // ── Stage 7: collect_fee → the futarchy receives the accrued KASS cut ──
      const feeBefore = await h.tokenBalance(feeDestination);
      expect(feeBefore).toBe(0n);
      await h.sendIx(payer, [await collectFeeInstruction({ refs, feeDestination })], [], CU_COLLECT);

      m = decodeMarket(await h.getAccountData(market).then((d) => d!));
      expect(m.feeCollected).toBe(true); // fee stamped
      const feeKass = await h.tokenBalance(feeDestination);
      expect(feeKass).toBeGreaterThan(0n); // THE ASSERTION: the fee landed at the futarchy dest
      expect(m.lpTotal).toBeLessThan(lpTotalAtActivation); // lpTotal cut by the fee slice
      const reducedTotal = m.lpTotal;

      // ── Stage 8: claimLp for BOTH contributors, off the REDUCED lpTotal ────
      // The FIRST claimer (open_contributions == 2) takes the floor pro-rata share;
      // the LAST claimer (open_contributions == 1) sweeps the ENTIRE remaining
      // lp_vault so it ends at exactly 0. Each claim CLOSES its Contribution (rent →
      // contributor), so afterward the Contribution PDA is gone.
      const aShare = expectedShare(reducedTotal, SEED_A, MIN_LIQ);
      expect(aShare).toBeGreaterThan(0n);
      const creatorContribPda = (await pda.contribution(market, creator.publicKey)).address;
      const c2ContribPda = (await pda.contribution(market, c2.publicKey)).address;

      // creator claims first (not the last claimer) → floor pro-rata aShare.
      await h.sendIx(payer, [await claimLp({ market, contributor: creator.publicKey, contributorLpAta: earlyLp })]);
      expect(await h.tokenBalance(earlyLp)).toBe(aShare);
      expect(await h.getAccountData(creatorContribPda)).toBeNull(); // Contribution closed
      expect(decodeMarket(await h.getAccountData(market).then((d) => d!)).openContributions).toBe(1);

      // c2 claims last (open_contributions == 1) → sweeps the entire remaining vault.
      const bLp = await h.fundTokenAccount(refs.lpMint, c2.publicKey, 0n);
      const remainingLp = await h.tokenBalance(refs.lpVault);
      await h.sendIx(payer, [await claimLp({ market, contributor: c2.publicKey, contributorLpAta: bLp })]);
      expect(await h.tokenBalance(bLp)).toBe(remainingLp);
      expect(await h.tokenBalance(refs.lpVault)).toBe(0n); // last claimer swept the vault to 0
      expect(await h.getAccountData(c2ContribPda)).toBeNull(); // Contribution closed
      expect(decodeMarket(await h.getAccountData(market).then((d) => d!)).openContributions).toBe(0);

      // ── Stage 9: redeem → winner paid 1:1, loser paid 0 ────────────────────
      const { instructions: winRedeem } = await redeemInstructions({
        refs,
        user: winner.publicKey,
        userKassAta: win.userKass,
        userYesAta: win.userYes,
        userNoAta: win.userNo,
      });
      await h.sendIx(winner, winRedeem, [], CU_INTERACT);
      const winnerOut = await h.tokenBalance(win.userKass);
      expect(winnerOut).toBe(SPLIT_AMT); // winning cYES pays 1:1, drained cNO pays 0

      const { instructions: loseRedeem } = await redeemInstructions({
        refs,
        user: loser.publicKey,
        userKassAta: lose.userKass,
        userYesAta: lose.userYes,
        userNoAta: lose.userNo,
      });
      await h.sendIx(loser, loseRedeem, [], CU_INTERACT);
      const loserOut = await h.tokenBalance(lose.userKass);
      expect(loserOut).toBe(0n); // losing cNO pays 0

      // ── Stage 10: conservation ─────────────────────────────────────────────
      expect(await h.tokenBalance(escrow)).toBe(0n);
      const totalIn = MIN_LIQ + SWAP_KASS + SPLIT_AMT + SPLIT_AMT; // crowdfunded + swap + both splits
      expect(winnerOut + loserOut + feeKass).toBeLessThanOrEqual(totalIn);
      expect(winnerOut).toBe(SPLIT_AMT); // traded-portion round trip is exact

      // ── Stage 11: closeMarket — reclaim ALL remaining rent to the creator ──
      // Resolved + fee_collected + open_contributions == 0 → the permissionless
      // crank SPL-CloseAccounts escrow/cyes/cno/lp_vault + closes the Market PDA,
      // routing every reclaimed rent lamport to the creator. MetaDAO accounts are
      // NOT ours (never closed), so the redeems above are unaffected.
      const creatorBeforeClose = await h.connection.getBalance(creator.publicKey);
      await h.sendIx(payer, [await closeMarket({ market, creator: creator.publicKey })]);
      // The Market PDA + all four market-owned token accounts are gone…
      expect(await h.getAccountData(market)).toBeNull();
      expect(await h.getAccountData(escrow)).toBeNull();
      expect(await h.getAccountData(refs.marketCyes)).toBeNull();
      expect(await h.getAccountData(refs.marketCno)).toBeNull();
      expect(await h.getAccountData(refs.lpVault)).toBeNull();
      // …and the creator received all of that rent.
      expect(await h.connection.getBalance(creator.publicKey)).toBeGreaterThan(creatorBeforeClose);
    }, 180_000);
  },
);

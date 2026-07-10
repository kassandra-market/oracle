/**
 * M2 surfpool METEORA DAMM v2 SPOT-PATH E2E (GATED, FORKED MAINNET) — verifies
 * the M1 zero-copy offsets against the DEPLOYED cp-amm binary.
 *
 * Boots surfpool FORKING MAINNET (`--network mainnet`) so the REAL Meteora
 * DAMM v2 program `cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG` executes over RPC,
 * clones a REAL mainnet cp-amm `Config` (a public/static config — index 0,
 * `8CNy9goNQNLM4wtgRw528tUQGMKD3vSuFRZY2gLGLLvF`, `pool_creator_authority ==
 * Pubkey::default`), fabricates two SPL mints + funded payer token accounts, and
 * DRIVES the M1 builders through the real program:
 *
 *   initializePool (creates Pool + first position, funded `liquidity`+`sqrt_price`,
 *                   mints the Token-2022 position NFT)
 *     → addLiquidity (deposits into that position)
 *     → swap (A→B; direction implicit in the token-account order; ACCRUES an LP fee)
 *     → createPosition (opens a second, empty position)
 *     → claimPositionFee (F1: sweeps the swap-accrued LP fee — NONZERO, in token B on
 *                         this Config's collect_fee_mode — to the owner; owner balance
 *                         rises by exactly the pending fee)
 *     → removeLiquidity (F1: withdraws ALL unlocked liquidity from the first
 *                        position; pool liquidity + both reserves fall, owner receives
 *                        the withdrawn amounts)
 *
 * All ixs go out with `skipPreflight:false` and confirm-throws-on-err, so a
 * rejected instruction FAILS the test. F1 completes LIVE coverage of all 6
 * cp-amm builders against the deployed binary (claim + remove were previously
 * unit-tested only).
 *
 * THE POINT — offset verification against the deployed layout:
 *   - after init, `decodePool` reads `sqrt_price` (abs offset 456), `liquidity`
 *     (360), `token_a_amount`/`token_b_amount` (680/688), mints (168/200) and they
 *     match the values we drove in (and the live vault balances);
 *   - `decodePosition` reads `unlocked_liquidity` (152) == the deposited liquidity;
 *   - after the A→B swap, `sqrt_price` MOVED DOWN and `token_a_amount` rose /
 *     `token_b_amount` fell — consistent with the trade;
 *   - a REAL mainnet pool is fetched from mainnet and `decodePool`d, asserting
 *     sqrt(price)² ≈ reserve_b/reserve_a (genuine deployed bytes → sane fields).
 * If any computed offset were wrong these reads would be garbage and the
 * assertions would fail — so passing them proves the offsets vs the deployed
 * binary.
 *
 * GATING: runs only under `KASSANDRA_E2E=1`; skips (not fails) when surfpool /
 * the `.so` are absent. Forks mainnet → needs network + is slower.
 */
import { Keypair } from "@solana/web3.js";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { meteora } from "../../src/index.js";

import { surfpoolReady } from "./harness.js";
import {
  ADD_LIQUIDITY,
  Fixture,
  INIT_LIQUIDITY,
  REAL_CONFIG,
  REAL_POOL,
  SQRT_MAX,
  SQRT_MIN,
  SQRT_PRICE_INIT,
  U64_MAX,
  deltaA,
  deltaB,
  fetchAccount,
  fetchMainnetAccount,
  sendIx,
  startFixture,
  tokenBalance,
} from "./meteora-spot-harness.js";

const ENABLED = process.env.KASSANDRA_E2E === "1" && surfpoolReady();

describe.skipIf(!ENABLED)("surfpool Meteora DAMM v2 spot path on FORKED mainnet (M2)", () => {
  let f: Fixture;

  beforeAll(async () => {
    f = await startFixture();
  }, 120_000);

  afterAll(async () => {
    await f?.harness.teardown();
  });

  it("cloned REAL mainnet Config decodes as public + static", async () => {
    const data = await fetchAccount(f, REAL_CONFIG);
    expect(data.length).toBe(328); // 8 + Config::INIT_SPACE(320)
    // pool_creator_authority @ abs 40 (struct 32) == Pubkey::default (public).
    expect(data.slice(40, 72).every((b) => b === 0)).toBe(true);
    // config_type @ abs 194 (struct 186) == 0 (Static).
    expect(data[194]).toBe(0);
  }, 60_000);

  it("decodes a REAL mainnet Pool (genuine deployed bytes): sqrt_price² ≈ reserveB/reserveA", async () => {
    const data = await fetchMainnetAccount(REAL_POOL);
    const p = meteora.decodePool(data);
    // token_b is USDC on this pool.
    expect(p.tokenBMint.toString()).toBe("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
    expect(p.sqrtPrice).toBeGreaterThan(SQRT_MIN);
    expect(p.sqrtPrice).toBeLessThan(SQRT_MAX);
    expect(p.liquidity).toBeGreaterThan(0n);
    expect(p.tokenAAmount).toBeGreaterThan(0n);
    expect(p.tokenBAmount).toBeGreaterThan(0n);
    // price = (sqrtPrice/2^64)^2 == token_b per token_a; must match the reserve
    // ratio to within 1% (proves sqrt_price@456 + reserves@680/688 are the real
    // fields, not garbage from a wrong offset).
    const priceScaled = (p.sqrtPrice * p.sqrtPrice) / (1n << 64n); // (price) * 2^64
    const ratioScaled = (p.tokenBAmount * (1n << 64n)) / p.tokenAAmount;
    const hi = priceScaled > ratioScaled ? priceScaled : ratioScaled;
    const lo = priceScaled > ratioScaled ? ratioScaled : priceScaled;
    expect(Number(hi) / Number(lo)).toBeLessThan(1.01);
  }, 60_000);

  it("drives initializePool → addLiquidity → swap → createPosition → claimPositionFee → removeLiquidity through the REAL cp-amm", async () => {
    const config = REAL_CONFIG;
    const poolAddr = (await meteora.pda.pool(config, f.mintA, f.mintB)).address;
    const tokenAVault = (await meteora.pda.tokenVault(f.mintA, poolAddr)).address;
    const tokenBVault = (await meteora.pda.tokenVault(f.mintB, poolAddr)).address;

    // --- initialize_pool (creates pool + first position, mints Token-2022 NFT) ---
    const posNftMint = await Keypair.generate();
    await sendIx(
      f,
      await meteora.initializePool({
        creator: f.payer.publicKey,
        payer: f.payer.publicKey,
        positionNftMint: posNftMint.publicKey,
        config,
        tokenAMint: f.mintA,
        tokenBMint: f.mintB,
        payerTokenA: f.payerTokenA,
        payerTokenB: f.payerTokenB,
        liquidity: INIT_LIQUIDITY,
        sqrtPrice: SQRT_PRICE_INIT,
      }),
      [posNftMint],
      1_400_000,
    );

    // --- OFFSET VERIFICATION #1: freshly-driven pool decodes to what we drove ---
    const expectA = deltaA(SQRT_PRICE_INIT, SQRT_MAX, INIT_LIQUIDITY);
    const expectB = deltaB(SQRT_MIN, SQRT_PRICE_INIT, INIT_LIQUIDITY);
    let pool = meteora.decodePool(await fetchAccount(f, poolAddr));
    expect(pool.tokenAMint.toString()).toBe(f.mintA.toString());
    expect(pool.tokenBMint.toString()).toBe(f.mintB.toString());
    expect(pool.tokenAVault.toString()).toBe(tokenAVault.toString());
    expect(pool.tokenBVault.toString()).toBe(tokenBVault.toString());
    expect(pool.sqrtPrice).toBe(SQRT_PRICE_INIT); // @456
    expect(pool.sqrtMinPrice).toBe(SQRT_MIN); // @424
    expect(pool.sqrtMaxPrice).toBe(SQRT_MAX); // @440
    expect(pool.liquidity).toBe(INIT_LIQUIDITY); // @360
    expect(pool.tokenAAmount).toBe(expectA); // @680
    expect(pool.tokenBAmount).toBe(expectB); // @688
    // Decoded reserves match the live on-chain vault balances.
    expect(await tokenBalance(f, tokenAVault)).toBe(expectA);
    expect(await tokenBalance(f, tokenBVault)).toBe(expectB);

    // Position decode: unlocked_liquidity @152 == the initial liquidity.
    const positionAddr = (await meteora.pda.position(posNftMint.publicKey)).address;
    const posNftAccount = (await meteora.pda.positionNftAccount(posNftMint.publicKey)).address;
    let position = meteora.decodePosition(await fetchAccount(f, positionAddr));
    expect(position.pool.toString()).toBe(poolAddr.toString());
    expect(position.nftMint.toString()).toBe(posNftMint.publicKey.toString());
    expect(position.unlockedLiquidity).toBe(INIT_LIQUIDITY);

    // --- add_liquidity into that position ---
    await sendIx(
      f,
      await meteora.addLiquidity({
        pool: poolAddr,
        position: positionAddr,
        tokenAAccount: f.payerTokenA,
        tokenBAccount: f.payerTokenB,
        tokenAVault,
        tokenBVault,
        tokenAMint: f.mintA,
        tokenBMint: f.mintB,
        positionNftAccount: posNftAccount,
        signer: f.payer.publicKey,
        liquidityDelta: ADD_LIQUIDITY,
        tokenAAmountThreshold: U64_MAX,
        tokenBAmountThreshold: U64_MAX,
      }),
      [],
      1_400_000,
    );

    // OFFSET VERIFICATION #2: unlocked_liquidity + pool liquidity rose by delta.
    position = meteora.decodePosition(await fetchAccount(f, positionAddr));
    expect(position.unlockedLiquidity).toBe(INIT_LIQUIDITY + ADD_LIQUIDITY);
    pool = meteora.decodePool(await fetchAccount(f, poolAddr));
    expect(pool.liquidity).toBe(INIT_LIQUIDITY + ADD_LIQUIDITY);

    const sqrtBeforeSwap = pool.sqrtPrice;
    const aBeforeSwap = pool.tokenAAmount;
    const bBeforeSwap = pool.tokenBAmount;

    // --- swap A→B (sell token A): input = payer A, output = payer B ---
    const amountIn = 100_000_000n;
    await sendIx(
      f,
      await meteora.swap({
        pool: poolAddr,
        inputTokenAccount: f.payerTokenA,
        outputTokenAccount: f.payerTokenB,
        tokenAVault,
        tokenBVault,
        tokenAMint: f.mintA,
        tokenBMint: f.mintB,
        payer: f.payer.publicKey,
        amountIn,
        minimumAmountOut: 0n,
      }),
      [],
      1_400_000,
    );

    // OFFSET VERIFICATION #3: A→B moved sqrt_price DOWN, reserves consistent.
    pool = meteora.decodePool(await fetchAccount(f, poolAddr));
    expect(pool.sqrtPrice).toBeLessThan(sqrtBeforeSwap); // selling A lowers sqrt(B/A)
    expect(pool.tokenAAmount).toBeGreaterThan(aBeforeSwap); // A reserve grew by ~amountIn
    expect(pool.tokenBAmount).toBeLessThan(bBeforeSwap); // B reserve shrank
    expect(pool.tokenAAmount - aBeforeSwap).toBe(amountIn); // exact A in (no transfer fee)
    expect(pool.liquidity).toBe(INIT_LIQUIDITY + ADD_LIQUIDITY); // liquidity unchanged by swap
    // Decoded reserves still track the live vaults: each vault holds the tracked
    // reserve PLUS any LP trading fee accrued by the swap (fees sit in the vault
    // but are not counted in token_{a,b}_amount), so vault >= decoded reserve.
    expect(await tokenBalance(f, tokenAVault)).toBeGreaterThanOrEqual(pool.tokenAAmount);
    expect(await tokenBalance(f, tokenBVault)).toBeGreaterThanOrEqual(pool.tokenBAmount);

    // --- create_position (a SECOND, empty position) ---
    const pos2Mint = await Keypair.generate();
    await sendIx(
      f,
      await meteora.createPosition({
        owner: f.payer.publicKey,
        positionNftMint: pos2Mint.publicKey,
        pool: poolAddr,
        payer: f.payer.publicKey,
      }),
      [pos2Mint],
      1_400_000,
    );
    const pos2Addr = (await meteora.pda.position(pos2Mint.publicKey)).address;
    const pos2 = meteora.decodePosition(await fetchAccount(f, pos2Addr));
    expect(pos2.pool.toString()).toBe(poolAddr.toString());
    expect(pos2.unlockedLiquidity).toBe(0n); // freshly opened, empty

    // ========================================================================
    // F1 — claim_position_fee: sweep the swap-accrued LP fee to the owner
    // ========================================================================
    // The A→B swaps accrue an LP trading fee. On THIS cloned public Config the
    // cp-amm `collect_fee_mode` collects fees in token B (the quote side) for
    // BOTH swap directions (empirically: after the A→B swaps `fee_b_pending`/
    // `protocol_b_fee` are nonzero while the A side stays 0), so the nonzero
    // claimable fee lands in token B. cp-amm updates a position's fee LAZILY
    // (only when the position is touched), so: (1) do a couple more A→B swaps to
    // grow a healthy fee, (2) a TINY addLiquidity to CHECKPOINT the accrued fee
    // onto the position (making `fee_b_pending` nonzero + decodable BEFORE the
    // claim), then claim.
    for (const extra of [200_000_000n, 200_000_000n]) {
      await sendIx(
        f,
        await meteora.swap({
          pool: poolAddr,
          inputTokenAccount: f.payerTokenA,
          outputTokenAccount: f.payerTokenB,
          tokenAVault,
          tokenBVault,
          tokenAMint: f.mintA,
          tokenBMint: f.mintB,
          payer: f.payer.publicKey,
          amountIn: extra,
          minimumAmountOut: 0n,
        }),
        [],
        1_400_000,
      );
    }
    // Tiny addLiquidity → forces `update_position_fee` → `fee_a_pending` populates.
    await sendIx(
      f,
      await meteora.addLiquidity({
        pool: poolAddr,
        position: positionAddr,
        tokenAAccount: f.payerTokenA,
        tokenBAccount: f.payerTokenB,
        tokenAVault,
        tokenBVault,
        tokenAMint: f.mintA,
        tokenBMint: f.mintB,
        positionNftAccount: posNftAccount,
        signer: f.payer.publicKey,
        liquidityDelta: 1n << 64n,
        tokenAAmountThreshold: U64_MAX,
        tokenBAmountThreshold: U64_MAX,
      }),
      [],
      1_400_000,
    );

    // Position now carries a NONZERO pending fee in token B (this Config's
    // collect_fee_mode). The A side stays 0. The nonzero B fee is the real,
    // claimable LP fee — asserting it proves a genuine (not no-op) claim.
    position = meteora.decodePosition(await fetchAccount(f, positionAddr));
    const feeA = position.feeAPending;
    const feeB = position.feeBPending;
    expect(feeB).toBeGreaterThan(0n); // real accrued LP fee (token B) — NOT a no-op claim

    const ownerABeforeClaim = await tokenBalance(f, f.payerTokenA);
    const ownerBBeforeClaim = await tokenBalance(f, f.payerTokenB);

    await sendIx(
      f,
      await meteora.claimPositionFee({
        pool: poolAddr,
        position: positionAddr,
        tokenAAccount: f.payerTokenA,
        tokenBAccount: f.payerTokenB,
        tokenAVault,
        tokenBVault,
        tokenAMint: f.mintA,
        tokenBMint: f.mintB,
        positionNftAccount: posNftAccount,
        signer: f.payer.publicKey,
      }),
      [],
      1_400_000,
    );

    // The owner's token-B account rose by EXACTLY the claimed fee (nonzero, real
    // transfer), token-A by feeA (0 here), and the Position's pending fees cleared.
    const ownerAAfterClaim = await tokenBalance(f, f.payerTokenA);
    const ownerBAfterClaim = await tokenBalance(f, f.payerTokenB);
    expect(ownerBAfterClaim - ownerBBeforeClaim).toBe(feeB);
    expect(ownerBAfterClaim - ownerBBeforeClaim).toBeGreaterThan(0n);
    expect(ownerAAfterClaim - ownerABeforeClaim).toBe(feeA);
    position = meteora.decodePosition(await fetchAccount(f, positionAddr));
    expect(position.feeAPending).toBe(0n); // swept
    expect(position.feeBPending).toBe(0n);

    // ========================================================================
    // F1 — remove_liquidity: withdraw ALL unlocked liquidity from the position
    // ========================================================================
    pool = meteora.decodePool(await fetchAccount(f, poolAddr));
    position = meteora.decodePosition(await fetchAccount(f, positionAddr));
    const removeDelta = position.unlockedLiquidity; // remove everything
    expect(removeDelta).toBeGreaterThan(0n);
    const poolLiqBeforeRemove = pool.liquidity;
    const poolABeforeRemove = pool.tokenAAmount;
    const poolBBeforeRemove = pool.tokenBAmount;
    const ownerABeforeRemove = await tokenBalance(f, f.payerTokenA);
    const ownerBBeforeRemove = await tokenBalance(f, f.payerTokenB);

    await sendIx(
      f,
      await meteora.removeLiquidity({
        pool: poolAddr,
        position: positionAddr,
        tokenAAccount: f.payerTokenA,
        tokenBAccount: f.payerTokenB,
        tokenAVault,
        tokenBVault,
        tokenAMint: f.mintA,
        tokenBMint: f.mintB,
        positionNftAccount: posNftAccount,
        signer: f.payer.publicKey,
        liquidityDelta: removeDelta,
        tokenAAmountThreshold: 0n, // MIN to receive — no lower bound
        tokenBAmountThreshold: 0n,
      }),
      [],
      1_400_000,
    );

    pool = meteora.decodePool(await fetchAccount(f, poolAddr));
    position = meteora.decodePosition(await fetchAccount(f, positionAddr));
    const ownerAAfterRemove = await tokenBalance(f, f.payerTokenA);
    const ownerBAfterRemove = await tokenBalance(f, f.payerTokenB);

    // Position's unlocked_liquidity dropped by the full delta (→ 0).
    expect(position.unlockedLiquidity).toBe(0n);
    // Pool liquidity dropped by exactly the removed delta.
    expect(pool.liquidity).toBe(poolLiqBeforeRemove - removeDelta);
    // Both tracked reserves fell.
    expect(pool.tokenAAmount).toBeLessThan(poolABeforeRemove);
    expect(pool.tokenBAmount).toBeLessThan(poolBBeforeRemove);
    // The owner received the withdrawn amounts == the reserve deltas (exact).
    expect(ownerAAfterRemove - ownerABeforeRemove).toBe(poolABeforeRemove - pool.tokenAAmount);
    expect(ownerBAfterRemove - ownerBBeforeRemove).toBe(poolBBeforeRemove - pool.tokenBAmount);
    expect(ownerAAfterRemove - ownerABeforeRemove).toBeGreaterThan(0n);
    expect(ownerBAfterRemove - ownerBBeforeRemove).toBeGreaterThan(0n);
  }, 300_000);
});

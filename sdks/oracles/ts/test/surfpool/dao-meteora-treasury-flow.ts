/**
 * D1 surfpool DAO-OWNED METEORA TREASURY-FEE CLAIM E2E — higher-level flow
 * builders (fork fixture bootstrap, DAO bootstrap, vault-owned position setup).
 * Extracted from `dao-meteora-treasury-e2e.test.ts` so no single file exceeds
 * ~400 lines. Pure move: the bodies (and their `expect`s) are verbatim; the
 * only adjustment is returning state instead of assigning module globals so the
 * two test files can each drive the flow.
 */
import { Address, Keypair } from "@solana/web3.js";
import { expect } from "vitest";

import { meteora } from "../../src/index.js";
import { TOKEN_PROGRAM_ID } from "../../src/constants.js";
import * as futarchy from "../../src/futarchy/index.js";

import { SurfpoolHarness, mintBytes, toHex } from "./harness.js";

import type { Fixture, MeteoraState } from "./dao-meteora-treasury-harness.js";
import {
  INIT_LIQUIDITY,
  PROBE_LIQUIDITY,
  REAL_CONFIG,
  SQRT_PRICE_INIT,
  U64_MAX,
  fabricateToken,
  fetchAccount,
  fetchMainnetAccount,
  sendIx,
} from "./dao-meteora-treasury-harness.js";

/**
 * Boot surfpool forking mainnet, fund the payer, materialise the KASS/USDC mints,
 * clone the REAL cp-amm Config, warm the cp-amm + Squads programs, and return the
 * seeded fixture (`dao`/`multisig`/`vault` filled in later by `bootstrapFlow`).
 */
export async function startForkFixture(port = 8924): Promise<Fixture> {
  const harness = await SurfpoolHarness.start({
    port,
    fork: "mainnet",
    readyTimeoutMs: 60_000,
  });
  const payer = await Keypair.generate();
  await harness.airdrop(payer.publicKey.toString(), 1_000_000_000_000);

  // Real KASS (9dp) + USDC (MUST be 6dp — initialize_dao `mint::decimals = 6`).
  const kassMint = await Keypair.generate();
  const usdcMint = await Keypair.generate();
  await harness.setAccount(kassMint.publicKey.toString(), {
    lamports: 1_000_000_000,
    owner: TOKEN_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(mintBytes(payer.publicKey.toBytes(), 0n, 9)),
  });
  await harness.setAccount(usdcMint.publicKey.toString(), {
    lamports: 1_000_000_000,
    owner: TOKEN_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(mintBytes(payer.publicKey.toBytes(), 0n, 6)),
  });

  // Clone the REAL mainnet cp-amm Config onto the fork (its bytes, cp-amm-owned).
  const cfg = await fetchMainnetAccount(REAL_CONFIG);
  await harness.setAccount(REAL_CONFIG.toString(), {
    lamports: 5_000_000,
    owner: meteora.METEORA_DAMM_V2_ID.toString(),
    executable: false,
    data: toHex(cfg),
  });
  // Warm the cp-amm + Squads programs the flow references.
  await harness.connection.getAccountInfo(meteora.METEORA_DAMM_V2_ID);
  await harness.connection.getAccountInfo(futarchy.SQUADS_V4_ID);

  return {
    harness,
    payer,
    kassMint,
    usdcMint,
    dao: undefined as unknown as Address,
    multisig: undefined as unknown as Address,
    vault: undefined as unknown as Address,
  };
}

/**
 * BOOTSTRAP: real `initialize_dao` → the `Dao` + its Squads multisig/vault (the
 * DAO treasury authority). Mutates `f.dao`/`f.multisig`/`f.vault`.
 */
export async function bootstrapFlow(f: Fixture): Promise<void> {
  // Squads ProgramConfig.treasury is NOT a PDA — read it live (treasury @48).
  const programConfig = (await futarchy.pda.squadsProgramConfig()).address;
  const pcInfo = await f.harness.connection.getAccountInfo(programConfig);
  expect(pcInfo, "Squads ProgramConfig not on the fork").not.toBeNull();
  const treasury = new Address(pcInfo!.data.slice(48, 80));

  const nonce = 1n;
  const dao = (await futarchy.pda.dao(f.payer.publicKey, nonce)).address;
  const multisig = (await futarchy.pda.squadsMultisig(dao)).address;
  const vault = (await futarchy.pda.squadsVault(multisig, 0)).address;

  await sendIx(
    f,
    await futarchy.initializeDao({
      daoCreator: f.payer.publicKey,
      payer: f.payer.publicKey,
      baseMint: f.kassMint.publicKey,
      quoteMint: f.usdcMint.publicKey,
      squadsProgramConfigTreasury: treasury,
      // TWAP params: observable immediately (start_delay 0), tiny windows so the
      // verdict arm can timeTravel past them (mirrors G3).
      twapInitialObservation: 1_000_000_000_000n,
      twapMaxObservationChangePerUpdate: 1_000_000_000_000n,
      twapStartDelaySeconds: 0,
      minQuoteFutarchicLiquidity: 1n,
      minBaseFutarchicLiquidity: 1n,
      baseToStake: 0n,
      passThresholdBps: 0,
      secondsPerProposal: 86_400,
      nonce,
    }),
    [],
    1_400_000,
  );

  // The Dao + its Squads multisig really exist (created by the deployed program).
  const daoInfo = await f.harness.connection.getAccountInfo(dao);
  expect(daoInfo!.owner.toString()).toBe(futarchy.FUTARCHY_ID.toString());
  const msInfo = await f.harness.connection.getAccountInfo(multisig);
  expect(msInfo!.owner.toString()).toBe(futarchy.SQUADS_V4_ID.toString());

  f.dao = dao;
  f.multisig = multisig;
  f.vault = vault;
}

/**
 * VAULT-OWNED METEORA POSITION: the DAO's Squads vault OWNS a funded cp-amm
 * position that accrues a nonzero token-B fee. Returns the Meteora state the
 * governance-claim flow reads.
 */
export async function positionFlow(f: Fixture): Promise<MeteoraState> {
  const config = REAL_CONFIG;
  const poolAddr = (await meteora.pda.pool(config, f.kassMint.publicKey, f.usdcMint.publicKey)).address;
  const tokenAVault = (await meteora.pda.tokenVault(f.kassMint.publicKey, poolAddr)).address;
  const tokenBVault = (await meteora.pda.tokenVault(f.usdcMint.publicKey, poolAddr)).address;

  // Payer source token accounts, funded far above what init/swap will pull.
  const payerKass = await fabricateToken(f, f.kassMint.publicKey, f.payer.publicKey, 10n ** 15n);
  const payerUsdc = await fabricateToken(f, f.usdcMint.publicKey, f.payer.publicKey, 10n ** 15n);

  // --- initialize_pool with creator == the Squads VAULT ----------------------
  // cp-amm mints the FUNDED first position's NFT to `creator` (an UncheckedAccount,
  // `token::authority = creator`), pulling the liquidity from the payer. So the
  // DAO's vault directly OWNS the funded position — no NFT transfer needed.
  const vaultPosNftMint = await Keypair.generate();
  await sendIx(
    f,
    await meteora.initializePool({
      creator: f.vault, // the DAO's Squads vault OWNS the position
      payer: f.payer.publicKey,
      positionNftMint: vaultPosNftMint.publicKey,
      config,
      tokenAMint: f.kassMint.publicKey,
      tokenBMint: f.usdcMint.publicKey,
      payerTokenA: payerKass,
      payerTokenB: payerUsdc,
      liquidity: INIT_LIQUIDITY,
      sqrtPrice: SQRT_PRICE_INIT,
    }),
    [vaultPosNftMint],
    1_400_000,
  );

  const vaultPos = (await meteora.pda.position(vaultPosNftMint.publicKey)).address;
  const vaultPosNftAccount = (await meteora.pda.positionNftAccount(vaultPosNftMint.publicKey)).address;

  // VERIFY OWNERSHIP: the position NFT account's authority (owner @ offset 32)
  // is the DAO's Squads vault — the position is genuinely DAO-owned.
  const nftAcctData = await fetchAccount(f, vaultPosNftAccount);
  const nftOwner = new Address(nftAcctData.slice(32, 64));
  expect(nftOwner.toString()).toBe(f.vault.toString());
  let vpos = meteora.decodePosition(await fetchAccount(f, vaultPos));
  expect(vpos.pool.toString()).toBe(poolAddr.toString());
  expect(vpos.nftMint.toString()).toBe(vaultPosNftMint.publicKey.toString());
  expect(vpos.unlockedLiquidity).toBe(INIT_LIQUIDITY);

  // --- a payer-owned PROBE position (to DECODE fee accrual pre-claim) ---------
  const probeNftMint = await Keypair.generate();
  await sendIx(
    f,
    await meteora.createPosition({
      owner: f.payer.publicKey,
      positionNftMint: probeNftMint.publicKey,
      pool: poolAddr,
      payer: f.payer.publicKey,
    }),
    [probeNftMint],
    1_400_000,
  );
  const probePos = (await meteora.pda.position(probeNftMint.publicKey)).address;
  const probeNftAccount = (await meteora.pda.positionNftAccount(probeNftMint.publicKey)).address;
  await sendIx(
    f,
    await meteora.addLiquidity({
      pool: poolAddr,
      position: probePos,
      tokenAAccount: payerKass,
      tokenBAccount: payerUsdc,
      tokenAVault,
      tokenBVault,
      tokenAMint: f.kassMint.publicKey,
      tokenBMint: f.usdcMint.publicKey,
      positionNftAccount: probeNftAccount,
      signer: f.payer.publicKey,
      liquidityDelta: PROBE_LIQUIDITY,
      tokenAAmountThreshold: U64_MAX,
      tokenBAmountThreshold: U64_MAX,
    }),
    [],
    1_400_000,
  );

  // --- A→B swaps (sell KASS for USDC) accrue a token-B (quote) LP fee ---------
  // On this cloned public Config the collect_fee_mode collects in token B for
  // both directions (F1 finding), so `fee_b_pending` grows on every LP position.
  for (const amt of [200_000_000n, 200_000_000n, 200_000_000n, 200_000_000n, 200_000_000n]) {
    await sendIx(
      f,
      await meteora.swap({
        pool: poolAddr,
        inputTokenAccount: payerKass,
        outputTokenAccount: payerUsdc,
        tokenAVault,
        tokenBVault,
        tokenAMint: f.kassMint.publicKey,
        tokenBMint: f.usdcMint.publicKey,
        payer: f.payer.publicKey,
        amountIn: amt,
        minimumAmountOut: 0n,
      }),
      [],
      1_400_000,
    );
  }

  // Checkpoint the PROBE (cp-amm updates position fees lazily) → decode a nonzero
  // token-B pending fee: proof the pool accrues real quote-side fees. The
  // vault-owned position (larger liquidity) accrues even more.
  await sendIx(
    f,
    await meteora.addLiquidity({
      pool: poolAddr,
      position: probePos,
      tokenAAccount: payerKass,
      tokenBAccount: payerUsdc,
      tokenAVault,
      tokenBVault,
      tokenAMint: f.kassMint.publicKey,
      tokenBMint: f.usdcMint.publicKey,
      positionNftAccount: probeNftAccount,
      signer: f.payer.publicKey,
      liquidityDelta: 1n << 64n,
      tokenAAmountThreshold: U64_MAX,
      tokenBAmountThreshold: U64_MAX,
    }),
    [],
    1_400_000,
  );
  const probe = meteora.decodePosition(await fetchAccount(f, probePos));
  expect(probe.feeBPending).toBeGreaterThan(0n); // real quote-side LP fee accruing

  // return Meteora state for the governance-claim flow
  return {
    poolAddr,
    tokenAVault,
    tokenBVault,
    vaultPos,
    vaultPosNftMint: vaultPosNftMint.publicKey,
    vaultPosNftAccount,
  };
}

/**
 * D1 — DAO-OWNED, ADMIN-FREE METEORA TREASURY-FEE CLAIM (GATED, FORKED MAINNET).
 *
 * The FIX for the F2a/F2b finding: a futarchy DAO collects its OWN Meteora
 * cp-amm LP fees WITHOUT any MetaDAO admin. The DAO's Squads vault OWNS the
 * Meteora position, and the fee claim is authorized by the DAO's own
 * governance — a real futarchy proposal whose PASS verdict CPI-approves a Squads
 * `vault_transaction` that `invoke_signed`s the cp-amm `claim_position_fee` as
 * the vault, sweeping the fee to the DAO's OWN token accounts. NO
 * `collect_meteora_damm_fees`, NO `metadao_admin` (`tSTp6B6k…`), NO MetaDAO
 * protocol vault (`6awyHMsh…`) anywhere in the flow.
 *
 * Boots surfpool FORKING MAINNET so the deployed programs execute over RPC:
 * futarchy v0.6.1 `FUTAREL…`, conditional_vault `VLTX1ish…`, Squads v4
 * `SQDS4ep6…`, Meteora DAMM v2 (cp-amm) `cpamd…`.
 *
 * Flow (all through the REAL programs, `skipPreflight:false`, confirm-throws):
 *   1. BOOTSTRAP — real `initialize_dao` → the `Dao` + its Squads multisig/vault
 *      (the DAO treasury authority). (Same bootstrap as F2b/G3.)
 *   2. VAULT-OWNED METEORA POSITION — clone a REAL public cp-amm `Config`,
 *      `initialize_pool` with `creator == the Squads vault` so the funded first
 *      position's NFT is minted straight to the vault (cp-amm `creator` is an
 *      unchecked non-signer; `token::authority = creator`), verified by decoding
 *      the position NFT account's authority == the vault. A payer-owned PROBE
 *      position + A→B swaps accrue a nonzero token-B (quote) LP fee; the probe is
 *      checkpointed to DECODE `fee_b_pending > 0` (proof the pool accrues real
 *      fees; the vault position — larger liquidity — accrues more).
 *   3. GOVERNANCE CLAIM — stage the cp-amm `claim_position_fee` (owner == the
 *      vault, recipients == the DAO's OWN vault-owned ATAs) as a Squads
 *      `vault_transaction_create` → `proposal_create`, then run a REAL futarchy
 *      proposal to a PASS TWAP verdict (G3's machinery) so `finalize_proposal`
 *      CPI-approves the Squads proposal, then `vault_transaction_execute` (member
 *      = the public permissionless member) `invoke_signed`s the claim as the vault.
 *   4. ASSERT — the DAO's ATA received the accrued fee (NONZERO delta), the vault
 *      position's `fee_b_pending` cleared to 0, and NO MetaDAO admin/vault appears
 *      in ANY account of the claim / staged message / execute remaining-accounts.
 *
 * GATING: `KASSANDRA_E2E=1`; skips (not fails) when surfpool/.so absent. Forks
 * mainnet → needs network + is slower.
 */
import type { AccountMeta } from "@solana/web3.js";
import {
  Address,
  ComputeBudgetProgram,
  Keypair,
  Transaction,
  TransactionInstruction,
} from "@solana/web3.js";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { meteora } from "../../src/index.js";
import { TOKEN_PROGRAM_ID } from "../../src/constants.js";
import * as futarchy from "../../src/futarchy/index.js";

import {
  SurfpoolHarness,
  mintBytes,
  surfpoolReady,
  toHex,
  tokenAccountAmount,
  tokenAccountBytes,
} from "./harness.js";

const ENABLED = process.env.KASSANDRA_E2E === "1" && surfpoolReady();

const MAINNET_RPC = "https://api.mainnet-beta.solana.com";

const ATA_PROGRAM_ID = new Address("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

/** A REAL public + static mainnet cp-amm Config (index 0): `pool_creator_authority
 * == Pubkey::default` (permissionless). Cloned onto the fork so `initialize_pool`
 * accepts our arbitrary payer as pool creator (same as M2). */
const REAL_CONFIG = new Address("8CNy9goNQNLM4wtgRw528tUQGMKD3vSuFRZY2gLGLLvF");

/** Full-range price bounds baked into the public configs (Q64.64). */
const SQRT_MIN = 4295048016n;
const SQRT_PRICE_INIT = 1n << 64n; // price 1.0
const INIT_LIQUIDITY = 1_000_000_000n * (1n << 64n); // vault position (the main LP)
const PROBE_LIQUIDITY = INIT_LIQUIDITY / 2n; // payer probe position
const U64_MAX = (1n << 64n) - 1n;

/**
 * MetaDAO's PUBLIC "permissionless" member keypair (futarchy
 * `sdk/permissionless-account.json` → EP3SoC2…), the multisig's only Initiate
 * member. Its secret is published by design; it (not the Dao) creates + executes
 * the Squads VaultTransaction (mirrors G3).
 */
const PERMISSIONLESS_SECRET = Uint8Array.from([
  249, 158, 188, 171, 243, 143, 1, 48, 87, 243, 209, 153, 144, 106, 23, 88, 161, 209, 65, 217,
  199, 121, 0, 250, 3, 203, 133, 138, 141, 112, 243, 38, 198, 205, 120, 222, 160, 224, 151, 190,
  84, 254, 127, 178, 224, 195, 130, 243, 145, 73, 20, 91, 9, 69, 222, 184, 23, 1, 2, 196, 202,
  206, 153, 192,
]);

interface Fixture {
  harness: SurfpoolHarness;
  payer: Keypair;
  kassMint: Keypair;
  usdcMint: Keypair;
  dao: Address;
  multisig: Address;
  vault: Address;
}

describe.skipIf(!ENABLED)("surfpool DAO-owned admin-free Meteora treasury-fee claim on FORKED mainnet (D1)", () => {
  let f: Fixture;

  beforeAll(async () => {
    const harness = await SurfpoolHarness.start({
      port: 8924,
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

    f = {
      harness,
      payer,
      kassMint,
      usdcMint,
      dao: undefined as unknown as Address,
      multisig: undefined as unknown as Address,
      vault: undefined as unknown as Address,
    };
  }, 120_000);

  afterAll(async () => {
    await f?.harness.teardown();
  });

  it("BOOTSTRAP: real initialize_dao → Dao + Squads multisig/vault (the DAO treasury authority)", async () => {
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
  }, 180_000);

  it("DAO's Squads vault OWNS a funded cp-amm position that accrues a nonzero token-B fee", async () => {
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

    // stash Meteora state on the fixture for the governance-claim test
    m = {
      poolAddr,
      tokenAVault,
      tokenBVault,
      vaultPos,
      vaultPosNftMint: vaultPosNftMint.publicKey,
      vaultPosNftAccount,
    };
  }, 300_000);

  it("GOVERNANCE CLAIM: futarchy PASS → Squads vault_transaction_execute claims the DAO's fee to its OWN ATAs (admin-free)", async () => {
    const permissionless = await Keypair.fromSecretKey(PERMISSIONLESS_SECRET);

    // --- the DAO's OWN fee-recipient ATAs (owned by the Squads vault treasury) --
    const daoFeeA = await ata(f.vault, f.kassMint.publicKey); // base (KASS)
    const daoFeeB = await ata(f.vault, f.usdcMint.publicKey); // quote (USDC) — the fee side
    await fabricateTokenAt(f, daoFeeA, f.kassMint.publicKey, f.vault, 0n);
    await fabricateTokenAt(f, daoFeeB, f.usdcMint.publicKey, f.vault, 0n);

    // --- the inner cp-amm claim_position_fee: owner == the vault, dest == DAO ATAs
    const claimIx = await meteora.claimPositionFee({
      pool: m.poolAddr,
      position: m.vaultPos,
      tokenAAccount: daoFeeA,
      tokenBAccount: daoFeeB,
      tokenAVault: m.tokenAVault,
      tokenBVault: m.tokenBVault,
      tokenAMint: f.kassMint.publicKey,
      tokenBMint: f.usdcMint.publicKey,
      positionNftAccount: m.vaultPosNftAccount,
      signer: f.vault, // the DAO's Squads vault (Squads invoke_signs it on execute)
    });

    // ADMIN-FREE assertion #1: the MetaDAO admin/vault appear NOWHERE in the claim.
    assertNoMetaDao(claimIx.keys.map((k) => k.pubkey));

    // --- stage the claim as a Squads VaultTransaction --------------------------
    const txIndex = 1n;
    const { message, remainingAccounts } = compileSquadsMessage(claimIx, f.vault);
    // ADMIN-FREE assertion #2: no MetaDAO admin/vault in the staged message keys.
    assertNoMetaDao(remainingAccounts.map((k) => k.pubkey));

    await sendIx(
      f,
      await futarchy.vaultTransactionCreate({
        multisig: f.multisig,
        creator: permissionless.publicKey,
        rentPayer: f.payer.publicKey,
        transactionIndex: txIndex,
        transactionMessage: message,
      }),
      [permissionless],
    );
    await sendIx(
      f,
      await futarchy.proposalCreate({
        multisig: f.multisig,
        creator: permissionless.publicKey,
        rentPayer: f.payer.publicKey,
        transactionIndex: txIndex,
        draft: false, // → ProposalStatus::Active (required by initialize_proposal)
      }),
      [permissionless],
    );
    const squadsProposal = (await futarchy.pda.squadsProposal(f.multisig, txIndex)).address;

    // --- run a REAL futarchy proposal to a PASS verdict (G3 machinery) ----------
    // Only a PASS `finalize_proposal` CPI-approves the Squads proposal (threshold
    // 1; the sole Vote member is the Dao PDA) — the DAO's governance is what
    // authorizes the vault_transaction to execute.
    const ammBaseVault = await ata(f.dao, f.kassMint.publicKey);
    const ammQuoteVault = await ata(f.dao, f.usdcMint.publicKey);

    // (1) seed the embedded spot AMM (fabricated LP balances; provide_liquidity real).
    const QUOTE_LIQ = 1_000_000_000n;
    const BASE_LIQ = 1_000_000_000n;
    const lpQuote = await fabricateToken(f, f.usdcMint.publicKey, f.payer.publicKey, QUOTE_LIQ);
    const lpBase = await fabricateToken(f, f.kassMint.publicKey, f.payer.publicKey, BASE_LIQ);
    await sendIx(
      f,
      await futarchy.provideLiquidity({
        dao: f.dao,
        liquidityProvider: f.payer.publicKey,
        liquidityProviderBaseAccount: lpBase,
        liquidityProviderQuoteAccount: lpQuote,
        payer: f.payer.publicKey,
        ammBaseVault,
        ammQuoteVault,
        positionAuthority: f.payer.publicKey,
        quoteAmount: QUOTE_LIQ,
        maxBaseAmount: BASE_LIQ,
        minLiquidity: 0n,
      }),
      [],
      400_000,
    );

    // (2) conditional question (oracle == futarchy Proposal PDA) + vaults.
    const futProposal = (await futarchy.pda.proposal(squadsProposal)).address;
    const questionId = new Uint8Array(32).fill(0x44);
    const question = (await futarchy.pda.question(questionId, futProposal, 2)).address;
    await sendIx(
      f,
      await futarchy.initializeQuestion({ questionId, oracle: futProposal, numOutcomes: 2, payer: f.payer.publicKey }),
      [],
      400_000,
    );
    const baseV = await condVault(f, question, f.kassMint.publicKey);
    const quoteV = await condVault(f, question, f.usdcMint.publicKey);

    // (3) initialize_proposal + launch_proposal.
    await sendIx(
      f,
      await futarchy.initializeProposal({
        squadsProposal,
        squadsMultisig: f.multisig,
        dao: f.dao,
        question,
        quoteVault: quoteV.vault,
        baseVault: baseV.vault,
        proposer: f.payer.publicKey,
        payer: f.payer.publicKey,
      }),
      [],
      400_000,
    );
    const ammPassBase = await ata(f.dao, baseV.passMint);
    const ammPassQuote = await ata(f.dao, quoteV.passMint);
    const ammFailBase = await ata(f.dao, baseV.failMint);
    const ammFailQuote = await ata(f.dao, quoteV.failMint);
    await sendIx(
      f,
      await futarchy.launchProposal({
        proposal: futProposal,
        baseVault: baseV.vault,
        quoteVault: quoteV.vault,
        passBaseMint: baseV.passMint,
        passQuoteMint: quoteV.passMint,
        failBaseMint: baseV.failMint,
        failQuoteMint: quoteV.failMint,
        dao: f.dao,
        payer: f.payer.publicKey,
        ammPassBaseVault: ammPassBase,
        ammPassQuoteVault: ammPassQuote,
        ammFailBaseVault: ammFailBase,
        ammFailQuoteVault: ammFailQuote,
        squadsMultisig: f.multisig,
        squadsProposal,
      }),
      [],
      400_000,
    );

    // (4) swap-driven PASS TWAP verdict: buy Pass repeatedly, >60s apart.
    const trader = f.payer;
    const traderUsdc = await fabricateToken(f, f.usdcMint.publicKey, trader.publicKey, 600_000_000n);
    const traderPassQuote = await fabricateToken(f, quoteV.passMint, trader.publicKey, 0n);
    const traderFailQuote = await fabricateToken(f, quoteV.failMint, trader.publicKey, 0n);
    const traderPassBase = await fabricateToken(f, baseV.passMint, trader.publicKey, 0n);
    await sendIx(
      f,
      await futarchy.splitTokens({
        question,
        vault: quoteV.vault,
        vaultUnderlying: quoteV.underlying,
        authority: trader.publicKey,
        userUnderlying: traderUsdc,
        conditionalMints: [quoteV.failMint, quoteV.passMint],
        userConditionalAccounts: [traderFailQuote, traderPassQuote],
        amount: 500_000_000n,
      }),
      [],
      400_000,
    );

    const enqueued = readI64(await fetchAccount(f, futProposal), 44); // Proposal.timestamp_enqueued @44
    const buyPass = async (amount: bigint) =>
      sendIx(
        f,
        await futarchy.conditionalSwap({
          dao: f.dao,
          ammBaseVault,
          ammQuoteVault,
          proposal: futProposal,
          ammPassBaseVault: ammPassBase,
          ammPassQuoteVault: ammPassQuote,
          ammFailBaseVault: ammFailBase,
          ammFailQuoteVault: ammFailQuote,
          trader: trader.publicKey,
          userInputAccount: traderPassQuote,
          userOutputAccount: traderPassBase,
          baseVault: baseV.vault,
          baseVaultUnderlying: baseV.underlying,
          quoteVault: quoteV.vault,
          quoteVaultUnderlying: quoteV.underlying,
          passBaseMint: baseV.passMint,
          failBaseMint: baseV.failMint,
          passQuoteMint: quoteV.passMint,
          failQuoteMint: quoteV.failMint,
          question,
          market: futarchy.Market.Pass,
          swapType: futarchy.SwapType.Buy,
          inputAmount: amount,
          minOutputAmount: 0n,
        }),
        [],
        1_400_000,
      );
    for (let i = 0; i < 4; i++) {
      await buyPass(80_000_000n);
      await f.harness.advanceToUnix((await f.harness.clockUnixTimestamp()) + 75n);
    }
    await f.harness.advanceToUnix(enqueued + 86_400n + 300n);
    await buyPass(20_000_000n);

    // (5) finalize_proposal → Passed (CPIs Squads proposal_approve).
    await sendIx(
      f,
      await futarchy.finalizeProposal({
        proposal: futProposal,
        dao: f.dao,
        question,
        squadsProposal,
        squadsMultisig: f.multisig,
        ammPassBaseVault: ammPassBase,
        ammPassQuoteVault: ammPassQuote,
        ammFailBaseVault: ammFailBase,
        ammFailQuoteVault: ammFailQuote,
        ammBaseVault,
        ammQuoteVault,
        quoteVault: quoteV.vault,
        quoteVaultUnderlying: quoteV.underlying,
        passQuoteMint: quoteV.passMint,
        failQuoteMint: quoteV.failMint,
        passBaseMint: baseV.passMint,
        failBaseMint: baseV.failMint,
        baseVault: baseV.vault,
        baseVaultUnderlying: baseV.underlying,
      }),
      [],
      1_400_000,
    );
    const finalProposal = await fetchAccount(f, futProposal);
    expect(finalProposal[52]).toBe(2); // Proposal.state @52 == Passed

    // --- EXECUTE: the DAO's Squads vault signs the Meteora claim ----------------
    const feeBBefore = await tokenBalance(f, daoFeeB);
    const feeABefore = await tokenBalance(f, daoFeeA);

    // ADMIN-FREE assertion #3: no MetaDAO admin/vault in the execute account list.
    const execIx = await futarchy.vaultTransactionExecute({
      multisig: f.multisig,
      transactionIndex: txIndex,
      member: permissionless.publicKey,
      remainingAccounts,
    });
    assertNoMetaDao(execIx.keys.map((k) => k.pubkey));

    await sendIx(f, execIx, [permissionless], 1_400_000);

    // --- HEADLINE: the DAO's OWN ATA received the accrued Meteora fee -----------
    const feeBAfter = await tokenBalance(f, daoFeeB);
    const feeAAfter = await tokenBalance(f, daoFeeA);
    const swept = feeBAfter - feeBBefore;
    expect(swept).toBeGreaterThan(0n); // NONZERO quote-side fee to the DAO treasury
    // The vault position's pending fees cleared (a genuine, non-no-op sweep).
    const after = meteora.decodePosition(await fetchAccount(f, m.vaultPos));
    expect(after.feeBPending).toBe(0n);
    expect(after.feeAPending).toBe(0n);
    // token-A (base) side matches whatever (0) the position had.
    expect(feeAAfter - feeABefore).toBe(0n);
  }, 400_000);
});

// ---------------------------------------------------------------------------
// shared Meteora state (set by the ownership test, read by the claim test)
// ---------------------------------------------------------------------------
interface MeteoraState {
  poolAddr: Address;
  tokenAVault: Address;
  tokenBVault: Address;
  vaultPos: Address;
  vaultPosNftMint: Address;
  vaultPosNftAccount: Address;
}
let m: MeteoraState;

// ---------------------------------------------------------------------------
// no-MetaDAO-admin/vault guard
// ---------------------------------------------------------------------------
function assertNoMetaDao(keys: Address[]): void {
  const banned = new Set([
    futarchy.METADAO_ADMIN.toString(), // tSTp6B6k…
    futarchy.METADAO_MULTISIG_VAULT.toString(), // 6awyHMsh…
  ]);
  for (const k of keys) {
    expect(banned.has(k.toString()), `MetaDAO admin/vault ${k} must not appear in the DAO-owned claim flow`).toBe(
      false,
    );
  }
}

// ---------------------------------------------------------------------------
// Squads compact TransactionMessage compiler (generic, from a web3 ix)
// ---------------------------------------------------------------------------
interface SquadsMessage {
  message: Uint8Array;
  /** account_keys in message order, as vault_transaction_execute remaining accounts
   * (writability mirrors the message; NONE marked signer — Squads signs the vault). */
  remainingAccounts: AccountMeta[];
}

/**
 * Compile a single web3 `TransactionInstruction` into Squads v4's compact
 * `TransactionMessage`. `vaultSigner` is the DAO vault PDA that Squads
 * `invoke_signed`s (it must be the message's sole signer, readonly). Keys are
 * deduped and ordered [w-signers, ro-signers, w-non-signers, ro-non-signers] per
 * the Squads message format (see NOTES.md "G3 ADDENDUM").
 */
function compileSquadsMessage(ix: TransactionInstruction, vaultSigner: Address): SquadsMessage {
  interface Role {
    pubkey: Address;
    isSigner: boolean;
    isWritable: boolean;
  }
  const roles = new Map<string, Role>();
  const note = (pubkey: Address, isSigner: boolean, isWritable: boolean) => {
    const k = pubkey.toString();
    const prev = roles.get(k);
    if (prev) {
      prev.isSigner ||= isSigner;
      prev.isWritable ||= isWritable;
    } else {
      roles.set(k, { pubkey, isSigner, isWritable });
    }
  };
  for (const meta of ix.keys) note(meta.pubkey, meta.isSigner, meta.isWritable);
  note(ix.programId, false, false); // the inner program must be an account_key
  // the vault is the message signer (Squads signs for it); it is readonly here.
  note(vaultSigner, true, false);

  const all = [...roles.values()];
  const rank = (r: Role) =>
    r.isSigner && r.isWritable ? 0 : r.isSigner ? 1 : r.isWritable ? 2 : 3;
  all.sort((a, b) => rank(a) - rank(b));

  const numSigners = all.filter((r) => r.isSigner).length;
  const numWritableSigners = all.filter((r) => r.isSigner && r.isWritable).length;
  const numWritableNonSigners = all.filter((r) => !r.isSigner && r.isWritable).length;

  const indexOf = (pk: Address) => all.findIndex((r) => r.pubkey.toString() === pk.toString());
  const compiled = {
    programIdIndex: indexOf(ix.programId),
    accountIndexes: ix.keys.map((meta) => indexOf(meta.pubkey)),
    data: new Uint8Array(ix.data),
  };

  const out: number[] = [numSigners, numWritableSigners, numWritableNonSigners];
  out.push(all.length & 0xff);
  for (const r of all) out.push(...r.pubkey.toBytes());
  out.push(1); // one instruction
  out.push(compiled.programIdIndex & 0xff);
  out.push(compiled.accountIndexes.length & 0xff);
  out.push(...compiled.accountIndexes.map((i) => i & 0xff));
  out.push(compiled.data.length & 0xff, (compiled.data.length >> 8) & 0xff); // u16 LE
  out.push(...compiled.data);
  out.push(0); // address_table_lookups: empty

  const remainingAccounts: AccountMeta[] = all.map((r) => ({
    pubkey: r.pubkey,
    isSigner: false,
    isWritable: r.isWritable,
  }));
  return { message: Uint8Array.from(out), remainingAccounts };
}

// ---------------------------------------------------------------------------
// helpers (mirrors G3 + M2)
// ---------------------------------------------------------------------------

async function sendIx(
  f: Fixture,
  ix: TransactionInstruction,
  signers: Keypair[] = [],
  computeUnits?: number,
): Promise<void> {
  const conn = f.harness.connection;
  const tx = new Transaction();
  tx.feePayer = f.payer.publicKey;
  tx.recentBlockhash = (await conn.getLatestBlockhash()).blockhash;
  if (computeUnits) tx.add(ComputeBudgetProgram.setComputeUnitLimit({ units: computeUnits }));
  tx.add(ix);
  await tx.sign(f.payer, ...signers);
  const sig = await conn.sendRawTransaction(await tx.serialize(), { skipPreflight: false });
  await f.harness.confirmSignature(sig);
}

async function fetchAccount(f: Fixture, address: Address, timeoutMs = 20_000): Promise<Uint8Array> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const info = await f.harness.connection.getAccountInfo(address);
    if (info && info.data.length > 0) return info.data;
    await new Promise((r) => setTimeout(r, 150));
  }
  throw new Error(`account ${address} did not appear within ${timeoutMs}ms`);
}

async function tokenBalance(f: Fixture, address: Address): Promise<bigint> {
  return tokenAccountAmount(await fetchAccount(f, address));
}

function readI64(data: Uint8Array, off: number): bigint {
  return new DataView(data.buffer, data.byteOffset, data.length).getBigInt64(off, true);
}

async function ata(owner: Address, mint: Address): Promise<Address> {
  return (
    await Address.findProgramAddress([owner.toBytes(), TOKEN_PROGRAM_ID.toBytes(), mint.toBytes()], ATA_PROGRAM_ID)
  )[0];
}

/** Materialise an SPL token account (fresh keypair address) with `amount`. */
async function fabricateToken(f: Fixture, mint: Address, owner: Address, amount: bigint): Promise<Address> {
  const acct = await Keypair.generate();
  await fabricateTokenAt(f, acct.publicKey, mint, owner, amount);
  return acct.publicKey;
}

/** Materialise an SPL token account at a SPECIFIC address (e.g. an ATA). */
async function fabricateTokenAt(
  f: Fixture,
  address: Address,
  mint: Address,
  owner: Address,
  amount: bigint,
): Promise<void> {
  await f.harness.setAccount(address.toString(), {
    lamports: 5_000_000,
    owner: TOKEN_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(tokenAccountBytes(mint.toBytes(), owner.toBytes(), amount)),
  });
}

interface CondVault {
  vault: Address;
  underlying: Address;
  passMint: Address; // conditional_token_mints[1]
  failMint: Address; // conditional_token_mints[0]
}

/** Real conditional_vault init (vault + 2 conditional mints) for `underlyingMint`. */
async function condVault(f: Fixture, question: Address, underlyingMint: Address): Promise<CondVault> {
  const vault = (await futarchy.pda.conditionalVault(question, underlyingMint)).address;
  const failMint = (await futarchy.pda.conditionalTokenMint(vault, 0)).address;
  const passMint = (await futarchy.pda.conditionalTokenMint(vault, 1)).address;
  const underlying = await ata(vault, underlyingMint);
  await sendIx(
    f,
    await futarchy.initializeConditionalVault({ question, underlyingMint, payer: f.payer.publicKey, numOutcomes: 2 }),
    [],
    400_000,
  );
  return { vault, underlying, passMint, failMint };
}

/** Fetch an account's raw data straight from mainnet (NOT the fork). */
async function fetchMainnetAccount(address: Address): Promise<Uint8Array> {
  const res = await fetch(MAINNET_RPC, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "getAccountInfo",
      params: [address.toString(), { encoding: "base64" }],
    }),
  });
  const json = (await res.json()) as {
    result?: { value?: { data: [string, string] } | null };
    error?: { message: string };
  };
  if (json.error) throw new Error(`mainnet getAccountInfo failed: ${json.error.message}`);
  const b64 = json.result?.value?.data?.[0];
  if (!b64) throw new Error(`mainnet account ${address} not found`);
  return new Uint8Array(Buffer.from(b64, "base64"));
}

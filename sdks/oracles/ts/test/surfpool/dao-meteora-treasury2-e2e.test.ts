/**
 * D1 (part 2) — DAO-OWNED, ADMIN-FREE METEORA TREASURY-FEE CLAIM: the GOVERNANCE
 * CLAIM arm (GATED, FORKED MAINNET). Split out of `dao-meteora-treasury-e2e.
 * test.ts` so no single file exceeds ~400 lines; the shared fixture bootstrap +
 * vault-owned Meteora position setup are reused verbatim via `startForkFixture`
 * / `bootstrapFlow` / `positionFlow` from the shared harness/flow modules.
 *
 * GOVERNANCE CLAIM — stage the cp-amm `claim_position_fee` (owner == the vault,
 * recipients == the DAO's OWN vault-owned ATAs) as a Squads `vault_transaction_
 * create` → `proposal_create`, then run a REAL futarchy proposal to a PASS TWAP
 * verdict (G3's machinery) so `finalize_proposal` CPI-approves the Squads
 * proposal, then `vault_transaction_execute` (member = the public permissionless
 * member) `invoke_signed`s the claim as the vault.
 *
 * ASSERT — the DAO's ATA received the accrued fee (NONZERO delta), the vault
 * position's `fee_b_pending` cleared to 0, and NO MetaDAO admin/vault appears in
 * ANY account of the claim / staged message / execute remaining-accounts.
 *
 * GATING: `KASSANDRA_E2E=1`; skips (not fails) when surfpool/.so absent. Forks
 * mainnet → needs network + is slower.
 */
import { Keypair } from "@solana/web3.js";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { meteora } from "../../src/index.js";
import * as futarchy from "../../src/futarchy/index.js";

import type { Fixture, MeteoraState } from "./dao-meteora-treasury-harness.js";
import {
  ENABLED,
  PERMISSIONLESS_SECRET,
  assertNoMetaDao,
  ata,
  compileSquadsMessage,
  condVault,
  fabricateToken,
  fabricateTokenAt,
  fetchAccount,
  readI64,
  sendIx,
  tokenBalance,
} from "./dao-meteora-treasury-harness.js";
import { bootstrapFlow, positionFlow, startForkFixture } from "./dao-meteora-treasury-flow.js";

describe.skipIf(!ENABLED)("surfpool DAO-owned admin-free Meteora treasury-fee claim on FORKED mainnet (D1 governance claim)", () => {
  let f: Fixture;
  let m: MeteoraState;

  beforeAll(async () => {
    f = await startForkFixture(8925);
    // Same bootstrap + vault-owned-position setup as the part-1 suite (reused so
    // the claim arm has a DAO + a funded, fee-accruing DAO-owned cp-amm position).
    await bootstrapFlow(f);
    m = await positionFlow(f);
  }, 900_000);

  afterAll(async () => {
    await f?.harness.teardown();
  });

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

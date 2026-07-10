/**
 * T-G3 surfpool FUTARCHY GOVERNANCE E2E — arm 2 (GATED, FORKED MetaDAO). The
 * headline verdict→execute loop, split out of `futarchy-governance-e2e.test.ts`:
 * STAGE the Kassandra `set_config` (+ a second `resolve_deadend` CPI) as a Squads
 * VaultTransaction, open the futarchy PROPOSAL, LAUNCH conditional AMMs, crank a
 * FULLY-REAL TWAP VERDICT (pass > fail), finalize to `Passed`, then EXECUTE the
 * Squads `vault_transaction_execute` so the futarchy verdict drives the on-chain
 * `set_config` (and `resolve_deadend`) via the DAO vault.
 *
 * Shares the surfpool boot + protocol init (`setupFixture`) and the real
 * `initialize_dao` + Squads multisig + G1 `set_governance` handoff (`bootstrapDao`)
 * with arm 1 via `./futarchy-governance-flow.js`.
 *
 * GATING: `KASSANDRA_E2E=1`; skips (not fails) when surfpool/.so absent.
 */
import { Keypair } from "@solana/web3.js";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { decodeProtocol } from "../../src/accounts/index.js";
import { KASSANDRA_PROGRAM_ID, Phase } from "../../src/constants.js";
import { resolveDeadend, setConfig, type SetConfigParams } from "../../src/instructions/index.js";
import * as pda from "../../src/pda.js";
import * as futarchy from "../../src/futarchy/index.js";

import { surfpoolReady } from "./harness.js";
import {
  type Fixture,
  PERMISSIONLESS_SECRET,
  SENTINEL_SUPPLY_CAP,
  ata,
  buildSquadsMessage,
  condVault,
  fabricateDeadendOracle,
  fabricateToken,
  fetchAccount,
  paramsFromProtocol,
  readI64,
  readKassPrice,
  ro,
  sendIx,
  w,
} from "./futarchy-governance-harness.js";
import { bootstrapDao, setupFixture } from "./futarchy-governance-flow.js";

const ENABLED = process.env.KASSANDRA_E2E === "1" && surfpoolReady();

describe.skipIf(!ENABLED)("surfpool futarchy verdict → set_config via Squads on FORKED MetaDAO (G3)", () => {
  let f: Fixture;

  beforeAll(async () => {
    f = await setupFixture(8922);
    await bootstrapDao(f);
  }, 300_000);

  afterAll(async () => {
    await f?.harness.teardown();
  });

  it("STAGE→PROPOSAL→LAUNCH→TWAP-VERDICT→EXECUTE: futarchy verdict drives set_config via Squads", async () => {
    const protocolPda = (await pda.protocol()).address;

    // --- (1) seed the embedded spot AMM via real provide_liquidity ----------
    // Fabricate funded LP token accounts (the mint authority is Kassandra's, so
    // we materialise balances directly — same pattern as the T4 challenge test).
    const QUOTE_LIQ = 1_000_000_000n; // 1000 USDC (6dp) raw
    const BASE_LIQ = 1_000_000_000n; // 1 KASS (9dp) raw → spot price = 1e12 (PRICE_SCALE)
    const lpQuote = await fabricateToken(f, f.usdcMint.publicKey, f.payer.publicKey, QUOTE_LIQ);
    const lpBase = await fabricateToken(f, f.kassMint.publicKey, f.payer.publicKey, BASE_LIQ);
    const ammBaseVault = await ata(f.dao, f.kassMint.publicKey);
    const ammQuoteVault = await ata(f.dao, f.usdcMint.publicKey);

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

    // --- (2) stage the Kassandra set_config as a Squads VaultTransaction -----
    const permissionless = await Keypair.fromSecretKey(PERMISSIONLESS_SECRET);
    const current = decodeProtocol(await fetchAccount(f, protocolPda));
    const newParams: SetConfigParams = { ...paramsFromProtocol(current), totalSupplyCap: SENTINEL_SUPPLY_CAP };
    const setConfigIx = await setConfig({ authority: f.vault, params: newParams });

    // Second governed action (SAME proposal, second inner CPI): resolve a
    // fabricated dead-ended oracle. `load_oracle` only checks owner+size+type, so
    // we materialise a 392-byte Kassandra-owned Oracle in Phase::InvalidDeadend.
    const DEADEND_OPTION = 1;
    const deadendOracle = await fabricateDeadendOracle(f, 2);
    const resolveIx = await resolveDeadend({ oracle: deadendOracle, authority: f.vault, option: DEADEND_OPTION });

    const txIndex = 1n;
    // Squads compact TransactionMessage wrapping BOTH inner CPIs. The vault PDA
    // is a readonly signer (the program signs for it on execute); protocol +
    // oracle are writable non-signers; the Kassandra program is the readonly
    // non-signer referenced by program_id_index.
    //   account_keys = [vault(0,ro-signer), protocol(1,w), oracle(2,w), program(3,ro)]
    const message = buildSquadsMessage({
      accountKeys: [f.vault, protocolPda, deadendOracle, KASSANDRA_PROGRAM_ID],
      numSigners: 1,
      numWritableSigners: 0,
      numWritableNonSigners: 2,
      instructions: [
        // set_config: accounts [protocol, dao_authority]
        { programIdIndex: 3, accountIndexes: [1, 0], data: setConfigIx.data },
        // resolve_deadend: accounts [protocol, oracle, dao_authority]
        { programIdIndex: 3, accountIndexes: [1, 2, 0], data: resolveIx.data },
      ],
    });

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

    // --- (3) conditional question (oracle == futarchy Proposal PDA) + vaults -
    const futProposal = (await futarchy.pda.proposal(squadsProposal)).address;
    const questionId = new Uint8Array(32).fill(0x33);
    const question = (await futarchy.pda.question(questionId, futProposal, 2)).address;
    await sendIx(f, await futarchy.initializeQuestion({ questionId, oracle: futProposal, numOutcomes: 2, payer: f.payer.publicKey }), [], 400_000);

    const baseV = await condVault(f, question, f.kassMint.publicKey);
    const quoteV = await condVault(f, question, f.usdcMint.publicKey);

    // --- (4) initialize_proposal + launch_proposal --------------------------
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

    // --- (5) FULLY-REAL TWAP verdict: crank pass > fail ----------------------
    // Trader splits USDC → pass/fail conditional QUOTE tokens, then Buys Pass
    // repeatedly (raising pass quote/base → pass price → pass TWAP). The oracle
    // updates at most once / 60s, so we surfnet_timeTravel +60s between swaps.
    const trader = f.payer;
    const traderUsdc = await fabricateToken(f, f.usdcMint.publicKey, trader.publicKey, 600_000_000n);
    const traderPassQuote = await fabricateToken(f, quoteV.passMint, trader.publicKey, 0n);
    const traderFailQuote = await fabricateToken(f, quoteV.failMint, trader.publicKey, 0n);
    const traderPassBase = await fabricateToken(f, baseV.passMint, trader.publicKey, 0n);

    // Split 500 USDC into pass/fail conditional quote tokens.
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

    const proposalDataBefore = await fetchAccount(f, futProposal);
    const enqueued = readI64(proposalDataBefore, 44); // Proposal.timestamp_enqueued @44

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

    // A few pump swaps spaced >60s apart so the oracle records a rising pass obs.
    for (let i = 0; i < 4; i++) {
      await buyPass(80_000_000n);
      await f.harness.advanceToUnix((await f.harness.clockUnixTimestamp()) + 75n);
    }
    // Jump past enqueue + duration (1 day) so the final swap stamps the oracle's
    // last_updated beyond the MarketsTooYoung / ProposalTooYoung windows.
    await f.harness.advanceToUnix(enqueued + 86_400n + 300n);
    await buyPass(20_000_000n);

    // --- (6) finalize_proposal → assert Passed + Squads proposal approved ----
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
    // Proposal.state tag @52: 2 == Passed.
    const finalProposal = await fetchAccount(f, futProposal);
    expect(finalProposal[52]).toBe(2);

    // --- (7) vault_transaction_execute → set_config applied via the vault ----
    await sendIx(
      f,
      await futarchy.vaultTransactionExecute({
        multisig: f.multisig,
        transactionIndex: txIndex,
        member: permissionless.publicKey,
        // message.account_keys order: [vault, protocol, oracle, kassandra program].
        remainingAccounts: [
          ro(f.vault),
          w(protocolPda),
          w(deadendOracle),
          ro(KASSANDRA_PROGRAM_ID),
        ],
      }),
      [permissionless],
      400_000,
    );

    // --- HEADLINE: the governable param changed to the sentinel on-chain -----
    const after = decodeProtocol(await fetchAccount(f, protocolPda));
    expect(after.totalSupplyCap).toBe(SENTINEL_SUPPLY_CAP);

    // --- SECOND ARM: the dead-ended oracle was governance-resolved -----------
    const oracleAfter = await fetchAccount(f, deadendOracle);
    expect(oracleAfter[161]).toBe(Phase.Resolved); // 7
    expect(oracleAfter[197]).toBe(DEADEND_OPTION); // resolved_option

    // --- LIVE kass_price: read the futarchy spot TWAP from the REAL Dao ------
    const twap = await readKassPrice(f, f.dao);
    expect(twap).toBeGreaterThan(0n);
  }, 300_000);
});

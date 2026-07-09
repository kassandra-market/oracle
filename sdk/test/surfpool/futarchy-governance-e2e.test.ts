/**
 * T-G3 surfpool FULL FUTARCHY GOVERNANCE E2E (GATED, FORKED MetaDAO) — the
 * headline loop: a real futarchy proposal whose pass/fail TWAP verdict drives a
 * real Squads `vault_transaction_execute` that applies a Kassandra `set_config`
 * on-chain.
 *
 * Boots surfpool FORKING MAINNET so MetaDAO's deployed programs execute over
 * RPC: futarchy v0.6 `FUTARELBf…`, conditional_vault `VLTX1ish…`, Squads v4
 * `SQDS4ep6…`.
 *
 * Arms (each only asserts what GENUINELY happens on the fork):
 *   1. BOOTSTRAP — real `initialize_dao` (creates the Dao + the Squads multisig
 *      with create_key==Dao + vault atomically; treasury fetched live) → the
 *      G1-hardened `set_governance(kass_dao=Dao, dao_authority=vault)`. Asserts
 *      `governanceSet==1`, `daoAuthority==vault`, `kassDao==dao` on-chain.
 *   2. STAGE + PROPOSAL + LAUNCH + VERDICT + EXECUTE — see below.
 *
 * GATING: `KASSANDRA_E2E=1`; skips (not fails) when surfpool/.so absent.
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

import { decodeProtocol, type Protocol } from "../../src/accounts/index.js";
import { TOKEN_PROGRAM_ID } from "../../src/constants.js";
import {
  initProtocol,
  kassPrice,
  resolveDeadend,
  setConfig,
  setGovernance,
  type SetConfigParams,
} from "../../src/instructions/index.js";
import { KASSANDRA_PROGRAM_ID, Phase } from "../../src/constants.js";
import * as pda from "../../src/pda.js";
import * as futarchy from "../../src/futarchy/index.js";

import { SurfpoolHarness, mintBytes, surfpoolReady, toHex, tokenAccountBytes } from "./harness.js";

const ENABLED = process.env.KASSANDRA_E2E === "1" && surfpoolReady();

const ATA_PROGRAM_ID = new Address("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
const SYSTEM_PROGRAM_ID = new Address("11111111111111111111111111111111");

/**
 * MetaDAO's PUBLIC "permissionless" proposer keypair (futarchy
 * `sdk/permissionless-account.json` → EP3SoC2…), a fixed multisig member with
 * Initiate|Execute. Its secret is published by design so anyone can stage +
 * execute futarchy-DAO Squads transactions. Used here to create + execute the
 * VaultTransaction (the Dao member can only Vote/Execute, not Initiate).
 */
const PERMISSIONLESS_SECRET = Uint8Array.from([
  249, 158, 188, 171, 243, 143, 1, 48, 87, 243, 209, 153, 144, 106, 23, 88, 161, 209, 65, 217,
  199, 121, 0, 250, 3, 203, 133, 138, 141, 112, 243, 38, 198, 205, 120, 222, 160, 224, 151, 190,
  84, 254, 127, 178, 224, 195, 130, 243, 145, 73, 20, 91, 9, 69, 222, 184, 23, 1, 2, 196, 202,
  206, 153, 192,
]);

/** A sentinel `total_supply_cap` the futarchy verdict must drive on-chain via Squads. */
const SENTINEL_SUPPLY_CAP = 424_242_424_242n;

interface Fixture {
  harness: SurfpoolHarness;
  payer: Keypair;
  kassMint: Keypair;
  usdcMint: Keypair;
  dao: Address;
  multisig: Address;
  vault: Address;
}

describe.skipIf(!ENABLED)("surfpool FULL futarchy governance loop on FORKED MetaDAO (G3)", () => {
  let f: Fixture;

  beforeAll(async () => {
    const harness = await SurfpoolHarness.start({
      port: 8921,
      fork: "mainnet",
      readyTimeoutMs: 60_000,
    });
    const payer = await Keypair.generate();
    await harness.airdrop(payer.publicKey.toString(), 1_000_000_000_000);

    // Real KASS (9dp) + USDC (MUST be 6dp — initialize_dao `mint::decimals = 6`).
    const mintAuth = await pda.mintAuthority();
    const kassMint = await Keypair.generate();
    const usdcMint = await Keypair.generate();
    await harness.setAccount(kassMint.publicKey.toString(), {
      lamports: 1_000_000_000,
      owner: TOKEN_PROGRAM_ID.toString(),
      executable: false,
      data: toHex(mintBytes(mintAuth.address.toBytes(), 0n, 9)),
    });
    await harness.setAccount(usdcMint.publicKey.toString(), {
      lamports: 1_000_000_000,
      owner: TOKEN_PROGRAM_ID.toString(),
      executable: false,
      data: toHex(mintBytes(payer.publicKey.toBytes(), 0n, 6)),
    });

    f = {
      harness,
      payer,
      kassMint,
      usdcMint,
      dao: undefined as unknown as Address,
      multisig: undefined as unknown as Address,
      vault: undefined as unknown as Address,
    };

    await sendIx(
      f,
      await initProtocol({
        admin: payer.publicKey,
        kassMint: kassMint.publicKey,
        usdcMint: usdcMint.publicKey,
      }),
    );
  }, 120_000);

  afterAll(async () => {
    await f?.harness.teardown();
  });

  it("BOOTSTRAP: real initialize_dao + Squads multisig + G1 set_governance handoff", async () => {
    const nonce = 1n;
    // Squads ProgramConfig.treasury is NOT a PDA — fetch it live from the
    // on-chain ProgramConfig account (treasury @ offset 8+32+8 = 48).
    const programConfig = (await futarchy.pda.squadsProgramConfig()).address;
    const pcInfo = await f.harness.connection.getAccountInfo(programConfig);
    expect(pcInfo, "Squads ProgramConfig not on the fork").not.toBeNull();
    const treasury = new Address(pcInfo!.data.slice(48, 80));

    const boot = await futarchy.bootstrapGovernance({
      payer: f.payer.publicKey,
      daoCreator: f.payer.publicKey,
      kassMint: f.kassMint.publicKey,
      usdcMint: f.usdcMint.publicKey,
      squadsProgramConfigTreasury: treasury,
      nonce,
      admin: f.payer.publicKey,
      // TWAP params: observable immediately (start_delay 0); tiny windows so the
      // verdict arm can timeTravel past them.
      twapInitialObservation: 1_000_000_000_000n, // 1.0 (PRICE_SCALE 1e12) quote/base
      twapMaxObservationChangePerUpdate: 1_000_000_000_000n, // > 0 (invariant); allows big moves
      twapStartDelaySeconds: 0,
      // invariant: min_base/min_quote futarchic liquidity must be > 0.
      minQuoteFutarchicLiquidity: 1n,
      minBaseFutarchicLiquidity: 1n,
      baseToStake: 0n,
      passThresholdBps: 0,
      // DAO invariant: seconds_per_proposal >= 86400 (1 day) and >= 2×start_delay.
      secondsPerProposal: 86_400,
    });

    f.dao = boot.dao;
    f.multisig = boot.multisig;
    f.vault = boot.vault;

    // initialize_dao CPIs into Squads (multisig create) + creates ATAs → heavy.
    await sendIx(f, boot.instructions[0], [], 1_400_000);
    await sendIx(f, boot.instructions[1]);

    const protocol = (await pda.protocol()).address;
    const p = decodeProtocol(await fetchAccount(f, protocol));
    expect(p.governanceSet).toBe(true);
    expect(p.daoAuthority.toString()).toBe(f.vault.toString());
    expect(p.kassDao.toString()).toBe(f.dao.toString());

    // Sanity: the Dao really exists, owned by futarchy, with the Dao disc.
    const daoInfo = await f.harness.connection.getAccountInfo(f.dao);
    expect(daoInfo!.owner.toString()).toBe(futarchy.FUTARCHY_ID.toString());
  }, 180_000);

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

// ---------------------------------------------------------------------------
// helpers
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

function w(pubkey: Address, isSigner = false): AccountMeta {
  return { pubkey, isSigner, isWritable: true };
}
function ro(pubkey: Address, isSigner = false): AccountMeta {
  return { pubkey, isSigner, isWritable: false };
}
function readI64(data: Uint8Array, off: number): bigint {
  return new DataView(data.buffer, data.byteOffset, data.length).getBigInt64(off, true);
}

async function ata(owner: Address, mint: Address): Promise<Address> {
  return (
    await Address.findProgramAddress([owner.toBytes(), TOKEN_PROGRAM_ID.toBytes(), mint.toBytes()], ATA_PROGRAM_ID)
  )[0];
}

/** Materialise an SPL token account (owner=`owner`) on `mint` with `amount`. */
async function fabricateToken(f: Fixture, mint: Address, owner: Address, amount: bigint): Promise<Address> {
  const acct = await Keypair.generate();
  await f.harness.setAccount(acct.publicKey.toString(), {
    lamports: 5_000_000,
    owner: TOKEN_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(tokenAccountBytes(mint.toBytes(), owner.toBytes(), amount)),
  });
  return acct.publicKey;
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

/** Materialise a 392-byte Kassandra-owned Oracle in Phase::InvalidDeadend(8). */
async function fabricateDeadendOracle(f: Fixture, optionsCount: number): Promise<Address> {
  const data = new Uint8Array(392);
  data[0] = 1; // AccountType::Oracle
  data[160] = optionsCount; // options_count
  data[161] = 8; // phase = InvalidDeadend
  data[197] = 0xff; // resolved_option sentinel
  const acct = await Keypair.generate();
  await f.harness.setAccount(acct.publicKey.toString(), {
    lamports: 5_000_000,
    owner: KASSANDRA_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(data),
  });
  return acct.publicKey;
}

/** Read the futarchy spot TWAP (u128 LE return data) via a simulated kass_price tx. */
async function readKassPrice(f: Fixture, dao: Address): Promise<bigint> {
  const conn = f.harness.connection;
  const tx = new Transaction();
  tx.feePayer = f.payer.publicKey;
  tx.recentBlockhash = (await conn.getLatestBlockhash()).blockhash;
  tx.add(await kassPrice({ kassDao: dao }));
  await tx.sign(f.payer);
  const b64 = Buffer.from(await tx.serialize()).toString("base64");
  const res = await f.harness.rpc<{
    value: { err: unknown; returnData: { data: [string, string] } | null };
  }>("simulateTransaction", [b64, { encoding: "base64", commitment: "confirmed" }]);
  if (res.value.err) throw new Error(`kass_price sim failed: ${JSON.stringify(res.value.err)}`);
  const rd = res.value.returnData?.data?.[0];
  if (!rd) throw new Error("kass_price returned no data");
  const bytes = Buffer.from(rd, "base64");
  let v = 0n;
  for (let i = bytes.length - 1; i >= 0; i--) v = (v << 8n) | BigInt(bytes[i]);
  return v;
}

function paramsFromProtocol(p: Protocol): SetConfigParams {
  return {
    emissionNum: p.emissionNum,
    emissionDen: p.emissionDen,
    totalSupplyCap: p.totalSupplyCap,
    feeEmaHalflife: p.feeEmaHalflife,
    feePerEmaUnit: p.feePerEmaUnit,
    feeEmaIncrement: p.feeEmaIncrement,
    thresholdNum: p.thresholdNum,
    thresholdDen: p.thresholdDen,
    marketThresholdNum: p.marketThresholdNum,
    marketThresholdDen: p.marketThresholdDen,
    flipSlashNum: p.flipSlashNum,
    flipSlashDen: p.flipSlashDen,
    phaseWindow: p.phaseWindow,
    proposalWindow: p.proposalWindow,
    factVoteSlashNum: p.factVoteSlashNum,
    factVoteSlashDen: p.factVoteSlashDen,
    rewardProposerWeight: p.rewardProposerWeight,
    rewardFactWeight: p.rewardFactWeight,
    challengeFailUsdcFeeNum: p.challengeFailUsdcFeeNum,
    challengeFailUsdcFeeDen: p.challengeFailUsdcFeeDen,
    challengeSuccessKassFeeNum: p.challengeSuccessKassFeeNum,
    challengeSuccessKassFeeDen: p.challengeSuccessKassFeeDen,
    stakeFloorEmaThreshold: p.stakeFloorEmaThreshold,
    stakeFloorEmaCap: p.stakeFloorEmaCap,
    stakeFloorMax: p.stakeFloorMax,
  };
}

interface SquadsCompiledIx {
  programIdIndex: number;
  accountIndexes: number[];
  data: Uint8Array;
}

/**
 * Encode a Squads v4 compact `TransactionMessage` (see the recon spec):
 *   num_signers u8, num_writable_signers u8, num_writable_non_signers u8,
 *   account_keys SmallVec<u8, Pubkey>, instructions SmallVec<u8, CompiledIx>,
 *   address_table_lookups SmallVec<u8, _> (empty here).
 * CompiledIx: program_id_index u8, account_indexes SmallVec<u8,u8>,
 *   data SmallVec<u16, u8> (u16 length prefix — the one wide field).
 */
function buildSquadsMessage(m: {
  accountKeys: Address[];
  numSigners: number;
  numWritableSigners: number;
  numWritableNonSigners: number;
  instructions: SquadsCompiledIx[];
}): Uint8Array {
  const parts: number[] = [m.numSigners, m.numWritableSigners, m.numWritableNonSigners];
  parts.push(m.accountKeys.length & 0xff);
  const out: number[] = [...parts];
  for (const k of m.accountKeys) out.push(...k.toBytes());
  out.push(m.instructions.length & 0xff);
  for (const ix of m.instructions) {
    out.push(ix.programIdIndex & 0xff);
    out.push(ix.accountIndexes.length & 0xff);
    out.push(...ix.accountIndexes.map((i) => i & 0xff));
    out.push(ix.data.length & 0xff, (ix.data.length >> 8) & 0xff); // u16 LE
    out.push(...ix.data);
  }
  out.push(0); // address_table_lookups: empty
  return Uint8Array.from(out);
}

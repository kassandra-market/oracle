/**
 * CU2 GATED FORKED-MAINNET surfpool TRADE / CRANK / SETTLE E2E (`KASSANDRA_E2E=1`).
 *
 * Proves the CU2 app trade action layer end-to-end against a surfpool validator
 * FORKING MAINNET (MetaDAO's DEPLOYED conditional_vault `VLTX…` + amm `AMMyu…`
 * are lazily fetched + EXECUTABLE), in `clock` block-production mode with a fast
 * slot-time (the v0.4 AMM crank is SLOT-based). It reuses the PROVEN RF4 /
 * SDK-challenge market-composition recipe (question + KASS/USDC conditional
 * vaults + two pass/fail v0.4 AMM pools), but drives the SWAP + CRANK through the
 * NEW app builders over the app {@link keypairSender}/{@link sendAndConfirm} seam:
 *
 *   buildSwapIxs      → a BUY on the FAIL pool (quote→base) MOVES the AMM: the
 *                       CU1 `decodeAmmV04` reserves + spot price shift, the mint
 *                       derivation lands on the right pool (a wrong mint fails).
 *   buildCrankTwapIxs → two cranks ≥150 slots apart ACCUMULATE the FAIL TWAP
 *                       oracle (the aggregator + `twapPrice` rise).
 *
 * SCOPE: this file asserts the app SWAP + CRANK builders (via CU1's decoder). The
 * settle path (`buildSettleChallengeIxs`) is proven live separately in RF4's
 * `challenge.e2e.test.ts` (a full swap-driven verdict → settle); the market
 * composition + open/settle plumbing use the raw CPIs the SDK/RF4 tests document
 * and are not re-driven here.
 *
 * Gated: skips (never fails) unless `KASSANDRA_E2E=1` AND surfpool + the `.so`
 * are present.
 */
import {
  Address,
  ComputeBudgetProgram,
  Keypair,
  Transaction,
  TransactionInstruction,
} from "@solana/web3.js";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { TOKEN_PROGRAM_ID, EXTERNAL_PROGRAM_IDS, ammV04 } from "@kassandra-market/oracles";

import {
  SurfpoolHarness,
  mintBytes,
  surfpoolReady,
  toHex,
  tokenAccountBytes,
} from "../../sdks/oracles/ts/test/surfpool/harness.ts";
import { buildCrankTwapIxs, buildSwapIxs, poolMints } from "../src/data/actions/challengeTrade.ts";
import { decodeAmmV04, instantaneousPrice, twapPrice } from "../src/data/ammV04.ts";
import { keypairSender, sendAndConfirm } from "../src/data/send.ts";

const ENABLED = process.env.KASSANDRA_E2E === "1" && surfpoolReady();

const SYSTEM_PROGRAM_ID = new Address("11111111111111111111111111111111");
const ATA_PROGRAM_ID = new Address("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
const VLTX = EXTERNAL_PROGRAM_IDS.conditionalVault;

const INITIALIZE_QUESTION = Uint8Array.from([0xf5, 0x97, 0x6a, 0xbc, 0x58, 0x2c, 0x41, 0xd4]);
const INITIALIZE_CONDITIONAL_VAULT = Uint8Array.from([
  0x25, 0x58, 0xfa, 0xd4, 0x36, 0xda, 0xe3, 0xaf,
]);

const enc = new TextEncoder();
const MAX_PRICE = ((1n << 64n) - 1n) * 1_000_000_000_000n;
const BASE_RESERVE = 100_000_000_000n;
const QUOTE_NEUTRAL = 100_000_000n;

interface Fixture {
  harness: SurfpoolHarness;
  payer: Keypair;
  kassMint: Keypair;
  usdcMint: Keypair;
}

describe.skipIf(!ENABLED)("CU2 trade/crank/settle over FORKED MetaDAO AMMs (app builders)", () => {
  let f: Fixture;

  beforeAll(async () => {
    const harness = await SurfpoolHarness.start({
      port: 8941, // dedicated port (RF4 uses 8940)
      fork: "mainnet",
      blockProductionMode: "clock",
      slotTimeMs: 10,
      readyTimeoutMs: 60_000,
    });
    const payer = await Keypair.generate();
    await harness.airdrop(payer.publicKey.toString(), 1_000_000_000_000);

    const kassMint = await Keypair.generate();
    const usdcMint = await Keypair.generate();
    await harness.setAccount(kassMint.publicKey.toString(), {
      lamports: 1_000_000_000,
      owner: TOKEN_PROGRAM_ID.toString(),
      executable: false,
      data: toHex(mintBytes(payer.publicKey.toBytes(), 10n ** 18n, 9)),
    });
    await harness.setAccount(usdcMint.publicKey.toString(), {
      lamports: 1_000_000_000,
      owner: TOKEN_PROGRAM_ID.toString(),
      executable: false,
      data: toHex(mintBytes(payer.publicKey.toBytes(), 10n ** 18n, 6)),
    });

    f = { harness, payer, kassMint, usdcMint };
  }, 120_000);

  afterAll(async () => {
    await f?.harness.teardown();
  });

  it("app buildSwapIxs moves the FAIL AMM + buildCrankTwapIxs accumulates its TWAP", async () => {
    // ---- compose the market (question + KASS/USDC conditional vaults) ----------
    const resolver = f.payer.publicKey; // any resolver — we only trade the pools here
    const questionId = new Uint8Array(32).fill(0x21);
    const question = await composeQuestion(f, resolver, questionId, 2);
    const kass = await composeVault(f, question, f.kassMint.publicKey);
    const usdc = await composeVault(f, question, f.usdcMint.publicKey);

    // A minimal Market stand-in carrying just the vaults the trade builders derive from.
    const market = { kassVault: kass.vault, usdcVault: usdc.vault } as unknown as Parameters<
      typeof poolMints
    >[0];

    // Sanity: the app-derived pool mints match the composed conditional-token mints.
    const passM = await poolMints(market, "pass");
    const failM = await poolMints(market, "fail");
    expect(passM.base.toString()).toBe(kass.passMint.toString());
    expect(passM.quote.toString()).toBe(usdc.passMint.toString());
    expect(failM.base.toString()).toBe(kass.failMint.toString());
    expect(failM.quote.toString()).toBe(usdc.failMint.toString());

    // ---- build both v0.4 AMM pools (base = conditional-KASS, quote = cond-USDC) -
    const passAmm = await buildPool(f, passM.base, passM.quote, BASE_RESERVE, QUOTE_NEUTRAL);
    const failAmm = await buildPool(f, failM.base, failM.quote, BASE_RESERVE, QUOTE_NEUTRAL);

    // Fund the payer's FAIL conditional-token ATAs so the app swap has quote to spend.
    await fundUserAta(f, failM.base, BASE_RESERVE);
    await fundUserAta(f, failM.quote, QUOTE_NEUTRAL * 2n);

    const before = decodeAmmV04(await fetchAccount(f, failAmm));
    const spotBefore = instantaneousPrice(before);

    // ============ SWAP via the APP builder (a BUY on the FAIL pool) =============
    await advanceSlots(f, 200);
    const swapIxs = await buildSwapIxs({
      connection: f.harness.connection,
      market,
      pool: "fail",
      side: "buy",
      amountIn: 90_000_000n,
      user: f.payer.publicKey,
      minAmountOut: 0n,
    });
    await sendViaApp(f, f.payer, swapIxs, 1_400_000);

    const afterSwap = decodeAmmV04(await fetchAccount(f, failAmm));
    // A BUY (quote→base) pulls base out + pushes quote in: base falls, quote rises.
    expect(afterSwap.baseAmount, "base reserve fell after buy").toBeLessThan(before.baseAmount);
    expect(afterSwap.quoteAmount, "quote reserve rose after buy").toBeGreaterThan(
      before.quoteAmount,
    );
    const spotAfter = instantaneousPrice(afterSwap);
    expect(spotAfter!, "spot price moved up after the buy").toBeGreaterThan(spotBefore!);

    // ============ CRANK via the APP builder (two cranks ≥150 slots apart) =======
    await advanceSlots(f, 300);
    await sendViaApp(f, f.payer, await buildCrankTwapIxs({ market, pool: "fail" }), 400_000);
    await advanceSlots(f, 300);
    await sendViaApp(f, f.payer, await buildCrankTwapIxs({ market, pool: "fail" }), 400_000);

    const afterCrank = decodeAmmV04(await fetchAccount(f, failAmm));
    expect(afterCrank.aggregator, "TWAP aggregator accumulated after cranks").toBeGreaterThan(0n);
    expect(afterCrank.lastUpdatedSlot, "TWAP last-updated advanced").toBeGreaterThan(
      before.lastUpdatedSlot,
    );
    const twap = twapPrice(afterCrank);
    expect(twap, "a real non-zero TWAP formed").not.toBeNull();
    expect(twap!).toBeGreaterThan(0n);

    // The pass pool was untouched (its mint derivation is a different pool).
    void passAmm;
  }, 300_000);
});

// ---------------------------------------------------------------------------
// App-seam sender + market composition (mirrors RF4 / challenge-market-e2e).
// ---------------------------------------------------------------------------

async function sendViaApp(
  f: Fixture,
  signer: Keypair,
  ixs: TransactionInstruction[],
  computeUnits?: number,
): Promise<void> {
  const conn = f.harness.connection;
  const withCu = computeUnits
    ? [ComputeBudgetProgram.setComputeUnitLimit({ units: computeUnits }), ...ixs]
    : ixs;
  await sendAndConfirm(conn, keypairSender(conn, signer), withCu);
}

async function ata(owner: Address, mint: Address): Promise<Address> {
  return (
    await Address.findProgramAddress(
      [owner.toBytes(), TOKEN_PROGRAM_ID.toBytes(), mint.toBytes()],
      ATA_PROGRAM_ID,
    )
  )[0];
}

async function composeQuestion(
  f: Fixture,
  resolver: Address,
  questionId: Uint8Array,
  numOutcomes: number,
): Promise<Address> {
  const [question] = await Address.findProgramAddress(
    [enc.encode("question"), questionId, resolver.toBytes(), Uint8Array.from([numOutcomes])],
    VLTX,
  );
  const [eventAuthority] = await Address.findProgramAddress([enc.encode("__event_authority")], VLTX);
  const data = new Uint8Array(73);
  data.set(INITIALIZE_QUESTION, 0);
  data.set(questionId, 8);
  data.set(resolver.toBytes(), 40);
  data[72] = numOutcomes;
  await sendIx(
    f,
    new TransactionInstruction({
      programId: VLTX,
      keys: [
        { pubkey: question, isSigner: false, isWritable: true },
        { pubkey: f.payer.publicKey, isSigner: true, isWritable: true },
        { pubkey: SYSTEM_PROGRAM_ID, isSigner: false, isWritable: false },
        { pubkey: eventAuthority, isSigner: false, isWritable: false },
        { pubkey: VLTX, isSigner: false, isWritable: false },
      ],
      data,
    }),
    [],
    400_000,
  );
  return question;
}

interface VaultAccounts {
  vault: Address;
  underlying: Address;
  passMint: Address;
  failMint: Address;
}

async function composeVault(
  f: Fixture,
  question: Address,
  underlyingMint: Address,
): Promise<VaultAccounts> {
  const [vault] = await Address.findProgramAddress(
    [enc.encode("conditional_vault"), question.toBytes(), underlyingMint.toBytes()],
    VLTX,
  );
  const [passMint] = await Address.findProgramAddress(
    [enc.encode("conditional_token"), vault.toBytes(), Uint8Array.from([0])],
    VLTX,
  );
  const [failMint] = await Address.findProgramAddress(
    [enc.encode("conditional_token"), vault.toBytes(), Uint8Array.from([1])],
    VLTX,
  );
  const [eventAuthority] = await Address.findProgramAddress([enc.encode("__event_authority")], VLTX);
  const underlying = await ata(vault, underlyingMint);
  await sendIx(
    f,
    new TransactionInstruction({
      programId: VLTX,
      keys: [
        { pubkey: vault, isSigner: false, isWritable: true },
        { pubkey: question, isSigner: false, isWritable: false },
        { pubkey: underlyingMint, isSigner: false, isWritable: false },
        { pubkey: underlying, isSigner: false, isWritable: true },
        { pubkey: f.payer.publicKey, isSigner: true, isWritable: true },
        { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
        { pubkey: ATA_PROGRAM_ID, isSigner: false, isWritable: false },
        { pubkey: SYSTEM_PROGRAM_ID, isSigner: false, isWritable: false },
        { pubkey: eventAuthority, isSigner: false, isWritable: false },
        { pubkey: VLTX, isSigner: false, isWritable: false },
        { pubkey: passMint, isSigner: false, isWritable: true },
        { pubkey: failMint, isSigner: false, isWritable: true },
      ],
      data: INITIALIZE_CONDITIONAL_VAULT,
    }),
    [],
    400_000,
  );
  return { vault, underlying, passMint, failMint };
}

async function setTokenAccountAt(
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

/** Fabricate the payer's ATA for a conditional-token mint with `amount` (so the swap has funds). */
async function fundUserAta(f: Fixture, mint: Address, amount: bigint): Promise<void> {
  await setTokenAccountAt(f, await ata(f.payer.publicKey, mint), mint, f.payer.publicKey, amount);
}

async function advanceSlots(f: Fixture, n: number): Promise<void> {
  const start = await f.harness.currentSlot();
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if ((await f.harness.currentSlot()) >= start + n) return;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`slot did not advance by ${n} within 30s (clock mode not producing blocks?)`);
}

async function buildPool(
  f: Fixture,
  baseMint: Address,
  quoteMint: Address,
  baseReserve: bigint,
  quoteReserve: bigint,
): Promise<Address> {
  const ammAddr = (await ammV04.pda.amm(baseMint, quoteMint)).address;
  const lp = (await ammV04.pda.lpMint(ammAddr)).address;
  const userBase = await ammV04.pda.ata(f.payer.publicKey, baseMint);
  const userQuote = await ammV04.pda.ata(f.payer.publicKey, quoteMint);
  await setTokenAccountAt(f, userBase, baseMint, f.payer.publicKey, baseReserve * 4n);
  await setTokenAccountAt(f, userQuote, quoteMint, f.payer.publicKey, quoteReserve * 4n);

  const initialObs = (quoteReserve * 1_000_000_000_000n) / baseReserve;
  await sendIx(
    f,
    await ammV04.createAmm({
      payer: f.payer.publicKey,
      baseMint,
      quoteMint,
      twapInitialObservation: initialObs,
      twapMaxObservationChangePerUpdate: MAX_PRICE,
      twapStartDelaySlots: 0n,
    }),
    [],
    1_400_000,
  );

  const userLp = await ammV04.pda.ata(f.payer.publicKey, lp);
  await setTokenAccountAt(f, userLp, lp, f.payer.publicKey, 0n);
  await sendIx(
    f,
    await ammV04.addLiquidity({
      payer: f.payer.publicKey,
      baseMint,
      quoteMint,
      quoteAmount: quoteReserve,
      maxBaseAmount: baseReserve,
      minLpTokens: 0n,
    }),
    [],
    1_400_000,
  );
  return ammAddr;
}

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

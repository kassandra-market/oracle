/**
 * RF4 GATED FORKED-MAINNET surfpool CHALLENGE + AI-CLAIM E2E (`KASSANDRA_E2E=1`).
 *
 * Proves the RF4 challenge action layer end-to-end against a surfpool validator
 * FORKING MAINNET (so MetaDAO's DEPLOYED programs — conditional_vault `VLTX…`,
 * amm `AMMyu…`, futarchy `FUTAREL…` — are lazily fetched + EXECUTABLE), in
 * `clock` block-production mode with a fast slot-time (the v0.4 AMM crank
 * rate-limit is SLOT-based). This mirrors the PROVEN SDK recipe
 * (`sdks/oracles/ts/test/surfpool/challenge-market-e2e.test.ts`) wholesale, but drives the
 * THREE app builders through the app's {@link keypairSender}/{@link sendAndConfirm}
 * seam:
 *
 *   buildSubmitAiClaimIxs → each proposer stamps its AI claim in the AiClaim
 *                           phase; the AiClaim PDA appears on-chain (decoded).
 *   buildOpenChallengeIxs → a Market + USDC escrow open against the composed
 *                           MetaDAO market (program-signed split_tokens CPI runs
 *                           on the forked conditional_vault); the Market PDA +
 *                           ai_claim.challenged flip are asserted.
 *   buildSettleFromMarketIxs → after a REAL swap-driven FAIL-pool TWAP clears the
 *                           10% margin (two cranks ≥150 slots apart), the SD1
 *                           derive-from-Market settle builder derives the full
 *                           15-account settle set from the DECODED on-chain Market
 *                           + Oracle (NOT the composed JSON), resolves the question
 *                           FAIL-side and DISQUALIFIES + slashes the proposer; the
 *                           economics are asserted.
 *
 * DRIVEN LIVE: submitAiClaim + openChallenge (RF4 builders) + the SD1 one-click
 * derive-from-Market settle (`buildSettleFromMarketIxs`).
 * The AMM pool build/swap/crank is SETUP via the SDK `ammV04` builders (the crank
 * is not an app builder); the market composition (question + conditional vaults)
 * uses the same raw CPIs the SDK test documents.
 *
 * Gated: skips (never fails) unless `KASSANDRA_E2E=1` AND surfpool + the `.so`
 * are present. The fork needs network (mainnet datasource) + is slower.
 */
import {
  Address,
  ComputeBudgetProgram,
  Keypair,
  Transaction,
  TransactionInstruction,
} from "@solana/web3.js";
import { buildDaoBlob } from "../../sdks/oracles/ts/test/surfpool/futarchy-dao.ts";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import {
  Phase,
  TOKEN_PROGRAM_ID,
  VOTE_APPROVE,
  EXTERNAL_PROGRAM_IDS,
  ammV04,
  futarchy,
  advancePhase,
  createOracle,
  decodeAiClaim,
  decodeMarket,
  decodeOracle,
  decodeProposer,
  finalizeAiClaims,
  finalizeFacts,
  finalizeProposals,
  initProtocol,
  propose,
  setGovernance,
  submitFact,
  voteFact,
} from "@kassandra-market/oracles";
import * as pda from "@kassandra-market/oracles";

import {
  SurfpoolHarness,
  mintBytes,
  surfpoolReady,
  toHex,
  tokenAccountAmount,
  tokenAccountBytes,
} from "../../sdks/oracles/ts/test/surfpool/harness.ts";
import {
  buildOpenChallengeIxs,
  buildSubmitAiClaimIxs,
} from "../src/data/actions/challenge.ts";
import { buildSettleFromMarketIxs } from "../src/data/actions/challengeSettle.ts";
import { keypairSender, sendAndConfirm } from "../src/data/send.ts";

const ENABLED = process.env.KASSANDRA_E2E === "1" && surfpoolReady();

const SYSTEM_PROGRAM_ID = new Address("11111111111111111111111111111111");
const ATA_PROGRAM_ID = new Address("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
const VLTX = EXTERNAL_PROGRAM_IDS.conditionalVault;
const FUTARCHY_ID = EXTERNAL_PROGRAM_IDS.futarchyV06;

const INITIALIZE_QUESTION = Uint8Array.from([0xf5, 0x97, 0x6a, 0xbc, 0x58, 0x2c, 0x41, 0xd4]);
const INITIALIZE_CONDITIONAL_VAULT = Uint8Array.from([0x25, 0x58, 0xfa, 0xd4, 0x36, 0xda, 0xe3, 0xaf]);

const KASS_PRICE_TWAP = 500_000_000n;
const KASS_PRICE_SCALE = 1_000_000_000_000n;

const enc = new TextEncoder();

const BOND = 1_000_000_000n;
const MAX_PRICE = ((1n << 64n) - 1n) * 1_000_000_000_000n;
const BASE_RESERVE = 100_000_000_000n;
const QUOTE_NEUTRAL = 100_000_000n;

interface Fixture {
  harness: SurfpoolHarness;
  payer: Keypair;
  kassMint: Keypair;
  usdcMint: Keypair;
  kassDao: Address;
}

describe.skipIf(!ENABLED)("RF4 challenge/ai-claim action layer over FORKED MetaDAO", () => {
  let f: Fixture;

  beforeAll(async () => {
    // Fork mainnet + clock block-production (fast slot-time) — the v0.4 AMM crank
    // is SLOT-based. Dedicated port (8940) so it never collides with the other
    // gated suites (finalize 8901 / claims 8931 / SDK challenge 8920).
    const harness = await SurfpoolHarness.start({
      port: 8940,
      fork: "mainnet",
      blockProductionMode: "clock",
      slotTimeMs: 10,
      readyTimeoutMs: 60_000,
    });
    const payer = await Keypair.generate();
    await harness.airdrop(payer.publicKey.toString(), 1_000_000_000_000);

    const mintAuth = await pda.mintAuthority();
    const kassMint = await Keypair.generate();
    const usdcMint = await Keypair.generate();
    await harness.setAccount(kassMint.publicKey.toString(), {
      lamports: 1_000_000_000,
      owner: TOKEN_PROGRAM_ID.toString(),
      executable: false,
      data: toHex(mintBytes(mintAuth.address.toBytes(), 10n ** 18n, 9)),
    });
    await harness.setAccount(usdcMint.publicKey.toString(), {
      lamports: 1_000_000_000,
      owner: TOKEN_PROGRAM_ID.toString(),
      executable: false,
      data: toHex(mintBytes(payer.publicKey.toBytes(), 10n ** 18n, 6)),
    });

    const kassDao = (await Keypair.generate()).publicKey;
    await harness.setAccount(kassDao.toString(), {
      lamports: 5_000_000,
      owner: FUTARCHY_ID.toString(),
      executable: false,
      data: toHex(buildDaoBlob(KASS_PRICE_TWAP * 1_000_000n, 1_000_000n, 0n, 0)),
    });

    f = { harness, payer, kassMint, usdcMint, kassDao };

    await sendIx(f, await initProtocol({
      admin: payer.publicKey,
      kassMint: kassMint.publicKey,
      usdcMint: usdcMint.publicKey,
    }));
    const multisig = (await futarchy.pda.squadsMultisig(kassDao)).address;
    const daoAuthority = (await futarchy.pda.squadsVault(multisig, 0)).address;
    await sendIx(f, await setGovernance({ authority: payer.publicKey, daoAuthority, kassDao }));
  }, 120_000);

  afterAll(async () => {
    await f?.harness.teardown();
  });

  it("DISQUALIFY: submitAiClaim → openChallenge → swap-driven TWAP crank → settle (via the app builders)", async () => {
    const nonce = 200n;

    // ---- drive to Challenge; the AiClaims are stamped via the APP builder -----
    const c = await frontDoorToChallenge(f, nonce);
    expect(decodeOracle(await fetchAccount(f, c.oracle)).phase).toBe(Phase.Challenge);

    // Each proposer's AiClaim PDA is live on-chain (submitAiClaim driven live).
    for (const proposer of c.proposerPdas) {
      const aiClaim = (await pda.aiClaim(c.oracle, proposer)).address;
      const decoded = decodeAiClaim(await fetchAccount(f, aiClaim));
      expect(decoded.proposer.toString()).toBe(proposer.toString());
      expect(Array.from(decoded.modelId)).toEqual(Array.from(new Uint8Array(32).fill(0xa1)));
    }

    const market = await composeMarket(f, c.oracle);

    // ---- REAL pass/fail v0.4 AMM pools; FAIL gets a genuine BUY swap ----------
    const passAmm = await buildPool(f, market.kass.passMint, market.usdc.passMint, BASE_RESERVE, QUOTE_NEUTRAL);
    const failAmm = await buildPool(f, market.kass.failMint, market.usdc.failMint, BASE_RESERVE, QUOTE_NEUTRAL);
    await crankPool(f, passAmm);
    await swapBuy(f, market.kass.failMint, market.usdc.failMint, 90_000_000n);
    await crankPool(f, failAmm);
    await crankPool(f, failAmm);

    const passTwap = decodeAmmTwap(await fetchAccount(f, passAmm)).twap;
    const failTwap = decodeAmmTwap(await fetchAccount(f, failAmm)).twap;
    expect(passTwap, "pass TWAP must be a real non-zero observation").toBeGreaterThan(0n);
    expect(failTwap * 10n, "fail*DEN must clear pass*(DEN+NUM)").toBeGreaterThan(passTwap * 11n);

    // ================= openChallenge via the APP builder =======================
    const challenger = await openChallengeViaApp(f, nonce, c, market, passAmm, failAmm);
    const marketPda = (await pda.market(c.aiClaim)).address;

    const m = decodeMarket(await fetchAccount(f, marketPda));
    expect(m.oracle.toString()).toBe(c.oracle.toString());
    expect(m.proposer.toString()).toBe(c.proposer.toString());
    expect(m.challenger.toString()).toBe(challenger.publicKey.toString());
    expect(m.question.toString()).toBe(market.question.toString());
    expect(m.kassVault.toString()).toBe(market.kass.vault.toString());
    expect(decodeAiClaim(await fetchAccount(f, c.aiClaim)).challenged).toBe(true);
    expect(decodeOracle(await fetchAccount(f, c.oracle)).openChallengeCount).toBe(1);

    const escrow = (await pda.challengeUsdcVault(marketPda)).address;
    const requiredUsdc = (BOND * KASS_PRICE_TWAP) / KASS_PRICE_SCALE;
    expect(await tokenBalance(f, escrow)).toBe(requiredUsdc);
    expect(m.challengerUsdc).toBe(requiredUsdc);
    expect(await tokenBalance(f, market.oraclePassKass)).toBe(BOND);
    expect(await tokenBalance(f, market.oracleFailKass)).toBe(BOND);

    // ================= settleChallenge via the APP builder =====================
    const oBefore = decodeOracle(await fetchAccount(f, c.oracle));
    const stakeVault = (await pda.stakeVault(c.oracle)).address;
    const stakeBefore = await tokenBalance(f, stakeVault);

    const payouts = await settleChallengeViaApp(f, nonce, c, market, marketPda, challenger, passAmm, failAmm);

    const escrowAmt = (BOND * KASS_PRICE_TWAP) / KASS_PRICE_SCALE; // 500_000
    const kassFee = BOND / 100n;
    expect(questionResolution(await fetchAccount(f, market.question))).toEqual([0, 1]);
    expect(decodeMarket(await fetchAccount(f, marketPda)).settled).toBe(true);
    expect(decodeOracle(await fetchAccount(f, c.oracle)).openChallengeCount).toBe(0);
    const p = decodeProposer(await fetchAccount(f, c.proposer));
    expect(p.disqualified).toBe(true);
    expect(p.slashed).toBe(true);
    expect(p.slashedAmount).toBe(BOND - kassFee);
    const oAfter = decodeOracle(await fetchAccount(f, c.oracle));
    expect(oAfter.survivingCount).toBe(oBefore.survivingCount - 1);
    expect(oAfter.bondPool).toBe(oBefore.bondPool + (BOND - kassFee));
    expect(await tokenBalance(f, payouts.challengerKass)).toBe(kassFee);
    expect(await tokenBalance(f, stakeVault)).toBe(stakeBefore + (BOND - kassFee));
    expect(await tokenBalance(f, payouts.challengerUsdcDest)).toBe(escrowAmt);
    expect(await tokenBalance(f, payouts.proposerUsdc)).toBe(0n);
    expect(await tokenBalance(f, payouts.escrowVault)).toBe(0n);
  }, 300_000);
});

// ---------------------------------------------------------------------------
// App-seam senders: build via the app builders, send via keypairSender/sendAndConfirm.
// ---------------------------------------------------------------------------

/** Send app-built ixs through the app seam (optionally prepending a CU budget). */
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

interface Challenged {
  oracle: Address;
  proposer: Address;
  proposerAuthority: Address;
  aiClaim: Address;
  proposerPdas: Address[];
  authorities: Keypair[];
}

async function frontDoorToChallenge(f: Fixture, nonce: bigint): Promise<Challenged> {
  const oracle = (await pda.oracle(nonce)).address;
  const aiOption = 0;

  await createOracleReal(f, nonce, 2);
  await openProposals(f, oracle);

  const authorities: Keypair[] = [];
  const proposerPdas: Address[] = [];
  for (const option of [0, 1]) {
    const { authority, proposer } = await proposeRealWithAuthority(f, oracle, option, BOND);
    authorities.push(authority);
    proposerPdas.push(proposer);
  }

  await advancePastPhaseEnd(f, oracle);
  await sendIx(f, await finalizeProposals({ oracle, proposers: proposerPdas }));

  const contentHash = new Uint8Array(32).fill(0x07);
  const submitter = await Keypair.generate();
  await f.harness.airdrop(submitter.publicKey.toString(), 2_000_000_000);
  const submitterKass = await fundKass(f, submitter.publicKey, 1_000_000n);
  await sendIx(
    f,
    await submitFact({ oracle, submitter: submitter.publicKey, submitterKass, contentHash, stake: 100n, uri: "ipfs://fact" }),
    [submitter],
  );
  const fact = (await pda.fact(oracle, contentHash)).address;

  await advancePastPhaseEnd(f, oracle);
  await sendIx(f, await advancePhase({ oracle }));

  const voter = await Keypair.generate();
  await f.harness.airdrop(voter.publicKey.toString(), 2_000_000_000);
  const voterKass = await fundKass(f, voter.publicKey, 10n * BOND);
  await sendIx(
    f,
    await voteFact({ oracle, fact, voter: voter.publicKey, voterKass, kind: VOTE_APPROVE, stake: 2n * BOND }),
    [voter],
  );

  await advancePastPhaseEnd(f, oracle);
  await sendIx(f, await finalizeFacts({ nonce, kassMint: f.kassMint.publicKey, tail: [fact] }));

  // --- submit_ai_claim via the APP builder (each proposer authority signs) ---
  for (let i = 0; i < proposerPdas.length; i++) {
    const ixs = await buildSubmitAiClaimIxs({
      oracle,
      proposer: proposerPdas[i],
      submitter: authorities[i].publicKey,
      modelId: new Uint8Array(32).fill(0xa1),
      paramsHash: new Uint8Array(32).fill(0xb2),
      ioHash: new Uint8Array(32).fill(0xc3),
      option: aiOption,
      optionsCount: 2,
    });
    await sendViaApp(f, authorities[i], ixs);
  }

  await advancePastPhaseEnd(f, oracle);
  await sendIx(f, await finalizeAiClaims({ oracle, proposers: proposerPdas }));

  const proposer = proposerPdas[0];
  const aiClaim = (await pda.aiClaim(oracle, proposer)).address;
  return { oracle, proposer, proposerAuthority: authorities[0].publicKey, aiClaim, proposerPdas, authorities };
}

interface VaultAccounts {
  vault: Address;
  underlying: Address;
  passMint: Address;
  failMint: Address;
}
interface MarketComposition {
  question: Address;
  kass: VaultAccounts;
  usdc: VaultAccounts;
  oraclePassKass: Address;
  oracleFailKass: Address;
}

async function composeMarket(f: Fixture, oracle: Address): Promise<MarketComposition> {
  const questionId = new Uint8Array(32).fill(0x07);
  const { question } = await composeQuestion(f, oracle, questionId, 2);
  const kass = await composeVault(f, question, f.kassMint.publicKey);
  const usdc = await composeVault(f, question, f.usdcMint.publicKey);
  const oraclePassKass = await fabricateTokenAccountMint(f, kass.passMint, oracle, 0n);
  const oracleFailKass = await fabricateTokenAccountMint(f, kass.failMint, oracle, 0n);
  return { question, kass, usdc, oraclePassKass, oracleFailKass };
}

/** openChallenge via `buildOpenChallengeIxs` → the app seam (challenger is fee-payer + signer). */
async function openChallengeViaApp(
  f: Fixture,
  nonce: bigint,
  c: Challenged,
  m: MarketComposition,
  passAmm: Address,
  failAmm: Address,
): Promise<Keypair> {
  const challenger = await Keypair.generate();
  await f.harness.airdrop(challenger.publicKey.toString(), 2_000_000_000);
  const challengerUsdcSrc = await fabricateTokenAccountMint(f, f.usdcMint.publicKey, challenger.publicKey, 5_000_000n);
  const cvEventAuthority = (await Address.findProgramAddress([enc.encode("__event_authority")], VLTX))[0];

  const ixs = await buildOpenChallengeIxs({
    oracleNonce: nonce,
    proposer: c.proposer,
    challenger: challenger.publicKey,
    question: m.question,
    kassVault: m.kass.vault,
    usdcVault: m.usdc.vault,
    passAmm,
    failAmm,
    kassVaultUnderlying: m.kass.underlying,
    passKassMint: m.kass.passMint,
    failKassMint: m.kass.failMint,
    oraclePassKass: m.oraclePassKass,
    oracleFailKass: m.oracleFailKass,
    cvEventAuthority,
    kassDao: f.kassDao,
    usdcMint: f.usdcMint.publicKey,
    challengerUsdcSrc,
  });
  await sendViaApp(f, challenger, ixs, 1_400_000);
  return challenger;
}

interface Payouts {
  escrowVault: Address;
  proposerUsdc: Address;
  challengerUsdcDest: Address;
  challengerKass: Address;
}

/**
 * settleChallenge via the SD1 `buildSettleFromMarketIxs` — DERIVING the full 15
 * settle accounts from the DECODED on-chain {@link Market} + {@link Oracle} (NOT
 * the composed-account JSON `m`), then sending through the app seam
 * (permissionless; payer sends). The three payout destinations are the DERIVED
 * ATAs (proposer-authority USDC, challenger USDC + KASS); we fabricate token
 * accounts AT those ATA addresses so the handler's `assert_token_account` +
 * transfers land on them, exactly as they would on a real cluster after an
 * idempotent create. Asserts each derived account == what the market was composed
 * with.
 */
async function settleChallengeViaApp(
  f: Fixture,
  nonce: bigint,
  c: Challenged,
  m: MarketComposition,
  market: Address,
  challenger: Keypair,
  passAmm: Address,
  failAmm: Address,
): Promise<Payouts> {
  // DECODE the on-chain Market + Oracle — the derive source (no composed JSON).
  const decodedMarket = decodeMarket(await fetchAccount(f, market));
  const decodedOracle = decodeOracle(await fetchAccount(f, c.oracle));

  // The derived payout ATAs (owner: proposer.authority / challenger).
  const proposerUsdc = await ata(c.proposerAuthority, f.usdcMint.publicKey);
  const challengerUsdcDest = await ata(challenger.publicKey, f.usdcMint.publicKey);
  const challengerKass = await ata(challenger.publicKey, f.kassMint.publicKey);
  await setTokenAccountAt(f, proposerUsdc, f.usdcMint.publicKey, c.proposerAuthority, 0n);
  await setTokenAccountAt(f, challengerUsdcDest, f.usdcMint.publicKey, challenger.publicKey, 0n);
  await setTokenAccountAt(f, challengerKass, f.kassMint.publicKey, challenger.publicKey, 0n);
  const escrowVault = (await pda.challengeUsdcVault(market)).address;

  const twapEnd = decodedMarket.twapEnd;
  await f.harness.advanceToUnix(twapEnd + 120n);

  const ixs = await buildSettleFromMarketIxs({
    oracleNonce: nonce,
    market: decodedMarket,
    oracle: decodedOracle,
    proposerAuthority: c.proposerAuthority,
  });

  // The derived settle ix binds EXACTLY the accounts the market was composed with
  // (proves the derivation is correct against the real Market, pre-flight).
  const settleIx = ixs[ixs.length - 1];
  const keys = settleIx.keys.map((k) => k.pubkey.toString());
  expect(keys[2]).toBe(c.aiClaim.toString());
  expect(keys[3]).toBe(c.proposer.toString());
  expect(keys[4]).toBe(m.question.toString());
  expect(keys[5]).toBe(passAmm.toString());
  expect(keys[6]).toBe(failAmm.toString());
  expect(keys[11]).toBe(m.kass.vault.toString());
  expect(keys[12]).toBe(m.kass.underlying.toString());
  expect(keys[13]).toBe(m.kass.passMint.toString());
  expect(keys[14]).toBe(m.kass.failMint.toString());
  expect(keys[15]).toBe(m.oraclePassKass.toString());
  expect(keys[16]).toBe(m.oracleFailKass.toString());
  expect(keys[18]).toBe(proposerUsdc.toString());
  expect(keys[19]).toBe(challengerUsdcDest.toString());
  expect(keys[20]).toBe(challengerKass.toString());

  await sendViaApp(f, f.payer, ixs, 1_400_000);
  return { escrowVault, proposerUsdc, challengerUsdcDest, challengerKass };
}

// ---------------------------------------------------------------------------
// MetaDAO market composition + AMM driving (ported from challenge-market-e2e).
// ---------------------------------------------------------------------------
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
): Promise<{ question: Address }> {
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
  return { question };
}

async function composeVault(f: Fixture, question: Address, underlyingMint: Address): Promise<VaultAccounts> {
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

async function fabricateTokenAccountMint(
  f: Fixture,
  mint: Address,
  owner: Address,
  amount: bigint,
): Promise<Address> {
  const acct = await Keypair.generate();
  await f.harness.setAccount(acct.publicKey.toString(), {
    lamports: 5_000_000,
    owner: TOKEN_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(tokenAccountBytes(mint.toBytes(), owner.toBytes(), amount)),
  });
  return acct.publicKey;
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

async function swapBuy(f: Fixture, baseMint: Address, quoteMint: Address, amountIn: bigint): Promise<void> {
  await advanceSlots(f, 200);
  await sendIx(
    f,
    await ammV04.swap({
      payer: f.payer.publicKey,
      baseMint,
      quoteMint,
      swapType: ammV04.SwapType.Buy,
      inputAmount: amountIn,
      minOutputAmount: 0n,
    }),
    [],
    1_400_000,
  );
}

async function crankPool(f: Fixture, amm: Address): Promise<void> {
  await advanceSlots(f, 300);
  await sendIx(f, await ammV04.crankThatTwap({ amm }), [], 400_000);
}

function decodeAmmTwap(data: Uint8Array): { twap: bigint } {
  const dv = new DataView(data.buffer, data.byteOffset, data.length);
  const u128 = (off: number): bigint => dv.getBigUint64(off, true) | (dv.getBigUint64(off + 8, true) << 64n);
  const createdAt = dv.getBigUint64(9, true);
  const lastUpdated = dv.getBigUint64(131, true);
  const aggregator = u128(171);
  const startDelay = dv.getBigUint64(219, true);
  const slots = lastUpdated - (createdAt + startDelay);
  const twap = slots > 0n && aggregator > 0n ? aggregator / slots : 0n;
  return { twap };
}

function questionResolution(data: Uint8Array): [number, number] {
  const dv = new DataView(data.buffer, data.byteOffset, data.length);
  return [dv.getUint32(76, true), dv.getUint32(80, true)];
}

// ---------------------------------------------------------------------------
// Dispute-core drivers over RPC (self-contained; mirror challenge-market-e2e).
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

async function fundKass(f: Fixture, owner: Address, amount: bigint): Promise<Address> {
  return fabricateTokenAccountMint(f, f.kassMint.publicKey, owner, amount);
}

async function createOracleReal(f: Fixture, nonce: bigint, optionsCount: number): Promise<void> {
  const creatorKass = await fundKass(f, f.payer.publicKey, 10n ** 15n);
  const nowUnix = await f.harness.clockUnixTimestamp();
  await sendIx(
    f,
    await createOracle({
      nonce,
      optionsCount,
      deadline: nowUnix + 1_000n,
      twapWindow: 600n,
      creator: f.payer.publicKey,
      creatorKassToken: creatorKass,
      kassMint: f.kassMint.publicKey,
      usdcMint: f.usdcMint.publicKey,
    }),
  );
}

async function openProposals(f: Fixture, oracle: Address): Promise<void> {
  const o = decodeOracle(await fetchAccount(f, oracle));
  await f.harness.advanceToUnix(o.deadline + 60n);
}

async function advancePastPhaseEnd(f: Fixture, oracle: Address): Promise<void> {
  const o = decodeOracle(await fetchAccount(f, oracle));
  await f.harness.advanceToUnix(o.phaseEndsAt + 120n);
}

async function proposeRealWithAuthority(
  f: Fixture,
  oracle: Address,
  option: number,
  bond: bigint,
): Promise<{ authority: Keypair; proposer: Address }> {
  const authority = await Keypair.generate();
  await f.harness.airdrop(authority.publicKey.toString(), 2_000_000_000);
  const authorityKass = await fundKass(f, authority.publicKey, bond * 10n);
  await sendIx(f, await propose({ oracle, authority: authority.publicKey, authorityKass, option, bond }), [authority]);
  const proposer = (await pda.proposer(oracle, authority.publicKey)).address;
  return { authority, proposer };
}

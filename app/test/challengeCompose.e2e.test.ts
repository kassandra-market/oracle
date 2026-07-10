/**
 * CU3 GATED FORKED-MAINNET surfpool COMPOSE→OPEN E2E (`KASSANDRA_E2E=1`).
 *
 * Proves the CLIENT-SIDE challenge-market composition end-to-end against a
 * surfpool validator FORKING MAINNET (so MetaDAO's DEPLOYED conditional_vault
 * `VLTX…` + v0.4 AMM `AMMyu…` are lazily fetched + EXECUTABLE), in `clock`
 * block-production mode. Unlike RF4's `challenge.e2e` — which composes the market
 * via raw CPIs + setAccount cheatcodes and only drives `open_challenge` through
 * the app — THIS suite drives the WHOLE compose→open STEP SEQUENCE through the
 * app's {@link buildComposeAndOpenChallengeIxs} builder over the
 * {@link keypairSender}/{@link sendAndConfirm} seam:
 *
 *   question → KASS vault → USDC vault → fund+split → pass pool → fail pool → open
 *
 * i.e. the same real Market the SDK challenge-market E2E's `composeMarket` +
 * `buildPool` produce, but assembled ENTIRELY by the app's compose builder (real
 * ixs, no cheatcodes: the E2E's `fabricate…`/`setTokenAccountAt` become real ATA
 * creates + `split_tokens`). The challenger is funded with REAL KASS + USDC (the
 * only setAccount here — funding a wallet, the production equivalent of a user
 * already holding tokens), then the app composes + seeds + opens.
 *
 * Asserts a REAL Market on-chain: the question/vaults/conditional mints/AMMs
 * exist + are funded, `open_challenge_count == 1`, `ai_claim.challenged`, and the
 * USDC escrow == the on-chain-computed required amount.
 *
 * Gated: skips (never fails) unless `KASSANDRA_E2E=1` AND surfpool + the `.so`
 * are present. Dedicated port (8942). The fork needs network + is slower.
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
import { buildComposeAndOpenChallengeIxs } from "../src/data/actions/challengeCompose.ts";
import { buildSubmitAiClaimIxs } from "../src/data/actions/challenge.ts";
import { keypairSender, sendAndConfirm } from "../src/data/send.ts";

const ENABLED = process.env.KASSANDRA_E2E === "1" && surfpoolReady();

const FUTARCHY_ID = EXTERNAL_PROGRAM_IDS.futarchyV06;
const KASS_PRICE_TWAP = 500_000_000n;
const KASS_PRICE_SCALE = 1_000_000_000_000n;

const BOND = 1_000_000_000n;
// The compose defaults (challengeCompose DEFAULT_BASE/QUOTE_RESERVE).
const BASE_RESERVE = 100_000_000_000n;
const QUOTE_RESERVE = 100_000_000n;

interface Fixture {
  harness: SurfpoolHarness;
  payer: Keypair;
  kassMint: Keypair;
  usdcMint: Keypair;
  kassDao: Address;
}

describe.skipIf(!ENABLED)("CU3 client-side compose→open over FORKED MetaDAO", () => {
  let f: Fixture;

  beforeAll(async () => {
    const harness = await SurfpoolHarness.start({
      port: 8942,
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

  it("app compose→open builds a REAL Market on the fork (question/vaults/mints/AMMs + escrow)", async () => {
    const nonce = 400n;
    const c = await frontDoorToChallenge(f, nonce);
    expect(decodeOracle(await fetchAccount(f, c.oracle)).phase).toBe(Phase.Challenge);
    expect(decodeAiClaim(await fetchAccount(f, c.aiClaim)).challenged).toBe(false);

    // A funded challenger with REAL KASS + USDC ATAs (the compose splits from
    // them; the ATAs must exist + hold the underlying to seed the two pools).
    const challenger = await Keypair.generate();
    await f.harness.airdrop(challenger.publicKey.toString(), 5_000_000_000);
    const challengerKassAta = await ammV04.pda.ata(challenger.publicKey, f.kassMint.publicKey);
    const challengerUsdcAta = await ammV04.pda.ata(challenger.publicKey, f.usdcMint.publicKey);
    // KASS to split into seed conditional-KASS for BOTH pools + the escrow bond.
    await setTokenAccountAt(f, challengerKassAta, f.kassMint.publicKey, challenger.publicKey, BASE_RESERVE * 4n);
    // USDC to split into seed conditional-USDC + fund the escrow (required = BOND/2000).
    await setTokenAccountAt(f, challengerUsdcAta, f.usdcMint.publicKey, challenger.publicKey, QUOTE_RESERVE * 4n + 10_000_000n);

    // ===== Build the compose→open STEP SEQUENCE via the APP builder =====
    const { steps, composed } = await buildComposeAndOpenChallengeIxs({
      connection: f.harness.connection,
      oracleNonce: nonce,
      proposer: c.proposer,
      challenger: challenger.publicKey,
      kassMint: f.kassMint.publicKey,
      usdcMint: f.usdcMint.publicKey,
      kassDao: f.kassDao,
      baseReserve: BASE_RESERVE,
      quoteReserve: QUOTE_RESERVE,
    });
    expect(steps.map((s) => s.id)).toEqual([
      "question", "kass-vault", "usdc-vault", "fund-split", "pass-pool", "fail-pool", "open",
    ]);

    // Drive each step as its own sendAndConfirm (the staged UI sequence, headless).
    for (const step of steps) {
      const ixs: TransactionInstruction[] = step.computeUnits
        ? [ComputeBudgetProgram.setComputeUnitLimit({ units: step.computeUnits }), ...step.ixs]
        : step.ixs;
      await sendAndConfirm(f.harness.connection, keypairSender(f.harness.connection, challenger), ixs);
    }

    // ===== ASSERT a REAL Market composed + opened over the fork =====
    const marketPda = (await pda.market(c.aiClaim)).address;
    const m = decodeMarket(await fetchAccount(f, marketPda));
    expect(m.oracle.toString()).toBe(c.oracle.toString());
    expect(m.proposer.toString()).toBe(c.proposer.toString());
    expect(m.challenger.toString()).toBe(challenger.publicKey.toString());
    expect(m.question.toString()).toBe(composed.question.toString());
    expect(m.kassVault.toString()).toBe(composed.kassVault.toString());
    expect(m.passAmm.toString()).toBe(composed.passAmm.toString());
    expect(m.failAmm.toString()).toBe(composed.failAmm.toString());

    // ai_claim flipped + counter incremented.
    expect(decodeAiClaim(await fetchAccount(f, c.aiClaim)).challenged).toBe(true);
    expect(decodeOracle(await fetchAccount(f, c.oracle)).openChallengeCount).toBe(1);

    // Question + vaults + conditional mints exist (owned by the forked vault).
    expect((await f.harness.connection.getAccountInfo(composed.question))!.owner.toString()).toBe(
      EXTERNAL_PROGRAM_IDS.conditionalVault.toString(),
    );
    for (const mint of [composed.passKassMint, composed.failKassMint, composed.passUsdcMint, composed.failUsdcMint]) {
      expect((await f.harness.connection.getAccountInfo(mint))!.owner.toString()).toBe(
        TOKEN_PROGRAM_ID.toString(),
      );
    }
    // The two AMMs exist + are owned by the forked AMM program (real create_amm).
    for (const amm of [composed.passAmm, composed.failAmm]) {
      const info = await f.harness.connection.getAccountInfo(amm);
      expect(info, "amm pool must be created on-chain").not.toBeNull();
      expect(info!.owner.toString()).toBe(EXTERNAL_PROGRAM_IDS.ammV04.toString());
    }
    // Pools seeded: each vault (pool base/quote ATA) holds the seed reserve.
    const passVaultBase = await ammV04.pda.ata(composed.passAmm, composed.passKassMint);
    const passVaultQuote = await ammV04.pda.ata(composed.passAmm, composed.passUsdcMint);
    expect(await tokenBalance(f, passVaultBase)).toBe(BASE_RESERVE);
    expect(await tokenBalance(f, passVaultQuote)).toBe(QUOTE_RESERVE);

    // USDC escrow funded with the on-chain required amount + bond split into KASS.
    const escrow = (await pda.challengeUsdcVault(marketPda)).address;
    const requiredUsdc = (BOND * KASS_PRICE_TWAP) / KASS_PRICE_SCALE;
    expect(await tokenBalance(f, escrow)).toBe(requiredUsdc);
    expect(m.challengerUsdc).toBe(requiredUsdc);
    expect(await tokenBalance(f, composed.oraclePassKass)).toBe(BOND);
    expect(await tokenBalance(f, composed.oracleFailKass)).toBe(BOND);
  }, 300_000);
});

// ---------------------------------------------------------------------------
// Fixture helpers (mirror challenge.e2e; the market composition itself is the
// app builder under test, so only the front-door drivers are ported here).
// ---------------------------------------------------------------------------
interface Challenged {
  oracle: Address;
  proposer: Address;
  aiClaim: Address;
  proposerPdas: Address[];
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
    await sendAndConfirm(f.harness.connection, keypairSender(f.harness.connection, authorities[i]), ixs);
  }

  await advancePastPhaseEnd(f, oracle);
  await sendIx(f, await finalizeAiClaims({ oracle, proposers: proposerPdas }));

  const proposer = proposerPdas[0];
  const aiClaim = (await pda.aiClaim(oracle, proposer)).address;
  return { oracle, proposer, aiClaim, proposerPdas };
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

async function fabricateTokenAccountMint(
  f: Fixture,
  mint: Address,
  owner: Address,
  amount: bigint,
): Promise<Address> {
  const acct = await Keypair.generate();
  await setTokenAccountAt(f, acct.publicKey, mint, owner, amount);
  return acct.publicKey;
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

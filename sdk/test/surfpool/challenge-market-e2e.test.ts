/**
 * T4 surfpool CHALLENGE-MARKET E2E (GATED, FORKED MetaDAO) — the challenge push.
 *
 * Unlike T1-T3 (a hermetic standalone simnet), this suite boots surfpool
 * FORKING MAINNET (`--network mainnet`) so MetaDAO's DEPLOYED programs are
 * lazily fetched and EXECUTABLE over RPC:
 *   - conditional_vault `VLTX1ish…` (v0.4.0)
 *   - amm              `AMMyu265…` (v0.4.2 delayed-twap)
 *   - futarchy         `FUTARELBf…` (v0.6)
 *   - Meteora DAMM v2  `cpamdpZC…`, Squads v4 `SQDS4ep6…`
 *
 * Three arms, in increasing depth (each only asserts what GENUINELY happens on
 * the fork — see the README's covered-vs-deferred):
 *
 *   1. PROGRAMS LOAD — fetch all five MetaDAO program accounts over RPC; assert
 *      each is `executable` + owned by the (upgradeable) BPF loader. Proves the
 *      fork makes the real deployed programs available.
 *
 *   2. CONDITIONAL_VAULT EXECUTES — drive a REAL `initialize_question` CPI to
 *      the forked vault and assert the on-chain `Question` account is created
 *      (owner == vault program, decoded `oracle`/`num_outcomes` match). Proves
 *      the forked program is not just present but EXECUTES (far past "program
 *      not found").
 *
 *   3. OPEN A CHALLENGE — drive the full Kassandra dispute core to `Challenge`
 *      (real instructions, clock advanced via `surfnet_timeTravel`), COMPOSE the
 *      MetaDAO market (real `initialize_question` + KASS/USDC `initialize_
 *      conditional_vault` CPIs; the pass/fail AMMs are placeholder accounts
 *      owned by the AMM program — `open_challenge` only checks AMM OWNERSHIP),
 *      then call the Kassandra `openChallenge` instruction. Its program-signed
 *      `split_tokens` CPI runs against the FORKED conditional_vault. Asserts the
 *      `Market` PDA is created, `ai_claim.challenged == 1`, the USDC escrow is
 *      funded, and the bond was physically split into conditional KASS. This is
 *      a Kassandra instruction that CPIs into a forked MetaDAO program and
 *      succeeds.
 *
 * DEFERRED (documented, NOT asserted): full `settle_challenge` (it reads a real
 * swap-driven AMM TWAP — building + cranking two live AMM pools over RPC on a
 * fork is left to a future pass) and live-cluster submission.
 *
 * GATING: included only when `KASSANDRA_E2E=1`; skips (not fails) when surfpool
 * / the `.so` are absent. The fork needs network (mainnet datasource) + is
 * slower than the standalone core path.
 */
import {
  Address,
  ComputeBudgetProgram,
  Keypair,
  Transaction,
  TransactionInstruction,
} from "@solana/web3.js";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { decodeAiClaim, decodeMarket, decodeOracle } from "../../src/accounts/index.js";
import { EXTERNAL_PROGRAM_IDS, Phase, TOKEN_PROGRAM_ID, VOTE_APPROVE } from "../../src/constants.js";
import {
  advancePhase,
  createOracle,
  finalizeAiClaims,
  finalizeFacts,
  finalizeProposals,
  initProtocol,
  openChallenge,
  propose,
  setGovernance,
  submitAiClaim,
  submitFact,
  voteFact,
} from "../../src/instructions/index.js";
import * as pda from "../../src/pda.js";

import {
  SurfpoolHarness,
  mintBytes,
  surfpoolReady,
  toHex,
  tokenAccountAmount,
  tokenAccountBytes,
} from "./harness.js";

const ENABLED = process.env.KASSANDRA_E2E === "1" && surfpoolReady();

const SYSTEM_PROGRAM_ID = new Address("11111111111111111111111111111111");
const ATA_PROGRAM_ID = new Address("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
const VLTX = EXTERNAL_PROGRAM_IDS.conditionalVault;
const AMM_ID = EXTERNAL_PROGRAM_IDS.ammV04;
const FUTARCHY_ID = EXTERNAL_PROGRAM_IDS.futarchyV06;
const BPF_UPGRADEABLE = "BPFLoaderUpgradeab1e11111111111111111111111";

// MetaDAO Anchor discriminators (mirror `src/cpi/metadao.rs`).
const INITIALIZE_QUESTION = Uint8Array.from([0xf5, 0x97, 0x6a, 0xbc, 0x58, 0x2c, 0x41, 0xd4]);
const INITIALIZE_CONDITIONAL_VAULT = Uint8Array.from([0x25, 0x58, 0xfa, 0xd4, 0x36, 0xda, 0xe3, 0xaf]);

// `kass_price` consts (mirror `config.rs` + the Rust test harness).
const KASS_PRICE_TWAP = 500_000_000n;
const KASS_PRICE_SCALE = 1_000_000_000_000n;
const FUTARCHY_DAO_DISC = Uint8Array.from([0xa3, 0x09, 0x2f, 0x1f, 0x34, 0x55, 0xc5, 0x31]);

const enc = new TextEncoder();

/** 1 KASS (9 dp) bond — large enough that required_usdc = bond×twap/scale > 0. */
const BOND = 1_000_000_000n;

interface Fixture {
  harness: SurfpoolHarness;
  payer: Keypair;
  kassMint: Keypair;
  usdcMint: Keypair;
  kassDao: Address;
}

describe.skipIf(!ENABLED)("surfpool challenge-market on FORKED MetaDAO (T4)", () => {
  let f: Fixture;

  beforeAll(async () => {
    // Fork mainnet so the deployed MetaDAO programs are fetchable. Dedicated
    // port (8920) so it never collides with the smoke (8899) / lifecycle (8901).
    const harness = await SurfpoolHarness.start({ port: 8920, fork: "mainnet", readyTimeoutMs: 60_000 });
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

    // Fabricate a futarchy-owned `Dao` carrying a deterministic spot TWAP
    // (== KASS_PRICE_TWAP), then `set_governance` records it as protocol.kass_dao
    // so open_challenge's `kass_price` returns a positive escrow size. Mirrors
    // the Rust harness `bless_kass_price` / `build_dao_blob`.
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
    // One-shot governance handoff: record the kass_dao + a stand-in dao_authority.
    const daoAuthority = (await Keypair.generate()).publicKey;
    await sendIx(f, await setGovernance({
      authority: payer.publicKey,
      daoAuthority,
      kassDao,
    }));
  }, 120_000);

  afterAll(async () => {
    await f?.harness.teardown();
  });

  it("forked MetaDAO programs load (executable, BPF-upgradeable-owned)", async () => {
    const ids: Array<[string, Address]> = [
      ["conditionalVault", VLTX],
      ["ammV04", AMM_ID],
      ["futarchyV06", FUTARCHY_ID],
      ["meteoraDammV2", EXTERNAL_PROGRAM_IDS.meteoraDammV2],
      ["squadsV4", EXTERNAL_PROGRAM_IDS.squadsV4],
    ];
    for (const [name, id] of ids) {
      const info = await f.harness.connection.getAccountInfo(id);
      expect(info, `${name} (${id}) not fetched from the fork`).not.toBeNull();
      expect(info!.executable, `${name} should be executable`).toBe(true);
      expect(info!.owner.toString(), `${name} owner`).toBe(BPF_UPGRADEABLE);
    }
  }, 60_000);

  it("forked conditional_vault EXECUTES: initialize_question creates a Question", async () => {
    const resolver = (await Keypair.generate()).publicKey;
    const questionId = new Uint8Array(32).fill(0x5a);
    const { question } = await composeQuestion(f, resolver, questionId, 2);

    const data = await fetchAccount(f, question);
    // owner == the forked vault program; the CPI actually ran + allocated state.
    const info = await f.harness.connection.getAccountInfo(question);
    expect(info!.owner.toString()).toBe(VLTX.toString());
    // Question layout (8-byte Anchor disc first): oracle @40, num_outcomes len @72.
    expect(toHex(data.slice(40, 72))).toBe(toHex(resolver.toBytes()));
    expect(new DataView(data.buffer, data.byteOffset, data.length).getUint32(72, true)).toBe(2);
  }, 90_000);

  it("opens a challenge market against the FORKED MetaDAO (real split_tokens CPI)", async () => {
    const nonce = 100n;
    const oracle = (await pda.oracle(nonce)).address;
    const aiOption = 0;

    // --- REAL dispute core → Challenge (clock advanced via surfnet_timeTravel) ---
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
    expect(decodeOracle(await fetchAccount(f, oracle)).phase).toBe(Phase.FactProposal);

    // submit_fact → advance → advance_phase → vote → advance → finalize_facts → AiClaim
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
    expect(decodeOracle(await fetchAccount(f, oracle)).phase).toBe(Phase.FactVoting);

    const voter = await Keypair.generate();
    await f.harness.airdrop(voter.publicKey.toString(), 2_000_000_000);
    const voterKass = await fundKass(f, voter.publicKey, 10n * BOND);
    await sendIx(
      f,
      await voteFact({ oracle, fact, voter: voter.publicKey, voterKass, kind: VOTE_APPROVE, stake: 2n * BOND }),
      [voter],
    );

    await advancePastPhaseEnd(f, oracle);
    await sendIx(f, await finalizeFacts({ oracle, tail: [fact] }));
    expect(decodeOracle(await fetchAccount(f, oracle)).phase).toBe(Phase.AiClaim);

    // Both proposers claim option 0: proposer[0] (orig 0) does NOT flip → survives
    // un-slashed → the clean bond we challenge.
    for (let i = 0; i < proposerPdas.length; i++) {
      await sendIx(
        f,
        await submitAiClaim({
          oracle,
          proposer: proposerPdas[i],
          authority: authorities[i].publicKey,
          modelId: new Uint8Array(32).fill(0xa1),
          paramsHash: new Uint8Array(32).fill(0xb2),
          ioHash: new Uint8Array(32).fill(0xc3),
          option: aiOption,
        }),
        [authorities[i]],
      );
    }

    await advancePastPhaseEnd(f, oracle);
    await sendIx(f, await finalizeAiClaims({ oracle, proposers: proposerPdas }));
    expect(decodeOracle(await fetchAccount(f, oracle)).phase).toBe(Phase.Challenge);

    const proposer = proposerPdas[0];
    const aiClaim = (await pda.aiClaim(oracle, proposer)).address;
    expect(decodeAiClaim(await fetchAccount(f, aiClaim)).challenged).toBe(false);

    // --- COMPOSE the MetaDAO market (resolver == oracle PDA) on the fork ---
    const questionId = new Uint8Array(32).fill(0x07);
    const { question } = await composeQuestion(f, oracle, questionId, 2);
    const kass = await composeVault(f, question, f.kassMint.publicKey);
    const usdc = await composeVault(f, question, f.usdcMint.publicKey);

    // Oracle-PDA-owned pass/fail conditional-KASS holders (fabricated empty;
    // split_tokens mints into them). Mirrors the Rust harness fabricate_token_account.
    const oraclePassKass = await fabricateTokenAccountMint(f, kass.passMint, oracle, 0n);
    const oracleFailKass = await fabricateTokenAccountMint(f, kass.failMint, oracle, 0n);

    // Pass/fail AMM placeholders: open_challenge only checks owner == AMM program.
    const passAmm = await fabricateAmmOwned(f);
    const failAmm = await fabricateAmmOwned(f);

    // Challenger USDC source (escrow = BOND×twap/scale = 0.5 USDC; fund generously).
    const challenger = await Keypair.generate();
    await f.harness.airdrop(challenger.publicKey.toString(), 2_000_000_000);
    const challengerUsdcSrc = await fabricateTokenAccountMint(
      f,
      f.usdcMint.publicKey,
      challenger.publicKey,
      5_000_000n,
    );

    const cvEventAuthority = (await Address.findProgramAddress([enc.encode("__event_authority")], VLTX))[0];

    // --- OPEN THE CHALLENGE: program-signed split_tokens CPI → forked vault ---
    const ix = await openChallenge({
      nonce,
      proposer,
      challenger: challenger.publicKey,
      question,
      kassVault: kass.vault,
      usdcVault: usdc.vault,
      passAmm,
      failAmm,
      kassVaultUnderlying: kass.underlying,
      passKassMint: kass.passMint,
      failKassMint: kass.failMint,
      oraclePassKass,
      oracleFailKass,
      cvEventAuthority,
      kassDao: f.kassDao,
      usdcMint: f.usdcMint.publicKey,
      challengerUsdcSrc,
    });
    await sendIx(f, ix, [challenger], 1_400_000);

    // --- ASSERT the challenge market opened ---
    const market = (await pda.market(aiClaim)).address;
    const m = decodeMarket(await fetchAccount(f, market));
    expect(m.oracle.toString()).toBe(oracle.toString());
    expect(m.proposer.toString()).toBe(proposer.toString());
    expect(m.challenger.toString()).toBe(challenger.publicKey.toString());
    expect(m.question.toString()).toBe(question.toString());
    expect(m.kassVault.toString()).toBe(kass.vault.toString());

    // ai_claim flipped to challenged.
    expect(decodeAiClaim(await fetchAccount(f, aiClaim)).challenged).toBe(true);
    // open_challenge_count incremented.
    expect(decodeOracle(await fetchAccount(f, oracle)).openChallengeCount).toBe(1);

    // USDC escrow funded with the on-chain-computed required amount (BOND/2000).
    const escrow = (await pda.challengeUsdcVault(market)).address;
    const requiredUsdc = (BOND * KASS_PRICE_TWAP) / KASS_PRICE_SCALE;
    expect(await tokenBalance(f, escrow)).toBe(requiredUsdc);
    expect(m.challengerUsdc).toBe(requiredUsdc);

    // The bond was physically SPLIT into conditional KASS via the forked vault:
    // pass-KASS + fail-KASS each == BOND, and the underlying landed in the vault.
    expect(await tokenBalance(f, oraclePassKass)).toBe(BOND);
    expect(await tokenBalance(f, oracleFailKass)).toBe(BOND);
    expect(await tokenBalance(f, kass.underlying)).toBe(BOND);
  }, 240_000);
});

// ---------------------------------------------------------------------------
// MetaDAO market composition over RPC (mirrors challenge_e2e.rs setup_market)
// ---------------------------------------------------------------------------

/** Build a futarchy `Dao` blob with an embedded spot TWAP at the F0 offsets. */
function buildDaoBlob(aggregator: bigint, lastUpdated: bigint, createdAt: bigint, startDelay: number): Uint8Array {
  const data = new Uint8Array(141); // DAO_SPOT_POOL_OFFSET(9) + FUTARCHY_POOL_LEN(132)
  data.set(FUTARCHY_DAO_DISC, 0);
  data[8] = 0; // PoolState::Spot
  const dv = new DataView(data.buffer);
  // aggregator: u128 @9 (write as two u64 LE halves).
  dv.setBigUint64(9, aggregator & 0xffffffffffffffffn, true);
  dv.setBigUint64(17, aggregator >> 64n, true);
  dv.setBigInt64(25, lastUpdated, true); // last_updated_ts: i64 @25
  dv.setBigInt64(33, createdAt, true); // created_at_ts: i64 @33
  dv.setUint32(105, startDelay, true); // start_delay_seconds: u32 @105
  return data;
}

interface VaultAccounts {
  vault: Address;
  underlying: Address;
  passMint: Address;
  failMint: Address;
}

/** ATA derivation `[owner, token_program, mint]` under the ATA program. */
async function ata(owner: Address, mint: Address): Promise<Address> {
  return (
    await Address.findProgramAddress(
      [owner.toBytes(), TOKEN_PROGRAM_ID.toBytes(), mint.toBytes()],
      ATA_PROGRAM_ID,
    )
  )[0];
}

/** Real `initialize_question` CPI: a binary question whose resolver == `oracle`. */
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

/** Real `initialize_conditional_vault` CPI for `underlyingMint` against `question`. */
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

/** Fabricate an SPL token account (owner = `owner`) on `mint` with `amount`. */
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

/** Fabricate a placeholder account owned by the AMM program (open only checks owner). */
async function fabricateAmmOwned(f: Fixture): Promise<Address> {
  const acct = await Keypair.generate();
  await f.harness.setAccount(acct.publicKey.toString(), {
    lamports: 5_000_000,
    owner: AMM_ID.toString(),
    executable: false,
    data: toHex(new Uint8Array(8)),
  });
  return acct.publicKey;
}

// ---------------------------------------------------------------------------
// Dispute-core drivers over RPC (self-contained; mirror lifecycle-e2e.test.ts)
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
      promptHash: new Uint8Array(32).fill(0x42),
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

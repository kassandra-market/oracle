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
import { buildDaoBlob } from "./futarchy-dao.js";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import {
  decodeAiClaim,
  decodeMarket,
  decodeOracle,
  decodeProposer,
} from "../../src/accounts/index.js";
import { ammV04, futarchy } from "../../src/index.js";
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
  settleChallenge,
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

const enc = new TextEncoder();

/** 1 KASS (9 dp) bond — large enough that required_usdc = bond×twap/scale > 0. */
const BOND = 1_000_000_000n;

// --- v0.4 AMM pool seeding (mirror challenge_e2e.rs build_pool) ---------------
/** Largest per-update observation change — a single crank folds the current
 * price straight into the TWAP (no clamp), so the cranked TWAP is deterministic.
 * (== `u64::MAX × 1e12`, the same value the Rust e2e uses.) */
const MAX_PRICE = ((1n << 64n) - 1n) * 1_000_000_000_000n;
/** Base reserve: 100 conditional-KASS (9 dp). */
const BASE_RESERVE = 100_000_000_000n;
/** Quote reserve: 100 conditional-USDC (6 dp) → seeded price 1e9 (scaled). */
const QUOTE_NEUTRAL = 100_000_000n;

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
    // `clock` block-production (fast slot-time) so the on-chain EXECUTION slot
    // advances over wall-clock — the v0.4 AMM crank is SLOT-based and
    // surfnet_timeTravel moves only getSlot/unix_timestamp, not the slot the
    // program sees during execution. Dispute-core time gates still use timeTravel.
    const harness = await SurfpoolHarness.start({
      port: 8920,
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
    // One-shot governance handoff. The G1-hardened set_governance requires
    // dao_authority == the Squads v4 vault PDA derived for kass_dao
    // (multisig create_key == kass_dao → multisig → vault idx 0), so derive it
    // rather than passing a stand-in (else KassandraError::DaoAuthorityMismatch).
    const multisig = (await futarchy.pda.squadsMultisig(kassDao)).address;
    const daoAuthority = (await futarchy.pda.squadsVault(multisig, 0)).address;
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
    const c = await frontDoorToChallenge(f, nonce);
    expect(decodeOracle(await fetchAccount(f, c.oracle)).phase).toBe(Phase.Challenge);
    expect(decodeAiClaim(await fetchAccount(f, c.aiClaim)).challenged).toBe(false);

    const market = await composeMarket(f, c.oracle);

    // Pass/fail AMM placeholders: open_challenge only checks owner == AMM program.
    // (The two settle arms below build REAL pools instead.)
    const passAmm = await fabricateAmmOwned(f);
    const failAmm = await fabricateAmmOwned(f);

    const { challenger } = await openChallengeReal(f, nonce, c, market, passAmm, failAmm);

    // --- ASSERT the challenge market opened ---
    const marketPda = (await pda.market(c.aiClaim)).address;
    const m = decodeMarket(await fetchAccount(f, marketPda));
    expect(m.oracle.toString()).toBe(c.oracle.toString());
    expect(m.proposer.toString()).toBe(c.proposer.toString());
    expect(m.challenger.toString()).toBe(challenger.publicKey.toString());
    expect(m.question.toString()).toBe(market.question.toString());
    expect(m.kassVault.toString()).toBe(market.kass.vault.toString());

    // ai_claim flipped to challenged.
    expect(decodeAiClaim(await fetchAccount(f, c.aiClaim)).challenged).toBe(true);
    // open_challenge_count incremented.
    expect(decodeOracle(await fetchAccount(f, c.oracle)).openChallengeCount).toBe(1);

    // USDC escrow funded with the on-chain-computed required amount (BOND/2000).
    const escrow = (await pda.challengeUsdcVault(marketPda)).address;
    const requiredUsdc = (BOND * KASS_PRICE_TWAP) / KASS_PRICE_SCALE;
    expect(await tokenBalance(f, escrow)).toBe(requiredUsdc);
    expect(m.challengerUsdc).toBe(requiredUsdc);

    // The bond was physically SPLIT into conditional KASS via the forked vault:
    // pass-KASS + fail-KASS each == BOND, and the underlying landed in the vault.
    expect(await tokenBalance(f, market.oraclePassKass)).toBe(BOND);
    expect(await tokenBalance(f, market.oracleFailKass)).toBe(BOND);
    expect(await tokenBalance(f, market.kass.underlying)).toBe(BOND);
  }, 240_000);

  it("DISQUALIFY: real swap-driven FAIL-pool TWAP clears the 10% margin → settle slashes", async () => {
    const nonce = 200n;
    const c = await frontDoorToChallenge(f, nonce);
    const market = await composeMarket(f, c.oracle);

    // --- REAL pass/fail v0.4 AMM pools (port build_pool) ---
    // PASS pool stays neutral; FAIL pool gets a genuine BUY swap that pushes its
    // price up, then TWO cranks ≥150 slots apart accumulate the post-swap price
    // into the slot-weighted TWAP — so the disqualify decision is driven by REAL
    // trading moving the TWAP past `pass + 10% threshold`, not a seeded price.
    const passAmm = await buildPool(f, market.kass.passMint, market.usdc.passMint, BASE_RESERVE, QUOTE_NEUTRAL);
    const failAmm = await buildPool(f, market.kass.failMint, market.usdc.failMint, BASE_RESERVE, QUOTE_NEUTRAL);
    await crankPool(f, passAmm);
    // 90 USDC BUY drains the fail pool's base hard → instantaneous price ≈ 3.5e9.
    await swapBuy(f, market.kass.failMint, market.usdc.failMint, 90_000_000n);
    await crankPool(f, failAmm); // records the post-swap price
    await crankPool(f, failAmm); // accumulates it: TWAP ≈ (1e9 + 3.5e9)/2 ≫ 1.1e9

    // --- The REAL crank actually moved the FAIL TWAP past the margin (decode
    // the live Amm accounts over RPC; this is the swap-driven verdict, not a stub).
    const passTwap = decodeAmmTwap(await fetchAccount(f, passAmm)).twap;
    const failTwap = decodeAmmTwap(await fetchAccount(f, failAmm)).twap;
    expect(passTwap, "pass TWAP must be a real non-zero observation").toBeGreaterThan(0n);
    expect(failTwap * 10n, "fail*DEN must clear pass*(DEN+NUM) — the 10% margin").toBeGreaterThan(
      passTwap * 11n,
    );

    const { challenger } = await openChallengeReal(f, nonce, c, market, passAmm, failAmm);
    const marketPda = (await pda.market(c.aiClaim)).address;

    const oBefore = decodeOracle(await fetchAccount(f, c.oracle));
    const stakeVault = (await pda.stakeVault(c.oracle)).address;
    const stakeBefore = await tokenBalance(f, stakeVault);

    const payouts = await settleChallengeReal(f, nonce, c, market, marketPda, challenger, passAmm, failAmm);

    // --- ASSERT the disqualify economics over RPC ---
    const escrow = (BOND * KASS_PRICE_TWAP) / KASS_PRICE_SCALE; // 500_000
    const kassFee = BOND / 100n; // CHALLENGE_SUCCESS_KASS_FEE = 1/100
    // Question resolved FAIL-side [0,1].
    expect(questionResolution(await fetchAccount(f, market.question))).toEqual([0, 1]);
    // Market settled + counter back to 0.
    expect(decodeMarket(await fetchAccount(f, marketPda)).settled).toBe(true);
    expect(decodeOracle(await fetchAccount(f, c.oracle)).openChallengeCount).toBe(0);
    // Proposer disqualified + slashed `bond − kass_fee` into bond_pool.
    const p = decodeProposer(await fetchAccount(f, c.proposer));
    expect(p.disqualified).toBe(true);
    expect(p.slashed).toBe(true);
    expect(p.slashedAmount).toBe(BOND - kassFee);
    const oAfter = decodeOracle(await fetchAccount(f, c.oracle));
    expect(oAfter.survivingCount).toBe(oBefore.survivingCount - 1);
    expect(oAfter.bondPool).toBe(oBefore.bondPool + (BOND - kassFee));
    // KASS: kass_fee → challenger; bond − kass_fee redeemed into stake_vault.
    expect(await tokenBalance(f, payouts.challengerKass)).toBe(kassFee);
    expect(await tokenBalance(f, stakeVault)).toBe(stakeBefore + (BOND - kassFee));
    // The bond's conditional KASS was redeemed (holders burned, underlying drained).
    expect(await tokenBalance(f, market.oraclePassKass)).toBe(0n);
    expect(await tokenBalance(f, market.oracleFailKass)).toBe(0n);
    expect(await tokenBalance(f, market.kass.underlying)).toBe(0n);
    // USDC: full escrow → challenger; no proposer fee.
    expect(await tokenBalance(f, payouts.challengerUsdcDest)).toBe(escrow);
    expect(await tokenBalance(f, payouts.proposerUsdc)).toBe(0n);
    expect(await tokenBalance(f, payouts.escrowVault)).toBe(0n);
  }, 300_000);

  it("SURVIVE: both pools neutral (pass==fail) → settle returns escrow, bond unslashed", async () => {
    const nonce = 300n;
    const c = await frontDoorToChallenge(f, nonce);
    const market = await composeMarket(f, c.oracle);

    // Both pools at the neutral seeded price (1e9) → pass == fail → survives.
    const passAmm = await buildPool(f, market.kass.passMint, market.usdc.passMint, BASE_RESERVE, QUOTE_NEUTRAL);
    const failAmm = await buildPool(f, market.kass.failMint, market.usdc.failMint, BASE_RESERVE, QUOTE_NEUTRAL);
    await crankPool(f, passAmm);
    await crankPool(f, failAmm);

    // Neutral: BOTH pools carry a REAL non-zero observation (not a trivial
    // pass==0 survive), and fail does NOT clear pass*(DEN+NUM) → the margin
    // holds → survive.
    const passTwap = decodeAmmTwap(await fetchAccount(f, passAmm)).twap;
    const failTwap = decodeAmmTwap(await fetchAccount(f, failAmm)).twap;
    expect(passTwap, "pass TWAP must be a real non-zero observation").toBeGreaterThan(0n);
    expect(failTwap, "fail TWAP must be a real non-zero observation").toBeGreaterThan(0n);
    expect(failTwap * 10n).toBeLessThanOrEqual(passTwap * 11n);

    const { challenger } = await openChallengeReal(f, nonce, c, market, passAmm, failAmm);
    const marketPda = (await pda.market(c.aiClaim)).address;

    const oBefore = decodeOracle(await fetchAccount(f, c.oracle));
    const stakeVault = (await pda.stakeVault(c.oracle)).address;
    const stakeBefore = await tokenBalance(f, stakeVault);

    const payouts = await settleChallengeReal(f, nonce, c, market, marketPda, challenger, passAmm, failAmm);

    // --- ASSERT the survive economics over RPC ---
    const escrow = (BOND * KASS_PRICE_TWAP) / KASS_PRICE_SCALE; // 500_000
    const usdcFee = escrow / 100n; // CHALLENGE_FAIL_USDC_FEE = 1/100
    // Question resolved PASS-side [1,0].
    expect(questionResolution(await fetchAccount(f, market.question))).toEqual([1, 0]);
    expect(decodeMarket(await fetchAccount(f, marketPda)).settled).toBe(true);
    // Proposer survives un-slashed; bond stays theirs (redeemed into stake_vault).
    const p = decodeProposer(await fetchAccount(f, c.proposer));
    expect(p.disqualified).toBe(false);
    expect(p.slashedAmount).toBe(0n);
    const oAfter = decodeOracle(await fetchAccount(f, c.oracle));
    expect(oAfter.bondPool).toBe(oBefore.bondPool); // no slash
    expect(oAfter.survivingCount).toBe(oBefore.survivingCount);
    expect(await tokenBalance(f, stakeVault)).toBe(stakeBefore + BOND);
    expect(await tokenBalance(f, payouts.challengerKass)).toBe(0n);
    // USDC: fee → proposer, remainder → challenger (escrow fully accounted).
    expect(await tokenBalance(f, payouts.proposerUsdc)).toBe(usdcFee);
    expect(await tokenBalance(f, payouts.challengerUsdcDest)).toBe(escrow - usdcFee);
    expect(await tokenBalance(f, payouts.escrowVault)).toBe(0n);
  }, 300_000);
});

// ---------------------------------------------------------------------------
// MetaDAO market composition over RPC (mirrors challenge_e2e.rs setup_market)
// ---------------------------------------------------------------------------
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
// Front door → Challenge + market composition + open/settle (shared by arms)
// ---------------------------------------------------------------------------

interface Challenged {
  oracle: Address;
  proposer: Address;
  proposerAuthority: Address;
  aiClaim: Address;
  proposerPdas: Address[];
  authorities: Keypair[];
}

/**
 * Drive the REAL dispute core (clock advanced via `surfnet_timeTravel`) to
 * `Phase::Challenge`. The returned proposer is the option-0 proposer who claims
 * option 0 (no flip) → surviving, `slashed_amount == 0`: a clean bond to
 * challenge. Mirrors `challenge_e2e.rs::front_door_to_challenge`.
 */
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

  const proposer = proposerPdas[0];
  const aiClaim = (await pda.aiClaim(oracle, proposer)).address;
  return {
    oracle,
    proposer,
    proposerAuthority: authorities[0].publicKey,
    aiClaim,
    proposerPdas,
    authorities,
  };
}

interface MarketComposition {
  question: Address;
  kass: VaultAccounts;
  usdc: VaultAccounts;
  oraclePassKass: Address;
  oracleFailKass: Address;
}

/** Compose the binary question + KASS/USDC conditional vaults (resolver == oracle)
 * + the oracle-PDA-owned pass/fail conditional-KASS holders. */
async function composeMarket(f: Fixture, oracle: Address): Promise<MarketComposition> {
  const questionId = new Uint8Array(32).fill(0x07);
  const { question } = await composeQuestion(f, oracle, questionId, 2);
  const kass = await composeVault(f, question, f.kassMint.publicKey);
  const usdc = await composeVault(f, question, f.usdcMint.publicKey);
  const oraclePassKass = await fabricateTokenAccountMint(f, kass.passMint, oracle, 0n);
  const oracleFailKass = await fabricateTokenAccountMint(f, kass.failMint, oracle, 0n);
  return { question, kass, usdc, oraclePassKass, oracleFailKass };
}

/** Send the Kassandra `open_challenge` (program-signed `split_tokens` CPI →
 * forked vault). Returns the fresh challenger + the Market PDA. */
async function openChallengeReal(
  f: Fixture,
  nonce: bigint,
  c: Challenged,
  m: MarketComposition,
  passAmm: Address,
  failAmm: Address,
): Promise<{ challenger: Keypair; market: Address }> {
  const challenger = await Keypair.generate();
  await f.harness.airdrop(challenger.publicKey.toString(), 2_000_000_000);
  const challengerUsdcSrc = await fabricateTokenAccountMint(f, f.usdcMint.publicKey, challenger.publicKey, 5_000_000n);
  const cvEventAuthority = (await Address.findProgramAddress([enc.encode("__event_authority")], VLTX))[0];

  await sendIx(
    f,
    await openChallenge({
      nonce,
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
    }),
    [challenger],
    1_400_000,
  );
  const market = (await pda.market(c.aiClaim)).address;
  return { challenger, market };
}

interface Payouts {
  escrowVault: Address;
  proposerUsdc: Address;
  challengerUsdcDest: Address;
  challengerKass: Address;
}

/** Fabricate the (empty) settle payout destinations, advance past `twap_end`, and
 * send the Kassandra `settle_challenge`. Returns the payout accounts to assert on. */
async function settleChallengeReal(
  f: Fixture,
  nonce: bigint,
  c: Challenged,
  m: MarketComposition,
  market: Address,
  challenger: Keypair,
  passAmm: Address,
  failAmm: Address,
): Promise<Payouts> {
  const proposerUsdc = await fabricateTokenAccountMint(f, f.usdcMint.publicKey, c.proposerAuthority, 0n);
  const challengerUsdcDest = await fabricateTokenAccountMint(f, f.usdcMint.publicKey, challenger.publicKey, 0n);
  const challengerKass = await fabricateTokenAccountMint(f, f.kassMint.publicKey, challenger.publicKey, 0n);
  const escrowVault = (await pda.challengeUsdcVault(market)).address;
  const cvEventAuthority = (await Address.findProgramAddress([enc.encode("__event_authority")], VLTX))[0];

  // Gate: settle is allowed only after market.twap_end (now + oracle.twap_window).
  const twapEnd = decodeMarket(await fetchAccount(f, market)).twapEnd;
  await f.harness.advanceToUnix(twapEnd + 120n);

  await sendIx(
    f,
    await settleChallenge({
      nonce,
      aiClaim: c.aiClaim,
      proposer: c.proposer,
      question: m.question,
      passAmm,
      failAmm,
      cvEventAuthority,
      kassVault: m.kass.vault,
      kassVaultUnderlying: m.kass.underlying,
      passKassMint: m.kass.passMint,
      failKassMint: m.kass.failMint,
      oraclePassKass: m.oraclePassKass,
      oracleFailKass: m.oracleFailKass,
      proposerUsdc,
      challengerUsdcDest,
      challengerKass,
    }),
    [],
    1_400_000,
  );
  return { escrowVault, proposerUsdc, challengerUsdcDest, challengerKass };
}

// ---------------------------------------------------------------------------
// Real v0.4 AMM pool driving over RPC (port challenge_e2e.rs build/swap/crank)
// ---------------------------------------------------------------------------

/** Write canonical SPL token-account bytes AT a specific (ATA) address. */
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

/** Wait for the on-chain EXECUTION slot to advance by ≥ `n` (the v0.4 AMM crank
 * rate-limit is slot-based: `ONE_MINUTE_IN_SLOTS == 150`). In `clock`
 * block-production mode the slot advances on a wall-clock timer, so we poll
 * `getSlot` until it has moved past `start + n`. */
async function advanceSlots(f: Fixture, n: number): Promise<void> {
  const start = await f.harness.currentSlot();
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if ((await f.harness.currentSlot()) >= start + n) return;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`slot did not advance by ${n} within 30s (clock mode not producing blocks?)`);
}

/** `create_amm` + `add_liquidity` for one (base, quote) conditional pair. Funds
 * the payer's base/quote ATAs (4× reserve) so a later swap has headroom. Returns
 * the `Amm` PDA. Mirrors `challenge_e2e.rs::build_pool`. */
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

/** A genuine BUY (quote in, base out) that pushes the pool's price UP. Warps 5
 * slots first (mirror `swap_buy`'s `warp_slots(0, 5)`). */
async function swapBuy(f: Fixture, baseMint: Address, quoteMint: Address, amountIn: bigint): Promise<void> {
  // A generous forward jump (surfnet_timeTravel rejects tiny increments that
  // land at/under its internal slot with "Internal error"); the cranks below
  // weight the post-swap price into the slot-weighted TWAP regardless.
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

/** Advance ≥ ONE_MINUTE_IN_SLOTS (150) slots, then `crank_that_twap` once
 * (mirror `crank_pool`'s `warp_slots(0, 300)`). */
async function crankPool(f: Fixture, amm: Address): Promise<void> {
  await advanceSlots(f, 300);
  await sendIx(f, await ammV04.crankThatTwap({ amm }), [], 400_000);
}

/** Decode the v0.4 `Amm` TWAP fields + compute `get_twap()` (offsets from
 * `cpi/metadao.rs`: created_at @9, last_updated @131, aggregator(u128) @171,
 * start_delay @219). */
function decodeAmmTwap(data: Uint8Array): {
  createdAt: bigint;
  lastUpdated: bigint;
  aggregator: bigint;
  startDelay: bigint;
  twap: bigint;
} {
  const dv = new DataView(data.buffer, data.byteOffset, data.length);
  const u128 = (off: number): bigint => dv.getBigUint64(off, true) | (dv.getBigUint64(off + 8, true) << 64n);
  const createdAt = dv.getBigUint64(9, true);
  const lastUpdated = dv.getBigUint64(131, true);
  const aggregator = u128(171);
  const startDelay = dv.getBigUint64(219, true);
  const slots = lastUpdated - (createdAt + startDelay);
  const twap = slots > 0n && aggregator > 0n ? aggregator / slots : 0n;
  return { createdAt, lastUpdated, aggregator, startDelay, twap };
}

/** Read `Question.payout_numerators[0..2]` (after the u32 Vec length @72; the two
 * u32 numerators at @76, @80) to confirm the resolution `[pass, fail]`. */
function questionResolution(data: Uint8Array): [number, number] {
  const dv = new DataView(data.buffer, data.byteOffset, data.length);
  return [dv.getUint32(76, true), dv.getUint32(80, true)];
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

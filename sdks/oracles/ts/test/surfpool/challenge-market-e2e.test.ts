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
import { Address, Keypair } from "@solana/web3.js";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import {
  decodeAiClaim,
  decodeMarket,
  decodeOracle,
  decodeProposer,
} from "../../src/accounts/index.js";
import { futarchy } from "../../src/index.js";
import { EXTERNAL_PROGRAM_IDS, Phase, TOKEN_PROGRAM_ID } from "../../src/constants.js";
import { initProtocol, setGovernance } from "../../src/instructions/index.js";
import * as pda from "../../src/pda.js";

import { SurfpoolHarness, mintBytes, toHex } from "./harness.js";
import { buildDaoBlob } from "./futarchy-dao.js";

import {
  ENABLED,
  type Fixture,
  KASS_PRICE_TWAP,
  KASS_PRICE_SCALE,
  BOND,
  BASE_RESERVE,
  QUOTE_NEUTRAL,
  VLTX,
  AMM_ID,
  FUTARCHY_ID,
  BPF_UPGRADEABLE,
  sendIx,
  fetchAccount,
  tokenBalance,
  fabricateAmmOwned,
  decodeAmmTwap,
  questionResolution,
} from "./challenge-market-harness.js";
import {
  frontDoorToChallenge,
  composeMarket,
  composeQuestion,
  openChallengeReal,
  settleChallengeReal,
  buildPool,
  swapBuy,
  crankPool,
} from "./challenge-market-flow.js";

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

/**
 * I1 surfpool SETTLEMENT-TAIL E2E (GATED) — the 6 claim/close/sweep builders.
 *
 * The dispute/challenge/finalize builders are already driven through the real
 * program over RPC by `lifecycle-e2e.test.ts` / `challenge-market-e2e.test.ts`.
 * This suite closes the last SDK-live-coverage gap: it drives the SETTLEMENT
 * builders — `claimProposer` / `claimFact` / `claimFactVote` / `closeAiClaim` /
 * `closeMarket` / `sweepOracle` — through the REAL program over RPC, asserting
 * the on-chain entitlement matrix, the VotersOutstanding ordering, every
 * account close, the grace-gated sweep-to-treasury, and KASS conservation.
 *
 * Two arms (standalone simnet, no fork — settlement touches no MetaDAO):
 *
 *   1. RESOLVED: create → propose×3 (options 0/1/1, conflicting → dispute) →
 *      finalize_proposals → submit_fact×2 (one AGREED, one REJECTED) →
 *      advance_phase → vote_fact×2 → finalize_facts → submit_ai_claim×3
 *      (claims 0/0/1 → plurality option 0) → finalize_ai_claims →
 *      finalize_oracle → Resolved(0). Then EVERY staker claims + closes:
 *        - claim_fact_vote (agreed-approve → stake+reward; rejected-approve →
 *          stake−ceil-slash), claim_fact (agreed → stake+reward; rejected → 0),
 *          respecting the VotersOutstanding ordering (votes first, submitter
 *          last);
 *        - claim_proposer (correct+no-flip → bond+reward; correct+flip →
 *          bond−flip_slash+reward; wrong+surviving → bond, no reward);
 *        - close_ai_claim (each; rent → authority);
 *        - close_market (a SEEDED settled Market + empty escrow — the challenge
 *          settle path needs the forked MetaDAO AMMs, so the Market is fabricated
 *          via surfnet_setAccount and the REAL close_market builder is driven
 *          over RPC; rent → challenger);
 *        - sweep_oracle after the REAL 30-day grace → residual dust → treasury
 *          ATA, stake_vault + Oracle CLOSED, rents → creator.
 *      Conservation: Σ claim payouts + swept dust == vault_initial.
 *
 *   2. INVALID-DEADEND: create → propose×2 (0/1) → … → submit_ai_claim (0/1 →
 *      tie) → finalize_oracle → InvalidDeadend. Claims return the non-slashed
 *      principal (no rewards; reward_pool == 0), close_ai_claim each, then
 *      sweep_oracle after grace drains + closes the vault/oracle.
 *
 * --- real vs seeded (honest split) ---
 * EVERY settlement builder is DRIVEN through the real program over RPC. The
 * dispute core (create → … → finalize_oracle) is fully REAL (only the SPL
 * mints/token accounts are fabricated as canonical byte layouts, exactly as the
 * lifecycle/challenge E2Es do). Two preconditions are SEEDED (documented):
 *   - GOVERNANCE: the sweep treasury is ATA(dao_authority, kass_mint) and
 *     requires Protocol.governance_set. `set_governance` validates the kass_dao
 *     account is futarchy-owned + carries the Dao discriminator and that
 *     dao_authority == the Squads v4 vault PDA. In a standalone simnet the
 *     futarchy program isn't deployed, so the kass_dao account is fabricated
 *     (owner = futarchy id + Dao disc) and set_governance is then driven REAL
 *     (mirrors challenge-market-e2e's governance handoff). The treasury ATA is
 *     fabricated so the sweep Transfer has a destination.
 *   - close_market's SETTLED Market + empty escrow: driving a real settled market
 *     needs the forked MetaDAO conditional-vault + two cranked AMM pools
 *     (covered by challenge-market-e2e). Here the Market/escrow bytes are seeded
 *     and close_market is driven REAL — the on-chain close (owner/type/settled/
 *     escrow-empty guards + the SPL CloseAccount CPI + rent routing) is genuine.
 * The disqualified-proposer claim row (→ 0) needs a real settle_challenge
 * disqualify (forked AMMs) and is covered by challenge-market-e2e (asserts
 * slashed_amount == bond − kass_fee) + the Rust settlement_e2e; here the proposer
 * rows driven are correct+reward, correct+flip-slash+reward, and wrong→bond.
 *
 * GATING: only included when `KASSANDRA_E2E=1` (see `vitest.config.ts`), and
 * skips (not fails) when surfpool / the `.so` are absent.
 */
import { Address, Keypair, Transaction, type TransactionInstruction } from "@solana/web3.js";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { decodeFact, decodeOracle, decodeProposer } from "../../src/accounts/index.js";
import { KASSANDRA_PROGRAM_ID, Phase, TOKEN_PROGRAM_ID, VOTE_APPROVE } from "../../src/constants.js";
import { futarchy } from "../../src/index.js";
import {
  advancePhase,
  claimFact,
  claimFactVote,
  claimProposer,
  closeAiClaim,
  closeMarket,
  createOracle,
  finalizeAiClaims,
  finalizeFacts,
  finalizeOracle,
  finalizeProposals,
  initProtocol,
  propose,
  setGovernance,
  submitAiClaim,
  submitFact,
  sweepOracle,
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

const FUTARCHY_ID = futarchy.FUTARCHY_ID;
/** 30-day sweep grace (config.rs SWEEP_GRACE = 30·24·60·60). */
const SWEEP_GRACE = 30n * 24n * 60n * 60n;

interface Fixture {
  harness: SurfpoolHarness;
  payer: Keypair;
  kassMint: Keypair;
  usdcMint: Keypair;
  daoAuthority: Address;
  treasury: Address;
}

describe.skipIf(!ENABLED)("surfpool settlement tail (claim/close/sweep, real program)", () => {
  let f: Fixture;

  beforeAll(async () => {
    // Dedicated port (8930) so it never collides with smoke (8899) / lifecycle
    // (8901) / challenge (8920).
    const harness = await SurfpoolHarness.start({ port: 8930 });
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
      data: toHex(mintBytes(payer.publicKey.toBytes(), 0n, 6)),
    });

    // --- governance handoff (SEEDED kass_dao, REAL set_governance) ------------
    // set_governance validates: kass_dao owned by the futarchy program + carries
    // the Dao Anchor discriminator, and dao_authority == the Squads v4 vault PDA
    // derived for it. Fabricate the futarchy-owned Dao account (no CPI — only an
    // owner + disc + PDA check), then drive the REAL set_governance.
    const kassDao = (await Keypair.generate()).publicKey;
    const daoBlob = new Uint8Array(256);
    daoBlob.set(futarchy.ACCOUNT_DISC.dao, 0);
    await harness.setAccount(kassDao.toString(), {
      lamports: 5_000_000,
      owner: FUTARCHY_ID.toString(),
      executable: false,
      data: toHex(daoBlob),
    });
    const multisig = (await futarchy.pda.squadsMultisig(kassDao)).address;
    const daoAuthority = (await futarchy.pda.squadsVault(multisig, 0)).address;

    f = { harness, payer, kassMint, usdcMint, daoAuthority, treasury: daoAuthority };

    await sendIx(f, await initProtocol({
      admin: payer.publicKey,
      kassMint: kassMint.publicKey,
      usdcMint: usdcMint.publicKey,
    }));
    await sendIx(f, await setGovernance({ authority: payer.publicKey, daoAuthority, kassDao }));

    // Fabricate the DAO treasury ATA(dao_authority, kass_mint) so the sweep
    // Transfer has a live destination (the program validates the exact address).
    const treasury = (await pda.associatedTokenAccount(daoAuthority, kassMint.publicKey)).address;
    await harness.setAccount(treasury.toString(), {
      lamports: 5_000_000,
      owner: TOKEN_PROGRAM_ID.toString(),
      executable: false,
      data: toHex(tokenAccountBytes(kassMint.publicKey.toBytes(), daoAuthority.toBytes(), 0n)),
    });
    f.treasury = treasury;
  }, 120_000);

  afterAll(async () => {
    await f?.harness.teardown();
  });

  it("RESOLVED: every staker claims (matrix + ordering) → close_ai_claim/close_market → sweep → conservation", async () => {
    const nonce = 1n;
    const oracle = (await pda.oracle(nonce)).address;
    const vault = (await pda.stakeVault(oracle)).address;
    const bond = 1_000n;

    // ---- create → propose×3 (options 0/1/1 → conflict) → finalize_proposals --
    await createOracleReal(f, nonce, 2);
    await openProposals(f, oracle);
    const props: Array<{ authority: Keypair; proposer: Address; option: number }> = [];
    for (const option of [0, 1, 1]) {
      const { authority, proposer } = await proposeRealWithAuthority(f, oracle, option, bond);
      props.push({ authority, proposer, option });
    }
    await advancePastPhaseEnd(f, oracle);
    await sendIx(f, await finalizeProposals({ oracle, proposers: props.map((p) => p.proposer) }));
    expect(decodeOracle(await fetchAccount(f, oracle)).phase).toBe(Phase.FactProposal);

    // ---- submit_fact ×2 (one AGREED, one REJECTED) --------------------------
    // dispute_bond_total == 3·bond == 3000 → 2/3 quorum == 2000.
    const agreedHash = new Uint8Array(32).fill(0x07);
    const rejectedHash = new Uint8Array(32).fill(0x09);
    const agreedSubStake = 300n;
    const rejectedSubStake = 200n;
    const agreedSubmitter = await fundSigner(f);
    const rejectedSubmitter = await fundSigner(f);
    await sendIx(f, await submitFact({
      oracle, submitter: agreedSubmitter.publicKey,
      submitterKass: await fundKass(f, agreedSubmitter.publicKey, 1_000_000n),
      contentHash: agreedHash, stake: agreedSubStake, uri: "ipfs://agreed",
    }), [agreedSubmitter]);
    await sendIx(f, await submitFact({
      oracle, submitter: rejectedSubmitter.publicKey,
      submitterKass: await fundKass(f, rejectedSubmitter.publicKey, 1_000_000n),
      contentHash: rejectedHash, stake: rejectedSubStake, uri: "ipfs://rejected",
    }), [rejectedSubmitter]);
    const agreedFact = (await pda.fact(oracle, agreedHash)).address;
    const rejectedFact = (await pda.fact(oracle, rejectedHash)).address;

    // ---- advance → FactVoting → vote_fact ×2 --------------------------------
    await advancePastPhaseEnd(f, oracle);
    await sendIx(f, await advancePhase({ oracle }));

    const agreedVoteStake = 2_500n; // ≥ 2000 quorum → agreed
    const rejectedVoteStake = 500n; // < 2000, no duplicate → rejected (voter slashed)
    const agreedVoter = await fundSigner(f);
    const rejectedVoter = await fundSigner(f);
    await sendIx(f, await voteFact({
      oracle, fact: agreedFact, voter: agreedVoter.publicKey,
      voterKass: await fundKass(f, agreedVoter.publicKey, 10_000n),
      kind: VOTE_APPROVE, stake: agreedVoteStake,
    }), [agreedVoter]);
    await sendIx(f, await voteFact({
      oracle, fact: rejectedFact, voter: rejectedVoter.publicKey,
      voterKass: await fundKass(f, rejectedVoter.publicKey, 10_000n),
      kind: VOTE_APPROVE, stake: rejectedVoteStake,
    }), [rejectedVoter]);

    // ---- finalize_facts → AiClaim -------------------------------------------
    await advancePastPhaseEnd(f, oracle);
    await sendIx(f, await finalizeFacts({ nonce, kassMint: f.kassMint.publicKey, tail: [agreedFact, rejectedFact] }));
    expect(decodeFact(await fetchAccount(f, agreedFact)).agreed).toBe(true);
    expect(decodeFact(await fetchAccount(f, rejectedFact)).agreed).toBe(false);

    // ---- submit_ai_claim ×3 (claims 0/0/1 → plurality option 0) -------------
    const claimOptions = [0, 0, 1];
    for (let i = 0; i < props.length; i++) {
      await sendIx(f, await submitAiClaim({
        oracle, proposer: props[i].proposer, authority: props[i].authority.publicKey,
        modelId: new Uint8Array(32).fill(0xa1), paramsHash: new Uint8Array(32).fill(0xb2),
        ioHash: new Uint8Array(32).fill(0xc3), option: claimOptions[i],
      }), [props[i].authority]);
    }

    // ---- finalize_ai_claims → Challenge → finalize_oracle → Resolved(0) -----
    await advancePastPhaseEnd(f, oracle);
    await sendIx(f, await finalizeAiClaims({ oracle, proposers: props.map((p) => p.proposer) }));
    expect(decodeOracle(await fetchAccount(f, oracle)).phase).toBe(Phase.Challenge);
    await advancePastPhaseEnd(f, oracle);
    await sendIx(f, await finalizeOracle({ nonce, kassMint: f.kassMint.publicKey, proposers: props.map((p) => p.proposer) }));

    const o = decodeOracle(await fetchAccount(f, oracle));
    expect(o.phase).toBe(Phase.Resolved);
    expect(o.resolvedOption).toBe(0);
    expect(o.bondPool).toBeGreaterThan(0n); // the flipper (P1) funded bond_pool
    const vaultInitial = await tokenBalance(f, vault);
    const [pBucket, fBucket] = rewardBuckets(
      o.rewardPool, o.rewardProposerWeight, o.rewardFactWeight,
      o.totalCorrectProposerStake, o.totalApprovedFactStake,
    );
    let totalClaimed = 0n;

    // ======================= CLAIM (votes-first ordering) ====================
    // AGREED fact: approve-voter (stake+reward) then submitter (stake+reward).
    {
      const expected = agreedVoteStake + factReward(agreedVoteStake, fBucket, o.totalApprovedFactStake);
      const dest = await fundKass(f, agreedVoter.publicKey, 0n);
      await sendIx(f, await claimFactVote({
        nonce, factVote: (await pda.factVote(agreedFact, agreedVoter.publicKey)).address,
        fact: agreedFact, destKass: dest, rentRecipient: agreedVoter.publicKey,
      }));
      expect(await tokenBalance(f, dest)).toBe(expected);
      totalClaimed += expected;
    }
    {
      const fact = decodeFact(await fetchAccount(f, agreedFact));
      const expected = fact.stake + factReward(fact.stake, fBucket, o.totalApprovedFactStake);
      const dest = await fundKass(f, agreedSubmitter.publicKey, 0n);
      await sendIx(f, await claimFact({ nonce, fact: agreedFact, destKass: dest, rentRecipient: agreedSubmitter.publicKey }));
      expect(await tokenBalance(f, dest)).toBe(expected);
      expect(await isClosed(f, agreedFact)).toBe(true);
      totalClaimed += expected;
    }

    // REJECTED fact: approve-voter slashed ceil(stake·num/den); submitter → 0.
    {
      const slash = ceilSlash(rejectedVoteStake, o.factVoteSlashNum, o.factVoteSlashDen);
      const expected = rejectedVoteStake - slash;
      const dest = await fundKass(f, rejectedVoter.publicKey, 0n);
      await sendIx(f, await claimFactVote({
        nonce, factVote: (await pda.factVote(rejectedFact, rejectedVoter.publicKey)).address,
        fact: rejectedFact, destKass: dest, rentRecipient: rejectedVoter.publicKey,
      }));
      expect(await tokenBalance(f, dest)).toBe(expected);
      totalClaimed += expected;
    }
    {
      const dest = await fundKass(f, rejectedSubmitter.publicKey, 0n);
      await sendIx(f, await claimFact({ nonce, fact: rejectedFact, destKass: dest, rentRecipient: rejectedSubmitter.publicKey }));
      expect(await tokenBalance(f, dest)).toBe(0n); // rejected submitter forfeits
      expect(await isClosed(f, rejectedFact)).toBe(true);
      // totalClaimed += 0
    }

    // ---- claim_proposer ×3 (matrix) + close_ai_claim ------------------------
    let sawReward = false;
    let sawFlipSlash = false;
    let sawWrong = false;
    for (const { authority, proposer } of props) {
      const p = decodeProposer(await fetchAccount(f, proposer));
      const base = p.disqualified ? 0n : p.bond - p.slashedAmount;
      const reward = !p.disqualified && p.claimOption === o.resolvedOption
        ? proposerReward(p.bond, pBucket, o.totalCorrectProposerStake) : 0n;
      const expected = base + reward;
      if (reward > 0n) sawReward = true;
      if (p.slashedAmount > 0n) sawFlipSlash = true;
      if (p.claimOption !== o.resolvedOption) sawWrong = true;

      const dest = await fundKass(f, authority.publicKey, 0n);
      await sendIx(f, await claimProposer({ nonce, proposer, destKass: dest, rentRecipient: authority.publicKey }));
      expect(await tokenBalance(f, dest)).toBe(expected);
      expect(await isClosed(f, proposer)).toBe(true);
      totalClaimed += expected;

      const aiClaim = (await pda.aiClaim(oracle, proposer)).address;
      expect(await isClosed(f, aiClaim)).toBe(false);
      await sendIx(f, await closeAiClaim({ oracle, aiClaim, rentRecipient: authority.publicKey }));
      expect(await isClosed(f, aiClaim)).toBe(true);
    }
    // The matrix rows this arm actually exercised.
    expect(sawReward, "correct proposer earned a reward").toBe(true);
    expect(sawFlipSlash, "the flipper was flip-slashed (bond − slash)").toBe(true);
    expect(sawWrong, "a surviving-but-wrong proposer got bond only").toBe(true);

    // ---- close_market (SEEDED settled Market + escrow, REAL close over RPC) --
    const challenger = await fundSigner(f);
    const marketKp = await Keypair.generate();
    const escrow = (await pda.challengeUsdcVault(marketKp.publicKey)).address;
    await f.harness.setAccount(escrow.toString(), {
      lamports: 3_000_000,
      owner: TOKEN_PROGRAM_ID.toString(),
      executable: false,
      data: toHex(tokenAccountBytes(f.usdcMint.publicKey.toBytes(), oracle.toBytes(), 0n)),
    });
    await f.harness.setAccount(marketKp.publicKey.toString(), {
      lamports: 4_000_000,
      owner: KASSANDRA_PROGRAM_ID.toString(),
      executable: false,
      data: toHex(marketBytes(oracle, challenger.publicKey, escrow)),
    });
    const marketRent = (await f.harness.rpc<{ value: number }>("getBalance", [marketKp.publicKey.toString()])).value;
    const escrowRent = (await f.harness.rpc<{ value: number }>("getBalance", [escrow.toString()])).value;
    const chalBefore = (await f.harness.rpc<{ value: number }>("getBalance", [challenger.publicKey.toString()])).value;
    await sendIx(f, await closeMarket({ nonce, market: marketKp.publicKey, rentRecipient: challenger.publicKey }));
    expect(await isClosed(f, marketKp.publicKey)).toBe(true);
    expect(await isClosed(f, escrow)).toBe(true);
    const chalAfter = (await f.harness.rpc<{ value: number }>("getBalance", [challenger.publicKey.toString()])).value;
    expect(chalAfter).toBe(chalBefore + marketRent + escrowRent); // both rents → challenger

    // ---- CONSERVATION: Σ claims + residual dust == vault_initial ------------
    const dust = await tokenBalance(f, vault);
    expect(totalClaimed + dust).toBe(vaultInitial);
    expect(dust).toBeLessThan(8n); // floor/ceil rounding dust only

    // ---- sweep_oracle after the REAL 30-day grace ---------------------------
    const treasuryBefore = await tokenBalance(f, f.treasury);
    await f.harness.advanceToUnix(o.phaseEndsAt + SWEEP_GRACE + 1n);
    await sendIx(f, await sweepOracle({
      nonce, kassMint: f.kassMint.publicKey, daoAuthority: f.daoAuthority, creator: f.payer.publicKey,
    }));
    // Dust routed to the treasury; vault + oracle closed.
    expect(await tokenBalance(f, f.treasury)).toBe(treasuryBefore + dust);
    expect(await isClosed(f, vault)).toBe(true);
    expect(await isClosed(f, oracle)).toBe(true);
  }, 300_000);

  it("INVALID-DEADEND: claims return non-slashed principal → close_ai_claim → sweep drains + closes", async () => {
    const nonce = 2n;
    const oracle = (await pda.oracle(nonce)).address;
    const vault = (await pda.stakeVault(oracle)).address;
    const bond = 1_000n;

    // ---- drive a real dispute to a TIE → InvalidDeadend ---------------------
    await createOracleReal(f, nonce, 2);
    await openProposals(f, oracle);
    const props: Array<{ authority: Keypair; proposer: Address }> = [];
    for (const option of [0, 1]) {
      const { authority, proposer } = await proposeRealWithAuthority(f, oracle, option, bond);
      props.push({ authority, proposer });
    }
    await advancePastPhaseEnd(f, oracle);
    await sendIx(f, await finalizeProposals({ oracle, proposers: props.map((p) => p.proposer) }));

    const hash = new Uint8Array(32).fill(0x07);
    const submitter = await fundSigner(f);
    const subStake = 300n;
    await sendIx(f, await submitFact({
      oracle, submitter: submitter.publicKey,
      submitterKass: await fundKass(f, submitter.publicKey, 1_000_000n),
      contentHash: hash, stake: subStake, uri: "ipfs://fact",
    }), [submitter]);
    const fact = (await pda.fact(oracle, hash)).address;

    await advancePastPhaseEnd(f, oracle);
    await sendIx(f, await advancePhase({ oracle }));

    const voteStake = 2_500n;
    const voter = await fundSigner(f);
    await sendIx(f, await voteFact({
      oracle, fact, voter: voter.publicKey,
      voterKass: await fundKass(f, voter.publicKey, 10_000n),
      kind: VOTE_APPROVE, stake: voteStake,
    }), [voter]);

    await advancePastPhaseEnd(f, oracle);
    await sendIx(f, await finalizeFacts({ nonce, kassMint: f.kassMint.publicKey, tail: [fact] }));

    // Distinct claim options 0/1 → plurality tie → InvalidDeadend.
    for (let i = 0; i < props.length; i++) {
      await sendIx(f, await submitAiClaim({
        oracle, proposer: props[i].proposer, authority: props[i].authority.publicKey,
        modelId: new Uint8Array(32).fill(0xa1), paramsHash: new Uint8Array(32).fill(0xb2),
        ioHash: new Uint8Array(32).fill(0xc3), option: i,
      }), [props[i].authority]);
    }
    await advancePastPhaseEnd(f, oracle);
    await sendIx(f, await finalizeAiClaims({ oracle, proposers: props.map((p) => p.proposer) }));
    await advancePastPhaseEnd(f, oracle);
    await sendIx(f, await finalizeOracle({ nonce, kassMint: f.kassMint.publicKey, proposers: props.map((p) => p.proposer) }));

    const o = decodeOracle(await fetchAccount(f, oracle));
    expect(o.phase).toBe(Phase.InvalidDeadend);
    expect(o.rewardPool).toBe(0n); // no reward distribution out of a dead-end
    const vaultInitial = await tokenBalance(f, vault);
    let totalClaimed = 0n;

    // Fact vote + submitter: full stake back (no reward, no slash on this arm).
    {
      const dest = await fundKass(f, voter.publicKey, 0n);
      await sendIx(f, await claimFactVote({
        nonce, factVote: (await pda.factVote(fact, voter.publicKey)).address,
        fact, destKass: dest, rentRecipient: voter.publicKey,
      }));
      expect(await tokenBalance(f, dest)).toBe(voteStake);
      totalClaimed += voteStake;
    }
    {
      const factStake = decodeFact(await fetchAccount(f, fact)).stake;
      const dest = await fundKass(f, submitter.publicKey, 0n);
      await sendIx(f, await claimFact({ nonce, fact, destKass: dest, rentRecipient: submitter.publicKey }));
      expect(await tokenBalance(f, dest)).toBe(factStake);
      expect(await isClosed(f, fact)).toBe(true);
      totalClaimed += factStake;
    }

    // Proposers: bond − slashed_amount (neither flips here → full bond).
    for (const { authority, proposer } of props) {
      const p = decodeProposer(await fetchAccount(f, proposer));
      const expected = p.bond - p.slashedAmount;
      const dest = await fundKass(f, authority.publicKey, 0n);
      await sendIx(f, await claimProposer({ nonce, proposer, destKass: dest, rentRecipient: authority.publicKey }));
      expect(await tokenBalance(f, dest)).toBe(expected);
      expect(await isClosed(f, proposer)).toBe(true);
      totalClaimed += expected;

      const aiClaim = (await pda.aiClaim(oracle, proposer)).address;
      await sendIx(f, await closeAiClaim({ oracle, aiClaim, rentRecipient: authority.publicKey }));
      expect(await isClosed(f, aiClaim)).toBe(true);
    }

    // Conservation: full returns → vault drained to dust (0 here, no slashes).
    const dust = await tokenBalance(f, vault);
    expect(totalClaimed + dust).toBe(vaultInitial);

    // Sweep after grace drains the residual + closes the vault/oracle.
    const treasuryBefore = await tokenBalance(f, f.treasury);
    await f.harness.advanceToUnix(o.phaseEndsAt + SWEEP_GRACE + 1n);
    await sendIx(f, await sweepOracle({
      nonce, kassMint: f.kassMint.publicKey, daoAuthority: f.daoAuthority, creator: f.payer.publicKey,
    }));
    expect(await tokenBalance(f, f.treasury)).toBe(treasuryBefore + dust);
    expect(await isClosed(f, vault)).toBe(true);
    expect(await isClosed(f, oracle)).toBe(true);
  }, 300_000);
});

// ---------------------------------------------------------------------------
// Reward math (mirrors programs/kassandra/src/reward.rs — floor, u128-safe via
// bigint) + the ceil rejected-fact voter slash (claims.rs::slash_amount).
// ---------------------------------------------------------------------------

function rewardBuckets(pool: bigint, pw: bigint, fw: bigint, tcp: bigint, taf: bigint): [bigint, bigint] {
  if (taf === 0n) return [pool, 0n];
  if (tcp === 0n) return [0n, pool];
  const denom = pw + fw;
  if (denom === 0n) return [pool, 0n];
  return [(pool * pw) / denom, (pool * fw) / denom];
}

function proposerReward(bond: bigint, bucket: bigint, tcp: bigint): bigint {
  return tcp === 0n ? 0n : (bond * bucket) / tcp;
}

function factReward(stake: bigint, bucket: bigint, taf: bigint): bigint {
  return taf === 0n ? 0n : (stake * bucket) / taf;
}

/** `ceil(value·num/den)` — the per-voter rejected-fact slash. */
function ceilSlash(value: bigint, num: bigint, den: bigint): bigint {
  return den === 0n ? 0n : (value * num + den - 1n) / den;
}

// ---------------------------------------------------------------------------
// SEED helpers (documented above) + real-instruction drivers over RPC.
// ---------------------------------------------------------------------------

/** Minimal settled `Market` bytes (416) — the fields close_market validates. */
function marketBytes(oracle: Address, challenger: Address, escrow: Address): Uint8Array {
  const d = new Uint8Array(416);
  d[0] = 6; // AccountType.Market
  d.set(oracle.toBytes(), 8); // oracle @8
  d.set(challenger.toBytes(), 104); // challenger @104
  d.set(escrow.toBytes(), 360); // challenger_usdc_vault @360
  d[408] = 1; // settled @408
  return d;
}

async function sendIx(f: Fixture, ix: TransactionInstruction, signers: Keypair[] = []): Promise<void> {
  const conn = f.harness.connection;
  const tx = new Transaction();
  tx.feePayer = f.payer.publicKey;
  tx.recentBlockhash = (await conn.getLatestBlockhash()).blockhash;
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

/** True once an account is closed (reaped: absent or zero-length). */
async function isClosed(f: Fixture, address: Address): Promise<boolean> {
  const info = await f.harness.connection.getAccountInfo(address);
  return info === null || info.data.length === 0;
}

async function tokenBalance(f: Fixture, address: Address): Promise<bigint> {
  return tokenAccountAmount(await fetchAccount(f, address));
}

async function fundSigner(f: Fixture): Promise<Keypair> {
  const kp = await Keypair.generate();
  await f.harness.airdrop(kp.publicKey.toString(), 2_000_000_000);
  return kp;
}

async function fundKass(f: Fixture, owner: Address, amount: bigint): Promise<Address> {
  const acct = await Keypair.generate();
  await f.harness.setAccount(acct.publicKey.toString(), {
    lamports: 5_000_000,
    owner: TOKEN_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(tokenAccountBytes(f.kassMint.publicKey.toBytes(), owner.toBytes(), amount)),
  });
  return acct.publicKey;
}

async function createOracleReal(f: Fixture, nonce: bigint, optionsCount: number): Promise<void> {
  const creatorKass = await fundKass(f, f.payer.publicKey, 10n ** 15n);
  const nowUnix = await f.harness.clockUnixTimestamp();
  await sendIx(f, await createOracle({
    nonce, optionsCount,
    deadline: nowUnix + 1_000n, twapWindow: 600n,
    creator: f.payer.publicKey, creatorKassToken: creatorKass,
    kassMint: f.kassMint.publicKey, usdcMint: f.usdcMint.publicKey,
  }));
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
  f: Fixture, oracle: Address, option: number, bond: bigint,
): Promise<{ authority: Keypair; proposer: Address }> {
  const authority = await fundSigner(f);
  const authorityKass = await fundKass(f, authority.publicKey, bond * 10n);
  await sendIx(f, await propose({ oracle, authority: authority.publicKey, authorityKass, option, bond }), [authority]);
  const proposer = (await pda.proposer(oracle, authority.publicKey)).address;
  return { authority, proposer };
}

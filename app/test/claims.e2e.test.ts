/**
 * RF2 GATED surfpool CLAIM / CLOSE / SWEEP E2E (`KASSANDRA_E2E=1`).
 *
 * Proves the RF2 payout action layer end-to-end against a REAL surfpool
 * validator: a disputed oracle is driven to Resolved (the SDK setup builders,
 * mirroring `sdks/oracles/ts/test/surfpool/settlement-e2e.test.ts`), then EVERY settlement
 * builder is driven through the REAL program over the app's
 * {@link keypairSender}/{@link sendAndConfirm} seam:
 *
 *   buildClaimProposerIxs  → KASS payout lands in the proposer's canonical ATA,
 *                            Proposer account CLOSED (matrix: reward / flip-slash
 *                            / surviving-but-wrong);
 *   buildClaimFactIxs      → the agreed fact's submitter is paid, Fact CLOSED;
 *   buildClaimFactVoteIxs  → the approve-voter is paid (agreed) / slashed
 *                            (rejected), FactVote CLOSED;
 *   buildCloseAiClaimIxs   → each AiClaim CLOSED, rent → authority;
 *   buildCloseMarketIxs    → a SEEDED settled Market + escrow CLOSED, rent →
 *                            challenger;
 *   buildSweepOracleIxs    → after the REAL 30-day grace, residual dust →
 *                            treasury ATA, stake_vault + Oracle CLOSED.
 *
 * Conservation: Σ claim payouts + swept dust == vault_initial.
 *
 * Unlike the SDK suite (which funds a fresh dest ATA per claim and calls the SDK
 * builder directly) this drives the APP builders, which derive the participant's
 * CANONICAL `ATA(authority, kassMint)` as `destKass` and PREPEND a create-ATA
 * when absent — so the asserted payout lands in that canonical ATA. The
 * participant keypair is the fee-payer + create-ATA payer.
 *
 * GOVERNANCE (sweep precondition) + the SEEDED settled Market are fabricated
 * exactly as the SDK settlement E2E documents (a futarchy-owned kass_dao + REAL
 * set_governance, a treasury ATA, and settled Market/escrow bytes).
 *
 * Gated: skips (never fails) unless `KASSANDRA_E2E=1` AND surfpool + the `.so`
 * are present.
 */
import { Address, Keypair, Transaction, type TransactionInstruction } from "@solana/web3.js";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import {
  Phase,
  TOKEN_PROGRAM_ID,
  VOTE_APPROVE,
  advancePhase,
  associatedTokenAccount,
  createOracle,
  decodeFact,
  decodeOracle,
  decodeProposer,
  finalizeAiClaims,
  finalizeFacts,
  finalizeOracle,
  finalizeProposals,
  futarchy,
  initProtocol,
  propose,
  setGovernance,
  submitAiClaim,
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
  buildClaimFactIxs,
  buildClaimFactVoteIxs,
  buildClaimProposerIxs,
  buildCloseAiClaimIxs,
  buildCloseMarketIxs,
  buildSweepOracleIxs,
} from "../src/data/actions/claims.ts";
import { keypairSender, sendAndConfirm } from "../src/data/send.ts";

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

describe.skipIf(!ENABLED)("RF2 claim/close/sweep action layer over a real surfpool cluster", () => {
  let f: Fixture;

  beforeAll(async () => {
    const harness = await SurfpoolHarness.start({ port: 8931 });
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

    // Fabricate the DAO treasury ATA(dao_authority, kass_mint) — the sweep dest.
    const treasury = (await pda.associatedTokenAccount(daoAuthority, kassMint.publicKey)).address;
    await harness.setAccount(treasury.toString(), {
      lamports: 5_000_000,
      owner: TOKEN_PROGRAM_ID.toString(),
      executable: false,
      data: toHex(tokenAccountBytes(kassMint.publicKey.toBytes(), daoAuthority.toBytes(), 0n)),
    });
    f.treasury = treasury;
  }, 180_000);

  afterAll(async () => {
    await f?.harness.teardown();
  });

  it("drives Resolved → claim_* payouts land + close_* + sweep, asserting conservation", async () => {
    const nonce = 1n;
    const kassMint = f.kassMint.publicKey;
    const oracle = (await pda.oracle(nonce)).address;
    const vault = (await pda.stakeVault(oracle)).address;
    const bond = 1_000n;
    const conn = f.harness.connection;

    // ---- create → propose×3 (0/1/1 → conflict) → finalize_proposals ---------
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
    const agreedVoteStake = 2_500n;
    const rejectedVoteStake = 500n;
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
    await sendIx(f, await finalizeFacts({ nonce, kassMint, tail: [agreedFact, rejectedFact] }));
    expect(decodeFact(await fetchAccount(f, agreedFact)).agreed).toBe(true);
    expect(decodeFact(await fetchAccount(f, rejectedFact)).agreed).toBe(false);

    // ---- submit_ai_claim ×3 (0/0/1 → plurality option 0) --------------------
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
    await sendIx(f, await finalizeOracle({ nonce, kassMint, proposers: props.map((p) => p.proposer) }));

    const o = decodeOracle(await fetchAccount(f, oracle));
    expect(o.phase).toBe(Phase.Resolved);
    expect(o.resolvedOption).toBe(0);
    const vaultInitial = await tokenBalance(f, vault);
    const [pBucket, fBucket] = rewardBuckets(
      o.rewardPool, o.rewardProposerWeight, o.rewardFactWeight,
      o.totalCorrectProposerStake, o.totalApprovedFactStake,
    );
    let totalClaimed = 0n;

    // ================= CLAIMS via the RF2 app builders + keypairSender =========
    // Each claim's destKass is the participant's CANONICAL ATA (derived +
    // create-ATA prepended by the builder); balance after == the exact payout.

    // AGREED fact: approve-voter (stake+reward) then submitter (stake+reward).
    {
      const expected = agreedVoteStake + factReward(agreedVoteStake, fBucket, o.totalApprovedFactStake);
      await claimAs(f, agreedVoter, await buildClaimFactVoteIxs({
        connection: conn, oracleNonce: nonce,
        factVote: (await pda.factVote(agreedFact, agreedVoter.publicKey)).address,
        fact: agreedFact, voter: agreedVoter.publicKey, kassMint,
      }));
      expect(await ataBalance(f, agreedVoter.publicKey)).toBe(expected);
      totalClaimed += expected;
    }
    {
      const fact = decodeFact(await fetchAccount(f, agreedFact));
      const expected = fact.stake + factReward(fact.stake, fBucket, o.totalApprovedFactStake);
      await claimAs(f, agreedSubmitter, await buildClaimFactIxs({
        connection: conn, oracleNonce: nonce, fact: agreedFact,
        authority: agreedSubmitter.publicKey, kassMint,
      }));
      expect(await ataBalance(f, agreedSubmitter.publicKey)).toBe(expected);
      expect(await isClosed(f, agreedFact)).toBe(true);
      totalClaimed += expected;
    }

    // REJECTED fact: approve-voter slashed ceil(stake·num/den); submitter → 0.
    {
      const slash = ceilSlash(rejectedVoteStake, o.factVoteSlashNum, o.factVoteSlashDen);
      const expected = rejectedVoteStake - slash;
      await claimAs(f, rejectedVoter, await buildClaimFactVoteIxs({
        connection: conn, oracleNonce: nonce,
        factVote: (await pda.factVote(rejectedFact, rejectedVoter.publicKey)).address,
        fact: rejectedFact, voter: rejectedVoter.publicKey, kassMint,
      }));
      expect(await ataBalance(f, rejectedVoter.publicKey)).toBe(expected);
      totalClaimed += expected;
    }
    {
      await claimAs(f, rejectedSubmitter, await buildClaimFactIxs({
        connection: conn, oracleNonce: nonce, fact: rejectedFact,
        authority: rejectedSubmitter.publicKey, kassMint,
      }));
      expect(await ataBalance(f, rejectedSubmitter.publicKey)).toBe(0n);
      expect(await isClosed(f, rejectedFact)).toBe(true);
    }

    // ---- claim_proposer ×3 (matrix) + close_ai_claim ------------------------
    let sawReward = false, sawFlipSlash = false, sawWrong = false;
    for (const { authority, proposer } of props) {
      const p = decodeProposer(await fetchAccount(f, proposer));
      const base = p.disqualified ? 0n : p.bond - p.slashedAmount;
      const reward = !p.disqualified && p.claimOption === o.resolvedOption
        ? proposerReward(p.bond, pBucket, o.totalCorrectProposerStake) : 0n;
      const expected = base + reward;
      if (reward > 0n) sawReward = true;
      if (p.slashedAmount > 0n) sawFlipSlash = true;
      if (p.claimOption !== o.resolvedOption) sawWrong = true;

      await claimAs(f, authority, await buildClaimProposerIxs({
        connection: conn, oracleNonce: nonce, proposer, authority: authority.publicKey, kassMint,
      }));
      expect(await ataBalance(f, authority.publicKey)).toBe(expected);
      expect(await isClosed(f, proposer)).toBe(true);
      totalClaimed += expected;

      const aiClaim = (await pda.aiClaim(oracle, proposer)).address;
      await sendAndConfirm(conn, keypairSender(conn, f.payer),
        await buildCloseAiClaimIxs({ oracle, aiClaim, rentRecipient: authority.publicKey }));
      expect(await isClosed(f, aiClaim)).toBe(true);
    }
    expect(sawReward).toBe(true);
    expect(sawFlipSlash).toBe(true);
    expect(sawWrong).toBe(true);

    // ---- close_market (SEEDED settled Market + escrow, REAL close over RPC) --
    const challenger = await fundSigner(f);
    const marketKp = await Keypair.generate();
    const escrow = (await pda.challengeUsdcVault(marketKp.publicKey)).address;
    await f.harness.setAccount(escrow.toString(), {
      lamports: 3_000_000, owner: TOKEN_PROGRAM_ID.toString(), executable: false,
      data: toHex(tokenAccountBytes(f.usdcMint.publicKey.toBytes(), oracle.toBytes(), 0n)),
    });
    await f.harness.setAccount(marketKp.publicKey.toString(), {
      lamports: 4_000_000, owner: pda.KASSANDRA_PROGRAM_ID.toString(), executable: false,
      data: toHex(marketBytes(oracle, challenger.publicKey, escrow)),
    });
    const marketRent = (await f.harness.rpc<{ value: number }>("getBalance", [marketKp.publicKey.toString()])).value;
    const escrowRent = (await f.harness.rpc<{ value: number }>("getBalance", [escrow.toString()])).value;
    const chalBefore = (await f.harness.rpc<{ value: number }>("getBalance", [challenger.publicKey.toString()])).value;
    await sendAndConfirm(conn, keypairSender(conn, f.payer),
      await buildCloseMarketIxs({ oracleNonce: nonce, market: marketKp.publicKey, rentRecipient: challenger.publicKey }));
    expect(await isClosed(f, marketKp.publicKey)).toBe(true);
    expect(await isClosed(f, escrow)).toBe(true);
    const chalAfter = (await f.harness.rpc<{ value: number }>("getBalance", [challenger.publicKey.toString()])).value;
    expect(chalAfter).toBe(chalBefore + marketRent + escrowRent);

    // ---- CONSERVATION -------------------------------------------------------
    const dust = await tokenBalance(f, vault);
    expect(totalClaimed + dust).toBe(vaultInitial);
    expect(dust).toBeLessThan(8n);

    // ---- sweep_oracle after the REAL 30-day grace ---------------------------
    const treasuryBefore = await tokenBalance(f, f.treasury);
    await f.harness.advanceToUnix(o.phaseEndsAt + SWEEP_GRACE + 1n);
    await sendAndConfirm(conn, keypairSender(conn, f.payer), await buildSweepOracleIxs({
      oracleNonce: nonce, kassMint, daoAuthority: f.daoAuthority, creator: f.payer.publicKey,
    }));
    expect(await tokenBalance(f, f.treasury)).toBe(treasuryBefore + dust);
    expect(await isClosed(f, vault)).toBe(true);
    expect(await isClosed(f, oracle)).toBe(true);
  }, 300_000);

  /** Send a participant-signed claim (its keypair is fee-payer + create-ATA payer). */
  async function claimAs(fx: Fixture, signer: Keypair, ixs: TransactionInstruction[]): Promise<void> {
    await sendAndConfirm(fx.harness.connection, keypairSender(fx.harness.connection, signer), ixs);
  }

  /** Balance of an owner's canonical `ATA(owner, kassMint)` (the claim payout dest). */
  async function ataBalance(fx: Fixture, owner: Address): Promise<bigint> {
    const ata = (await associatedTokenAccount(owner, fx.kassMint.publicKey)).address;
    return tokenBalance(fx, ata);
  }
});

// ---------------------------------------------------------------------------
// Reward math (mirrors reward.rs / claims.rs; ported from settlement-e2e).
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
function ceilSlash(value: bigint, num: bigint, den: bigint): bigint {
  return den === 0n ? 0n : (value * num + den - 1n) / den;
}

// ---------------------------------------------------------------------------
// SEED + real-instruction drivers over RPC (ported from settlement-e2e).
// ---------------------------------------------------------------------------
function marketBytes(oracle: Address, challenger: Address, escrow: Address): Uint8Array {
  const d = new Uint8Array(416);
  d[0] = 6; // AccountType.Market
  d.set(oracle.toBytes(), 8);
  d.set(challenger.toBytes(), 104);
  d.set(escrow.toBytes(), 360);
  d[408] = 1; // settled
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
    lamports: 5_000_000, owner: TOKEN_PROGRAM_ID.toString(), executable: false,
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
  return { authority, proposer: (await pda.proposer(oracle, authority.publicKey)).address };
}

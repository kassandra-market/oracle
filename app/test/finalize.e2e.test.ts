/**
 * RF1 GATED surfpool FINALIZE/CRANK E2E (`KASSANDRA_E2E=1`).
 *
 * Proves the finalize action layer end-to-end against a REAL surfpool validator:
 * a funded test KEYPAIR cranks a DISPUTED oracle through EVERY phase transition
 * using the RF1 builders (`buildFinalizeProposalsIxs` → `buildAdvancePhaseIxs` →
 * `buildFinalizeFactsIxs` → `buildFinalizeAiClaimsIxs` → `buildFinalizeOracleIxs`)
 * over the {@link keypairSender}-backed {@link sendAndConfirm} seam, asserting the
 * on-chain phase after each crank via the decoded Oracle:
 *
 *   Proposal --finalizeProposals--> FactProposal --advancePhase--> FactVoting
 *     --finalizeFacts--> AiClaim --finalizeAiClaims--> Challenge
 *     --finalizeOracle--> Resolved (with the AI's option)
 *
 * The setup writes (create / propose×2 conflicting / submit_fact / vote_fact /
 * submit_ai_claim×2) use the SDK builders directly; the AI claims are submitted
 * with fabricated (program-opaque) hashes so the runner binary is NOT required —
 * only surfpool + the built `.so`. Phase windows are crossed with the harness's
 * `surfnet_timeTravel` clock jumps (mirrors the SDK lifecycle E2E).
 *
 * The near-cap v0/ALT finalize path is NOT re-seeded here (it needs a 40-proposer
 * set, each with a funded KASS account) — it is unit-covered (`needsAlt` flips at
 * MAX_LEGACY_TAIL) and proven on real chain by the SDK's own `v0-alt-e2e`.
 *
 * Gated: skips (never fails) unless `KASSANDRA_E2E=1` AND surfpool + the `.so`
 * are present.
 */
import { Keypair, Transaction, type Address, type TransactionInstruction } from "@solana/web3.js";
import {
  Phase,
  TOKEN_PROGRAM_ID,
  VOTE_APPROVE,
  createOracle,
  decodeAiClaim,
  decodeOracle,
  propose,
  submitAiClaim,
  submitFact,
  voteFact,
} from "@kassandra-market/oracles";
import * as pda from "@kassandra-market/oracles";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import {
  SurfpoolHarness,
  mintBytes,
  surfpoolReady,
  toHex,
  tokenAccountBytes,
} from "../../sdks/oracles/ts/test/surfpool/harness.ts";
import {
  buildAdvancePhaseIxs,
  buildFinalizeAiClaimsIxs,
  buildFinalizeFactsIxs,
  buildFinalizeOracleIxs,
  buildFinalizeProposalsIxs,
  type FinalizeAction,
} from "../src/data/actions/finalize.ts";
import { keypairSender, sendAndConfirm } from "../src/data/send.ts";

const ENABLED = process.env.KASSANDRA_E2E === "1" && surfpoolReady();

interface Fixture {
  harness: SurfpoolHarness;
  payer: Keypair;
  kassMint: Keypair;
  usdcMint: Keypair;
}

describe.skipIf(!ENABLED)("finalize/crank action layer over a real surfpool cluster", () => {
  let f: Fixture;
  const nonce = 1n;
  const aiOption = 0; // the option both AI claims resolve the dispute to
  let oracle: Address;
  const contentHash = new Uint8Array(32).fill(0x07);
  let fact: Address;
  let authorities: Keypair[] = [];
  let proposerPdas: Address[] = [];

  beforeAll(async () => {
    const harness = await SurfpoolHarness.start({ port: 8907 });
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

    f = { harness, payer, kassMint, usdcMint };
    await sendIx(
      f,
      await pda.initProtocol({
        admin: payer.publicKey,
        kassMint: kassMint.publicKey,
        usdcMint: usdcMint.publicKey,
      }),
    );

    // create → open proposals → propose×2 CONFLICTING (forces a dispute).
    await createOracleReal(f, nonce, 2);
    oracle = (await pda.oracle(nonce)).address;
    fact = (await pda.fact(oracle, contentHash)).address;
    await openProposals(f, oracle);
    for (const option of [0, 1]) {
      const { authority, proposer } = await proposeReal(f, oracle, option, 1_000n);
      authorities.push(authority);
      proposerPdas.push(proposer);
    }
  }, 180_000);

  afterAll(async () => {
    await f?.harness.teardown();
  });

  /** Send a small (legacy) finalize action through the RF1 keypair-sender seam. */
  async function crank(action: FinalizeAction): Promise<void> {
    expect(action.needsAlt).toBe(false); // small tails → legacy path
    const sender = keypairSender(f.harness.connection, f.payer);
    await sendAndConfirm(f.harness.connection, sender, action.ixs);
  }

  it("drives Proposal → Resolved via the finalize builders, asserting each phase", async () => {
    // --- Proposal --finalizeProposals--> FactProposal ---
    await advancePastPhaseEnd(f, oracle);
    await crank(await buildFinalizeProposalsIxs({ oracle, proposers: proposerPdas }));
    let o = decodeOracle(await fetchAccount(f, oracle));
    expect(o.phase).toBe(Phase.FactProposal);

    // --- submit a fact (setup) ---
    const submitter = await Keypair.generate();
    await f.harness.airdrop(submitter.publicKey.toString(), 2_000_000_000);
    const submitterKass = await fundKass(f, submitter.publicKey, 1_000_000n);
    await sendIx(
      f,
      await submitFact({
        oracle,
        submitter: submitter.publicKey,
        submitterKass,
        contentHash,
        stake: 100n,
        uri: "ipfs://fact",
      }),
      [submitter],
    );

    // --- FactProposal --advancePhase--> FactVoting ---
    await advancePastPhaseEnd(f, oracle);
    await crank(await buildAdvancePhaseIxs({ oracle }));
    o = decodeOracle(await fetchAccount(f, oracle));
    expect(o.phase).toBe(Phase.FactVoting);

    // --- vote to approve the fact (setup) ---
    const voter = await Keypair.generate();
    await f.harness.airdrop(voter.publicKey.toString(), 2_000_000_000);
    const voterKass = await fundKass(f, voter.publicKey, 10_000n);
    await sendIx(
      f,
      await voteFact({
        oracle,
        fact,
        voter: voter.publicKey,
        voterKass,
        kind: VOTE_APPROVE,
        stake: 2_000n,
      }),
      [voter],
    );

    // --- FactVoting --finalizeFacts--> AiClaim ---
    await advancePastPhaseEnd(f, oracle);
    await crank(
      await buildFinalizeFactsIxs({
        oracle,
        kassMint: f.kassMint.publicKey,
        facts: [fact],
        oracleNonce: nonce,
      }),
    );
    o = decodeOracle(await fetchAccount(f, oracle));
    expect(o.phase).toBe(Phase.AiClaim);

    // --- submit an AI claim per proposer (setup; fabricated program-opaque hashes) ---
    for (let i = 0; i < proposerPdas.length; i++) {
      await sendIx(
        f,
        await submitAiClaim({
          oracle,
          proposer: proposerPdas[i],
          authority: authorities[i].publicKey,
          modelId: new Uint8Array(32).fill(0x11),
          paramsHash: new Uint8Array(32).fill(0x22),
          ioHash: new Uint8Array(32).fill(0x33),
          option: aiOption,
        }),
        [authorities[i]],
      );
    }
    const claim = decodeAiClaim(await fetchAccount(f, (await pda.aiClaim(oracle, proposerPdas[0])).address));
    expect(claim.option).toBe(aiOption);

    // --- AiClaim --finalizeAiClaims--> Challenge ---
    await advancePastPhaseEnd(f, oracle);
    await crank(await buildFinalizeAiClaimsIxs({ oracle, proposers: proposerPdas }));
    o = decodeOracle(await fetchAccount(f, oracle));
    expect(o.phase).toBe(Phase.Challenge);

    // --- Challenge --finalizeOracle--> Resolved (AI option) ---
    await advancePastPhaseEnd(f, oracle);
    await crank(
      await buildFinalizeOracleIxs({
        oracle,
        kassMint: f.kassMint.publicKey,
        proposers: proposerPdas,
        oracleNonce: nonce,
      }),
    );
    o = decodeOracle(await fetchAccount(f, oracle));
    expect(o.phase).toBe(Phase.Resolved);
    expect(o.resolvedOption).toBe(aiOption);
  }, 240_000);
});

// ---------------------------------------------------------------------------
// Real-instruction drivers over RPC (mirror the SDK / WF1 surfpool E2Es).
// ---------------------------------------------------------------------------

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

async function fetchAccount(f: Fixture, address: Address, timeoutMs = 15_000): Promise<Uint8Array> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const info = await f.harness.connection.getAccountInfo(address);
    if (info && info.data.length > 0) return info.data;
    await new Promise((r) => setTimeout(r, 150));
  }
  throw new Error(`account ${address} did not appear within ${timeoutMs}ms`);
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

async function proposeReal(
  f: Fixture,
  oracle: Address,
  option: number,
  bond: bigint,
): Promise<{ authority: Keypair; proposer: Address }> {
  const authority = await Keypair.generate();
  await f.harness.airdrop(authority.publicKey.toString(), 2_000_000_000);
  const authorityKass = await fundKass(f, authority.publicKey, bond * 10n);
  await sendIx(
    f,
    await propose({ oracle, authority: authority.publicKey, authorityKass, option, bond }),
    [authority],
  );
  return { authority, proposer: (await pda.proposer(oracle, authority.publicKey)).address };
}

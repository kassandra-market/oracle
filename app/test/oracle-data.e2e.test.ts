/**
 * GATED surfpool integration test for the oracle READ data layer (`KASSANDRA_E2E=1`).
 *
 * Boots a real surfpool simnet, deploys the Kassandra program, `init_protocol`,
 * then SEEDS three oracles in varied phases with REAL instructions over RPC
 * (reusing `sdks/oracles/ts/test/surfpool/harness.ts` + the SDK builders — the same recipe
 * the SDK's own lifecycle E2E uses):
 *
 *   1. nonce 1 — created, left in `Proposal` (no proposers);
 *   2. nonce 2 — create → propose×3 (same option) → finalize_proposals → `Resolved`;
 *   3. nonce 3 (the DETAILED one) — create → propose×2 (conflicting) →
 *      finalize_proposals (→ FactProposal) → submit_fact → advance_phase
 *      (→ FactVoting) → vote_fact → finalize_facts (→ AiClaim) → submit_ai_claim.
 *      Ends in `AiClaim` with 2 proposers + 1 fact + 1 AI claim.
 *
 * Then it drives `fetchOracles` + `fetchOracleDetail` from `src/data/oracles.ts`
 * against the REAL cluster over RPC and asserts the decoded results match the
 * seeded state — proving the data layer against real on-chain bytes. Gated: skips
 * (never fails) unless `KASSANDRA_E2E=1` AND surfpool + the built `.so` are present.
 */
import {
  Keypair,
  Transaction,
  type Address,
  type TransactionInstruction,
} from "@solana/web3.js";
import {
  Phase,
  TOKEN_PROGRAM_ID,
  VOTE_APPROVE,
  advancePhase,
  createOracle,
  decodeOracle,
  finalizeFacts,
  finalizeProposals,
  initProtocol,
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
import { fetchOracleDetail, fetchOracles } from "../src/data/oracles.ts";

const ENABLED = process.env.KASSANDRA_E2E === "1" && surfpoolReady();

interface Fixture {
  harness: SurfpoolHarness;
  payer: Keypair;
  kassMint: Keypair;
  usdcMint: Keypair;
}

describe.skipIf(!ENABLED)("oracle read data layer over a seeded surfpool cluster", () => {
  let f: Fixture;
  // Oracle PDAs by nonce, filled during seeding.
  const oracleAddr: Record<number, string> = {};

  beforeAll(async () => {
    const harness = await SurfpoolHarness.start({ port: 8903 });
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
    await sendIx(f, await initProtocol({
      admin: payer.publicKey,
      kassMint: kassMint.publicKey,
      usdcMint: usdcMint.publicKey,
    }));

    // --- Oracle 1 (nonce 1): created, left in Proposal ---
    await createOracleReal(f, 1n, 2);
    oracleAddr[1] = (await pda.oracle(1n)).address.toString();

    // --- Oracle 2 (nonce 2): uncontested → Resolved ---
    await createOracleReal(f, 2n, 3);
    const o2 = (await pda.oracle(2n)).address;
    oracleAddr[2] = o2.toString();
    await openProposals(f, o2);
    const p2: Address[] = [];
    for (let i = 0; i < 3; i++) p2.push(await proposeReal(f, o2, 1, 5_000n));
    await advancePastPhaseEnd(f, o2);
    await sendIx(f, await finalizeProposals({ oracle: o2, proposers: p2 }));

    // --- Oracle 3 (nonce 3): dispute → AiClaim, with a fact + 2 proposers + 1 claim ---
    await createOracleReal(f, 3n, 2);
    const o3 = (await pda.oracle(3n)).address;
    oracleAddr[3] = o3.toString();
    await openProposals(f, o3);
    const authorities: Keypair[] = [];
    const p3: Address[] = [];
    for (const option of [0, 1]) {
      const { authority, proposer } = await proposeRealWithAuthority(f, o3, option, 1_000n);
      authorities.push(authority);
      p3.push(proposer);
    }
    await advancePastPhaseEnd(f, o3);
    await sendIx(f, await finalizeProposals({ oracle: o3, proposers: p3 }));

    // submit_fact (FactProposal window open)
    const contentHash = new Uint8Array(32).fill(0x07);
    const submitter = await Keypair.generate();
    await f.harness.airdrop(submitter.publicKey.toString(), 2_000_000_000);
    const submitterKass = await fundKass(f, submitter.publicKey, 1_000_000n);
    await sendIx(
      f,
      await submitFact({
        oracle: o3,
        submitter: submitter.publicKey,
        submitterKass,
        contentHash,
        stake: 100n,
        uri: "ipfs://seeded-fact",
      }),
      [submitter],
    );
    const factPda = (await pda.fact(o3, contentHash)).address;

    // advance → advance_phase → FactVoting → vote_fact (approve clears quorum)
    await advancePastPhaseEnd(f, o3);
    await sendIx(f, await advancePhase({ oracle: o3 }));
    const voter = await Keypair.generate();
    await f.harness.airdrop(voter.publicKey.toString(), 2_000_000_000);
    const voterKass = await fundKass(f, voter.publicKey, 10_000n);
    await sendIx(
      f,
      await voteFact({
        oracle: o3,
        fact: factPda,
        voter: voter.publicKey,
        voterKass,
        kind: VOTE_APPROVE,
        stake: 2_000n,
      }),
      [voter],
    );

    // advance → finalize_facts → AiClaim → submit_ai_claim (fabricated metadata)
    await advancePastPhaseEnd(f, o3);
    await sendIx(f, await finalizeFacts({ nonce: 3n, kassMint: f.kassMint.publicKey, tail: [factPda] }));
    await sendIx(
      f,
      await submitAiClaim({
        oracle: o3,
        proposer: p3[0],
        authority: authorities[0].publicKey,
        modelId: new Uint8Array(32).fill(0x11),
        paramsHash: new Uint8Array(32).fill(0x22),
        ioHash: new Uint8Array(32).fill(0x33),
        option: 0,
      }),
      [authorities[0]],
    );
  }, 240_000);

  afterAll(async () => {
    await f?.harness.teardown();
  });

  it("fetchOracles enumerates + decodes all 3 seeded oracles in their phases, sorted by deadline desc", async () => {
    const oracles = await fetchOracles(f.harness.connection);
    expect(oracles.length).toBe(3);

    const byPubkey = new Map(oracles.map((o) => [o.pubkey, o.oracle]));
    expect(byPubkey.get(oracleAddr[1])?.phase).toBe(Phase.Proposal);
    expect(byPubkey.get(oracleAddr[2])?.phase).toBe(Phase.Resolved);
    expect(byPubkey.get(oracleAddr[2])?.resolvedOption).toBe(1);
    expect(byPubkey.get(oracleAddr[3])?.phase).toBe(Phase.AiClaim);

    // Sorted by deadline descending.
    for (let i = 1; i < oracles.length; i++) {
      expect(oracles[i - 1].oracle.deadline >= oracles[i].oracle.deadline).toBe(true);
    }
  }, 30_000);

  it("fetchOracleDetail assembles the disputed oracle's fact + 2 proposers + AI claim", async () => {
    const detail = await fetchOracleDetail(f.harness.connection, oracleAddr[3]);
    expect(detail.oracle.phase).toBe(Phase.AiClaim);
    expect(detail.facts.length).toBe(1);
    expect(detail.facts[0].fact.oracle.toString()).toBe(oracleAddr[3]);
    expect(detail.facts[0].fact.uri).toBe("ipfs://seeded-fact");
    expect(detail.proposers.length).toBe(2);
    expect(new Set(detail.proposers.map((p) => p.proposer.originalOption))).toEqual(new Set([0, 1]));
    expect(detail.aiClaims.length).toBe(1);
    expect(detail.aiClaims[0].aiClaim.option).toBe(0);
    expect(toHex(detail.aiClaims[0].aiClaim.modelId)).toBe("11".repeat(32));
    expect(detail.market).toBeUndefined(); // no challenge opened
  }, 30_000);

  it("fetchOracleDetail returns an oracle with empty child sets", async () => {
    const detail = await fetchOracleDetail(f.harness.connection, oracleAddr[1]);
    expect(detail.oracle.phase).toBe(Phase.Proposal);
    expect(detail.facts).toEqual([]);
    expect(detail.proposers).toEqual([]);
    expect(detail.aiClaims).toEqual([]);
    expect(detail.market).toBeUndefined();
  }, 30_000);
});

// ---------------------------------------------------------------------------
// Real-instruction drivers over RPC (mirrors sdks/oracles/ts/test/surfpool/lifecycle-e2e.ts).
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
  // Distinct deadlines per nonce so fetchOracles' deadline-desc sort is exercised.
  await sendIx(
    f,
    await createOracle({
      nonce,
      optionsCount,
      deadline: nowUnix + 1_000n + nonce * 100n,
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

async function proposeReal(f: Fixture, oracle: Address, option: number, bond: bigint): Promise<Address> {
  return (await proposeRealWithAuthority(f, oracle, option, bond)).proposer;
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
  await sendIx(
    f,
    await propose({ oracle, authority: authority.publicKey, authorityKass, option, bond }),
    [authority],
  );
  const proposer = (await pda.proposer(oracle, authority.publicKey)).address;
  return { authority, proposer };
}

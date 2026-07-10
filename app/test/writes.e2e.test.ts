/**
 * WF1 GATED surfpool WRITE E2E (`KASSANDRA_E2E=1`).
 *
 * Proves the write ACTION layer end-to-end against a REAL surfpool validator: a
 * funded test KEYPAIR drives `buildProposeIxs` / `buildSubmitFactIxs` /
 * `buildVoteFactIxs` through the {@link keypairSender}-backed {@link sendAndConfirm}
 * seam, and each write is asserted to land on-chain by decoding the created
 * account.
 *
 * Recipe (mirrors the SDK/read E2Es — reusing `sdks/oracles/ts/test/surfpool/harness.ts`):
 *   boot + deploy + init_protocol + create an oracle (nonce 1, 2 options);
 *   fund the user KEYPAIR's KASS at its canonical ATA so the action layer uses it.
 *
 *   1. Proposal phase → user `buildProposeIxs(option 0)` → assert a Proposer
 *      (authority == user, originalOption 0, bond).
 *   2. Force a dispute (a 2nd proposer, conflicting option) + finalize_proposals
 *      → FactProposal → user `buildSubmitFactIxs` → assert a Fact
 *      (submitter == user, contentHash, stake, uri).
 *   3. advance_phase → FactVoting → user `buildVoteFactIxs(VOTE_APPROVE)` →
 *      assert a FactVote (voter == user, kindRaw 0, stake).
 *
 * Gated: skips (never fails) unless `KASSANDRA_E2E=1` AND surfpool + the built
 * `.so` are present.
 */
import { Keypair, Transaction, type Address, type TransactionInstruction } from "@solana/web3.js";
import {
  Phase,
  TOKEN_PROGRAM_ID,
  VOTE_APPROVE,
  advancePhase,
  associatedTokenAccount,
  createOracle,
  decodeFact,
  decodeFactVote,
  decodeOracle,
  decodeProposer,
  finalizeProposals,
  initProtocol,
  propose,
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
import { buildProposeIxs, buildSubmitFactIxs, buildVoteFactIxs } from "../src/data/actions.ts";
import { keypairSender, sendAndConfirm } from "../src/data/send.ts";

const ENABLED = process.env.KASSANDRA_E2E === "1" && surfpoolReady();

interface Fixture {
  harness: SurfpoolHarness;
  payer: Keypair;
  kassMint: Keypair;
  usdcMint: Keypair;
}

describe.skipIf(!ENABLED)("write action layer over a real surfpool cluster", () => {
  let f: Fixture;
  let user: Keypair;
  let oracle: Address;
  const nonce = 1n;
  const contentHash = new Uint8Array(32).fill(0x07);
  let factPda: Address;

  beforeAll(async () => {
    const harness = await SurfpoolHarness.start({ port: 8905 });
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

    // The USER keypair the action layer drives: funded SOL + KASS at its
    // CANONICAL ATA (so the action layer's getAccountInfo sees it present + uses
    // it as the bond/stake source — mirrors a real wallet already holding KASS).
    user = await Keypair.generate();
    await harness.airdrop(user.publicKey.toString(), 10_000_000_000);
    const userAta = (await associatedTokenAccount(user.publicKey, kassMint.publicKey)).address;
    await harness.setAccount(userAta.toString(), {
      lamports: 5_000_000,
      owner: TOKEN_PROGRAM_ID.toString(),
      executable: false,
      data: toHex(tokenAccountBytes(kassMint.publicKey.toBytes(), user.publicKey.toBytes(), 1_000_000n)),
    });

    // Create the oracle + open proposals (advance past its deadline).
    await createOracleReal(f, nonce, 2);
    oracle = (await pda.oracle(nonce)).address;
    factPda = (await pda.fact(oracle, contentHash)).address;
    const o = decodeOracle(await fetchAccount(f, oracle));
    await harness.advanceToUnix(o.deadline + 60n);
  }, 180_000);

  afterAll(async () => {
    await f?.harness.teardown();
  });

  it("propose: buildProposeIxs + sendAndConfirm lands a real Proposer on-chain", async () => {
    const bond = 5_000n;
    const sender = keypairSender(f.harness.connection, user);
    const ixs = await buildProposeIxs({
      connection: f.harness.connection,
      oracle,
      kassMint: f.kassMint.publicKey,
      authority: user.publicKey,
      option: 0,
      bond,
      optionsCount: 2,
    });
    const { signature } = await sendAndConfirm(f.harness.connection, sender, ixs);
    expect(signature).toBeTruthy();

    const proposerPda = (await pda.proposer(oracle, user.publicKey)).address;
    const p = decodeProposer(await fetchAccount(f, proposerPda));
    expect(p.authority.toString()).toBe(user.publicKey.toString());
    expect(p.originalOption).toBe(0);
    expect(p.bond).toBe(bond);
  }, 120_000);

  it("submitFact: after a dispute → FactProposal, buildSubmitFactIxs lands a real Fact", async () => {
    // Force a dispute: a 2nd proposer with the CONFLICTING option, then
    // finalize_proposals → FactProposal (a uniform option would resolve instead).
    const other = await proposeConflicting(f, oracle, 1, 5_000n);
    const userProposer = (await pda.proposer(oracle, user.publicKey)).address;
    await advancePastPhaseEnd(f, oracle);
    await sendIx(f, await finalizeProposals({ oracle, proposers: [userProposer, other] }));
    let o = decodeOracle(await fetchAccount(f, oracle));
    expect(o.phase).toBe(Phase.FactProposal);

    const stake = 100n;
    const uri = "ipfs://wf1-fact";
    const sender = keypairSender(f.harness.connection, user);
    const ixs = await buildSubmitFactIxs({
      connection: f.harness.connection,
      oracle,
      kassMint: f.kassMint.publicKey,
      submitter: user.publicKey,
      contentHash,
      stake,
      uri,
    });
    await sendAndConfirm(f.harness.connection, sender, ixs);

    const fact = decodeFact(await fetchAccount(f, factPda));
    expect(fact.oracle.toString()).toBe(oracle.toString());
    expect(fact.proposer.toString()).toBe(user.publicKey.toString());
    expect(toHex(fact.contentHash)).toBe("07".repeat(32));
    expect(fact.stake).toBe(stake);
    expect(fact.uri).toBe(uri);

    o = decodeOracle(await fetchAccount(f, oracle));
    expect(o.factCount).toBe(1);
  }, 120_000);

  it("voteFact: after advance_phase → FactVoting, buildVoteFactIxs lands a real FactVote", async () => {
    await advancePastPhaseEnd(f, oracle);
    await sendIx(f, await advancePhase({ oracle }));
    const o = decodeOracle(await fetchAccount(f, oracle));
    expect(o.phase).toBe(Phase.FactVoting);

    const stake = 2_000n;
    const sender = keypairSender(f.harness.connection, user);
    const ixs = await buildVoteFactIxs({
      connection: f.harness.connection,
      oracle,
      kassMint: f.kassMint.publicKey,
      fact: factPda,
      voter: user.publicKey,
      kind: VOTE_APPROVE,
      stake,
    });
    await sendAndConfirm(f.harness.connection, sender, ixs);

    const votePda = (await pda.factVote(factPda, user.publicKey)).address;
    const vote = decodeFactVote(await fetchAccount(f, votePda));
    expect(vote.fact.toString()).toBe(factPda.toString());
    expect(vote.voter.toString()).toBe(user.publicKey.toString());
    expect(vote.kindRaw).toBe(VOTE_APPROVE);
    expect(vote.stake).toBe(stake);
  }, 120_000);
});

// ---------------------------------------------------------------------------
// Real-instruction drivers over RPC (mirrors the SDK/read surfpool E2Es).
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

async function advancePastPhaseEnd(f: Fixture, oracle: Address): Promise<void> {
  const o = decodeOracle(await fetchAccount(f, oracle));
  await f.harness.advanceToUnix(o.phaseEndsAt + 120n);
}

/** A 2nd proposer with a conflicting option (direct SDK send) → forces a dispute. */
async function proposeConflicting(
  f: Fixture,
  oracle: Address,
  option: number,
  bond: bigint,
): Promise<Address> {
  const authority = await Keypair.generate();
  await f.harness.airdrop(authority.publicKey.toString(), 2_000_000_000);
  const authorityKass = await fundKass(f, authority.publicKey, bond * 10n);
  await sendIx(
    f,
    await propose({ oracle, authority: authority.publicKey, authorityKass, option, bond }),
    [authority],
  );
  return (await pda.proposer(oracle, authority.publicKey)).address;
}

/**
 * T3 surfpool CORE-LIFECYCLE E2E (GATED) — the headline.
 *
 * Drives the FULL Kassandra oracle lifecycle against a REAL surfpool RPC
 * validator, built by the SDK instruction builders, signed with web3.js v3, and
 * sent as real RPC transactions. Phase windows are crossed by advancing the
 * on-chain Clock `unix_timestamp` via the `surfnet_timeTravel({absoluteSlot})`
 * cheatcode (it moves `unix_timestamp` at ~0.4 s/slot — the value the program's
 * `now()` reads, so `now >= phase_ends_at` gates fire). Two arms:
 *
 *   1. UNCONTESTED-RESOLVE: init → create → propose×3 (same option) → advance →
 *      finalize_proposals → Oracle decodes to `Resolved` + the agreed option.
 *
 *   2. DISPUTE → AI-CLAIM (runner in the loop): create → propose×2 (CONFLICTING
 *      options) → finalize_proposals (→ FactProposal) → submit_fact → advance →
 *      advance_phase (→ FactVoting) → vote_fact → advance → finalize_facts
 *      (→ AiClaim) → **invoke the REAL runner** (its genuine AnthropicProvider
 *      against the T2 mock, `setOption(N)`) to PRODUCE the claim metadata →
 *      `submitAiClaimFromRunner` (the SDK bridge) → submit over RPC →
 *      finalize_ai_claims (→ Challenge) → finalize_oracle → Oracle decodes to
 *      `Resolved` with the AI's option, and the on-chain AiClaim decodes to the
 *      runner's exact model_id/params_hash/io_hash/option.
 *
 * This proves the WHOLE path end-to-end: runner(mock AI) → SDK bridge → real
 * program on surfpool → resolved oracle.
 *
 * --- real vs seeded ---
 * EVERYTHING in both arms is driven by REAL instructions over RPC — there is no
 * `setAccount` seeding of any Kassandra program account or phase. The ONLY
 * fabricated state is the SPL plumbing (the KASS/USDC mints + the funded
 * creator/proposer/submitter/voter KASS token accounts), packed as canonical SPL
 * byte layouts and written token-program-owned — exactly as the litesvm
 * `e2e.test.ts` and the Rust `common/mod.rs` harness fund them (the program's own
 * SPL CPIs run against the real Token program). The full phase chain — propose,
 * finalize_proposals, submit_fact, advance_phase, vote_fact, finalize_facts,
 * submit_ai_claim (via the runner bridge), finalize_ai_claims, finalize_oracle —
 * is REAL, with `surfnet_timeTravel` only moving the clock between phases.
 *
 * GATING: only included when `KASSANDRA_E2E=1` (see `vitest.config.ts`), and
 * skips (not fails) when surfpool / the `.so` / the runner binary are absent.
 */
import { Keypair, Transaction, type Address, type TransactionInstruction } from "@solana/web3.js";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { decodeAiClaim, decodeOracle } from "../../src/accounts/index.js";
import { Phase, TOKEN_PROGRAM_ID, VOTE_APPROVE } from "../../src/constants.js";
import {
  advancePhase,
  createOracle,
  finalizeAiClaims,
  finalizeFacts,
  finalizeOracle,
  finalizeProposals,
  initProtocol,
  propose,
  submitFact,
  voteFact,
} from "../../src/instructions/index.js";
import * as pda from "../../src/pda.js";
import { submitAiClaimFromRunner } from "../../src/runner-bridge.js";

import {
  SurfpoolHarness,
  mintBytes,
  surfpoolReady,
  toHex,
  tokenAccountAmount,
  tokenAccountBytes,
} from "./harness.js";
import { MockAnthropic } from "./mock-anthropic.js";
import { runRunner, runnerAvailable, writeRunnerConfig, type RunOutput } from "./run-runner.js";

const ENABLED = process.env.KASSANDRA_E2E === "1" && surfpoolReady() && runnerAvailable();

/** A funded admin/payer + the canonical KASS/USDC mints, shared by both arms. */
interface Fixture {
  harness: SurfpoolHarness;
  payer: Keypair;
  kassMint: Keypair;
  usdcMint: Keypair;
}

describe.skipIf(!ENABLED)("surfpool core lifecycle (runner-in-the-loop, mock AI)", () => {
  let f: Fixture;
  let mock: MockAnthropic;

  beforeAll(async () => {
    // Distinct port from the smoke test (8899) so the two gated surfpool suites
    // never collide if vitest runs them concurrently.
    const harness = await SurfpoolHarness.start({ port: 8901 });
    const payer = await Keypair.generate();
    await harness.airdrop(payer.publicKey.toString(), 1_000_000_000_000);

    // KASS authority = the mint-authority PDA (mirrors the harness bootstrap; not
    // load-bearing — emissions/genesis-fee are 0 so create_oracle mints/burns
    // nothing). USDC authority = payer.
    const mintAuth = await pda.mintAuthority();
    const kassMint = await Keypair.generate();
    const usdcMint = await Keypair.generate();
    // Back the KASS mint with a large supply so create_oracle's dynamic-fee Burn
    // (positive on the 2nd+ oracle in the shared protocol — see the Rust
    // `e2e_second_oracle_fee_is_burned`) does not underflow the mint supply.
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
    mock = await MockAnthropic.start();

    // init_protocol once (the singleton both arms share).
    await sendIx(f, await initProtocol({
      admin: payer.publicKey,
      kassMint: kassMint.publicKey,
      usdcMint: usdcMint.publicKey,
    }));
  }, 90_000);

  afterAll(async () => {
    await mock?.stop();
    await f?.harness.teardown();
  });

  it("uncontested arm: create → propose×3 (same option) → finalize → Resolved", async () => {
    const nonce = 1n;
    const oracle = (await pda.oracle(nonce)).address;
    const agreedOption = 1;
    const bond = 5_000n;

    await createOracleReal(f, nonce, 3);
    await openProposals(f, oracle);

    const proposers: Address[] = [];
    for (let i = 0; i < 3; i++) {
      proposers.push(await proposeReal(f, oracle, agreedOption, bond));
    }

    let o = decodeOracle(await fetchAccount(f, oracle));
    expect(o.phase).toBe(Phase.Proposal);
    expect(o.proposerCount).toBe(3);
    expect(o.totalOracleStake).toBe(bond * 3n);

    await advancePastPhaseEnd(f, oracle);
    await sendIx(f, await finalizeProposals({ oracle, proposers }));

    o = decodeOracle(await fetchAccount(f, oracle));
    expect(o.phase).toBe(Phase.Resolved);
    expect(o.resolvedOption).toBe(agreedOption);
    expect(o.disputeBondTotal).toBe(0n);

    // Stake-vault conservation: it still holds exactly Σ bonds (no CPI on resolve).
    const vault = (await pda.stakeVault(oracle)).address;
    expect(await tokenBalance(f, vault)).toBe(bond * 3n);
  }, 120_000);

  it("dispute → AI-claim arm: runner(mock AI) → bridge → program → Resolved with the AI's option", async () => {
    const nonce = 2n;
    const oracle = (await pda.oracle(nonce)).address;
    const bond = 1_000n;
    const aiOption = 0; // the option the mock AI resolves the dispute to

    // --- create → propose×2 CONFLICTING → finalize_proposals → FactProposal ---
    await createOracleReal(f, nonce, 2);
    await openProposals(f, oracle);

    const authorities: Keypair[] = [];
    const proposerPdas: Address[] = [];
    for (const option of [0, 1]) {
      const { authority, proposer } = await proposeRealWithAuthority(f, oracle, option, bond);
      authorities.push(authority);
      proposerPdas.push(proposer);
    }

    await advancePastPhaseEnd(f, oracle);
    await sendIx(f, await finalizeProposals({ oracle, proposers: proposerPdas }));
    let o = decodeOracle(await fetchAccount(f, oracle));
    expect(o.phase).toBe(Phase.FactProposal);
    expect(o.disputeBondTotal).toBe(bond * 2n);

    // --- submit_fact (FactProposal window still open) ---
    const contentHash = new Uint8Array(32).fill(0x07);
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
    const fact = (await pda.fact(oracle, contentHash)).address;
    o = decodeOracle(await fetchAccount(f, oracle));
    expect(o.factCount).toBe(1);

    // --- advance past FactProposal window → advance_phase → FactVoting ---
    await advancePastPhaseEnd(f, oracle);
    await sendIx(f, await advancePhase({ oracle }));
    o = decodeOracle(await fetchAccount(f, oracle));
    expect(o.phase).toBe(Phase.FactVoting);

    // --- vote_fact: approve 2000 clears 2/3 of dispute_bond_total (2000) ---
    const voter = await Keypair.generate();
    await f.harness.airdrop(voter.publicKey.toString(), 2_000_000_000);
    const voterKass = await fundKass(f, voter.publicKey, 10_000n);
    await sendIx(
      f,
      await voteFact({ oracle, fact, voter: voter.publicKey, voterKass, kind: VOTE_APPROVE, stake: 2_000n }),
      [voter],
    );

    // --- advance past voting window → finalize_facts([fact]) → AiClaim ---
    await advancePastPhaseEnd(f, oracle);
    await sendIx(f, await finalizeFacts({ nonce, kassMint: f.kassMint.publicKey, tail: [fact] }));
    o = decodeOracle(await fetchAccount(f, oracle));
    expect(o.phase).toBe(Phase.AiClaim);

    // ===================== RUNNER IN THE LOOP =====================
    // The mock AI resolves the disputed question to `aiOption`. For EACH proposer
    // we invoke the REAL runner (its genuine AnthropicProvider HTTP+parse path,
    // against the mock) with that proposer in the config so the bridge's
    // claim_pda_seeds cross-check is exercised, then submit the produced claim via
    // the SDK bridge over RPC. Wall-clock time does not move the on-chain clock,
    // so the AiClaim window stays open across the subprocess invocations.
    mock.setOption(aiOption, "claude-opus-4-8");
    let firstOut: RunOutput | undefined;
    for (let i = 0; i < proposerPdas.length; i++) {
      const cfgPath = writeRunnerConfig({
        interpretation: "Resolve the dispute to the option the agreed evidence supports.",
        options_count: 2,
        option_labels: [
          { index: 0, label: "A" },
          { index: 1, label: "B" },
        ],
        facts: [],
        oracle: oracle.toString(),
        proposer: proposerPdas[i].toString(),
      });
      const { code, stdout, stderr } = await runRunner(cfgPath, mock.baseUrl);
      expect(code, `runner failed: ${stderr}`).toBe(0);
      const out = JSON.parse(stdout) as RunOutput;
      expect(out.option_index).toBe(aiOption);
      if (i === 0) firstOut = out;

      // The bridge rebuilds + byte-parity-checks the payload AND cross-checks the
      // runner's claim_pda_seeds against our oracle/proposer.
      const ix = await submitAiClaimFromRunner(out, {
        oracle,
        proposer: proposerPdas[i],
        authority: authorities[i].publicKey,
      });
      await sendIx(f, ix, [authorities[i]]);
    }

    // The on-chain AiClaim (proposer 0) decodes to the runner's EXACT metadata.
    const claimPda = (await pda.aiClaim(oracle, proposerPdas[0])).address;
    const claim = decodeAiClaim(await fetchAccount(f, claimPda));
    expect(claim.option).toBe(aiOption);
    expect(toHex(claim.modelId)).toBe(firstOut!.model_id_hex);
    expect(toHex(claim.paramsHash)).toBe(firstOut!.params_hash_hex);
    expect(toHex(claim.ioHash)).toBe(firstOut!.io_hash_hex);
    expect(claim.oracle.toString()).toBe(oracle.toString());
    expect(claim.proposer.toString()).toBe(proposerPdas[0].toString());
    expect(claim.authority.toString()).toBe(authorities[0].publicKey.toString());

    // --- advance past AiClaim window → finalize_ai_claims → Challenge ---
    await advancePastPhaseEnd(f, oracle);
    await sendIx(f, await finalizeAiClaims({ oracle, proposers: proposerPdas }));
    o = decodeOracle(await fetchAccount(f, oracle));
    expect(o.phase).toBe(Phase.Challenge);

    // --- advance past Challenge window → finalize_oracle → Resolved (AI option) ---
    await advancePastPhaseEnd(f, oracle);
    await sendIx(f, await finalizeOracle({ nonce, kassMint: f.kassMint.publicKey, proposers: proposerPdas }));
    o = decodeOracle(await fetchAccount(f, oracle));
    expect(o.phase).toBe(Phase.Resolved);
    expect(o.resolvedOption).toBe(aiOption);
  }, 180_000);
});

// ---------------------------------------------------------------------------
// Helpers: real-instruction drivers over RPC.
// ---------------------------------------------------------------------------

/** Build, sign (payer + extra signers), send over RPC, and confirm a single ix. */
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

/** Poll `getAccountInfo` until the account exists, returning its raw bytes. */
async function fetchAccount(f: Fixture, address: Address, timeoutMs = 15_000): Promise<Uint8Array> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const info = await f.harness.connection.getAccountInfo(address);
    if (info && info.data.length > 0) return info.data;
    await new Promise((r) => setTimeout(r, 150));
  }
  throw new Error(`account ${address} did not appear within ${timeoutMs}ms`);
}

/** Read an SPL token account's balance over RPC. */
async function tokenBalance(f: Fixture, address: Address): Promise<bigint> {
  return tokenAccountAmount(await fetchAccount(f, address));
}

/** Fabricate a funded KASS token account owned by `owner` (the bond/stake source). */
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

/** create_oracle (real) at `nonce` with a near deadline; oracle opens in Proposal. */
async function createOracleReal(f: Fixture, nonce: bigint, optionsCount: number): Promise<void> {
  // Fund the creator's burn source generously: the dynamic EMA creation fee is 0
  // on the genesis oracle but positive on later ones in the shared protocol.
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

/** Advance the clock past the oracle's `deadline` (proposals open), staying in-window. */
async function openProposals(f: Fixture, oracle: Address): Promise<void> {
  const o = decodeOracle(await fetchAccount(f, oracle));
  await f.harness.advanceToUnix(o.deadline + 60n);
}

/** Advance the clock past the oracle's current `phase_ends_at` (window elapsed). */
async function advancePastPhaseEnd(f: Fixture, oracle: Address): Promise<void> {
  const o = decodeOracle(await fetchAccount(f, oracle));
  await f.harness.advanceToUnix(o.phaseEndsAt + 120n);
}

/** propose (real) from a fresh funded authority; returns the Proposer PDA. */
async function proposeReal(f: Fixture, oracle: Address, option: number, bond: bigint): Promise<Address> {
  return (await proposeRealWithAuthority(f, oracle, option, bond)).proposer;
}

/** propose (real) returning both the authority keypair and the Proposer PDA. */
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

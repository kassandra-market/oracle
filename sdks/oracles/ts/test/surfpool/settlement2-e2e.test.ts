/**
 * I1 surfpool SETTLEMENT-TAIL E2E (GATED) — arm 2: INVALID-DEADEND.
 *
 * Split out of `settlement-e2e.test.ts` (see that file's header for the full
 * suite rationale + the real-vs-seeded split). This arm drives a real dispute
 * to a plurality TIE → InvalidDeadend, then asserts every claim returns the
 * non-slashed principal (no rewards; reward_pool == 0), close_ai_claim each,
 * and a grace-gated sweep_oracle that drains the residual + closes vault/oracle.
 * Shares the fixture setup + drivers via `./settlement-harness.js`.
 *
 * GATING: only included when `KASSANDRA_E2E=1` (see `vitest.config.ts`), and
 * skips (not fails) when surfpool / the `.so` are absent.
 */
import { Address, Keypair } from "@solana/web3.js";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { decodeFact, decodeOracle, decodeProposer } from "../../src/accounts/index.js";
import { Phase, VOTE_APPROVE } from "../../src/constants.js";
import {
  advancePhase,
  claimFact,
  claimFactVote,
  claimProposer,
  closeAiClaim,
  finalizeAiClaims,
  finalizeFacts,
  finalizeOracle,
  finalizeProposals,
  submitAiClaim,
  submitFact,
  sweepOracle,
  voteFact,
} from "../../src/instructions/index.js";
import * as pda from "../../src/pda.js";

import {
  ENABLED,
  type Fixture,
  SWEEP_GRACE,
  advancePastPhaseEnd,
  createOracleReal,
  fetchAccount,
  fundKass,
  fundSigner,
  isClosed,
  openProposals,
  proposeRealWithAuthority,
  sendIx,
  setupFixture,
  tokenBalance,
} from "./settlement-harness.js";

describe.skipIf(!ENABLED)("surfpool settlement tail — invalid-deadend (real program)", () => {
  let f: Fixture;

  beforeAll(async () => {
    f = await setupFixture(8931);
  }, 120_000);

  afterAll(async () => {
    await f?.harness.teardown();
  });

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

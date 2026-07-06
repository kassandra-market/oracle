/**
 * I2 surfpool E2E (GATED) — v0-tx + Address Lookup Table path for near-cap finalizes.
 *
 * PROVES the v0/ALT path removes the documented legacy-tx-overflow limitation:
 *
 *   1. build an oracle with a LARGE proposer set (`PROPOSER_COUNT = 40`,
 *      comfortably past the ~28-proposer legacy overflow threshold and under
 *      MAX_PROPOSERS = 60), all proposing the SAME option (an uncontested
 *      resolve — the cheapest path to a real one-shot `finalize_proposals`);
 *   2. FIRST demonstrate the NEED: building the LEGACY `finalize_proposals` tx
 *      over the full 40-proposer set THROWS a size/overflow error (its compiled
 *      message exceeds the 1232-byte packet);
 *   3. THEN publish an ALT over the 40 proposer PDAs via the I2 helper
 *      (`sendFinalizeViaAlt` → create + chunked extends + slot-wait) and send
 *      the SAME finalize as a v0 tx over the ALT → it SUCCEEDS and the oracle
 *      decodes to `Resolved` with the agreed option.
 *
 * ALT activation needs a slot to pass, so this boots surfpool in `clock`
 * block-production mode with a fast slot-time (mirroring `challenge-market-e2e`);
 * dispute-core time gates still move via `surfnet_timeTravel`. Live-cluster /
 * surfpool only — the ALT path is NOT litesvm-representable.
 *
 * GATING: only when `KASSANDRA_E2E=1` (see `vitest.config.ts`); skips (not
 * fails) when surfpool / the `.so` are absent.
 */
import {
  ComputeBudgetProgram,
  Keypair,
  Transaction,
  type Address,
  type TransactionInstruction,
} from "@solana/web3.js";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { decodeOracle } from "../../src/accounts/index.js";
import { Phase, SYSTEM_PROGRAM_ID, TOKEN_PROGRAM_ID } from "../../src/constants.js";
import {
  createOracle,
  finalizeProposals,
  initProtocol,
  propose,
} from "../../src/instructions/index.js";
import * as pda from "../../src/pda.js";
import { sendFinalizeViaAlt } from "../../src/v0.js";

import {
  SurfpoolHarness,
  mintBytes,
  surfpoolReady,
  toHex,
  tokenAccountBytes,
} from "./harness.js";

const ENABLED = process.env.KASSANDRA_E2E === "1" && surfpoolReady();

/**
 * Proposer-set size driven. 40 is well past the ~28-proposer point where a
 * legacy `finalize_proposals` compiled message exceeds the 1232-byte packet
 * (each key inlines 32 bytes), and under MAX_PROPOSERS = 60. Large enough to
 * PROVE the overflow, small enough to fund + propose reliably on the simnet.
 */
const PROPOSER_COUNT = 40;

interface Fixture {
  harness: SurfpoolHarness;
  payer: Keypair;
  kassMint: Keypair;
  usdcMint: Keypair;
}

describe.skipIf(!ENABLED)("surfpool v0 + ALT near-cap finalize", () => {
  let f: Fixture;

  beforeAll(async () => {
    // `clock` block-production + fast slot-time so the on-chain slot advances
    // over wall-clock — an ALT is only usable one slot AFTER its last extend.
    // Dedicated port (8930) so it never collides with the other gated suites.
    const harness = await SurfpoolHarness.start({
      port: 8930,
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
      data: toHex(mintBytes(payer.publicKey.toBytes(), 0n, 6)),
    });

    f = { harness, payer, kassMint, usdcMint };

    await sendIx(
      f,
      await initProtocol({
        admin: payer.publicKey,
        kassMint: kassMint.publicKey,
        usdcMint: usdcMint.publicKey,
      }),
    );
  }, 120_000);

  afterAll(async () => {
    await f?.harness.teardown();
  });

  it(`legacy finalize OVERFLOWS at ${PROPOSER_COUNT} proposers → v0+ALT finalize SUCCEEDS → Resolved`, async () => {
    const nonce = 1n;
    const oracle = (await pda.oracle(nonce)).address;
    const agreedOption = 1;
    const bond = 1_000n;

    // --- create → propose × PROPOSER_COUNT (all the same option) ---
    await createOracleReal(f, nonce, 2);
    await openProposals(f, oracle);

    const proposerPdas: Address[] = [];
    for (let i = 0; i < PROPOSER_COUNT; i++) {
      proposerPdas.push(await proposeReal(f, oracle, agreedOption, bond));
    }

    let o = decodeOracle(await fetchAccount(f, oracle));
    expect(o.phase).toBe(Phase.Proposal);
    expect(o.proposerCount).toBe(PROPOSER_COUNT);

    await advancePastPhaseEnd(f, oracle);

    // --- (1) demonstrate the NEED: the LEGACY finalize tx overflows the packet ---
    const legacyIx = await finalizeProposals({ oracle, proposers: proposerPdas });
    await expect(buildLegacySerialized(f, legacyIx)).rejects.toThrow();

    // Oracle untouched by the failed (never-sent) legacy attempt.
    o = decodeOracle(await fetchAccount(f, oracle));
    expect(o.phase).toBe(Phase.Proposal);

    // --- (2) the v0 + ALT path: publish an ALT over the proposer PDAs + send v0 ---
    const { signature, lookupTableAccount } = await sendFinalizeViaAlt({
      connection: f.harness.connection,
      payer: f.payer,
      instruction: legacyIx,
      lookupAddresses: proposerPdas,
      // finalize_proposals loops over all 40 proposers — bump CU past the 200k default.
      prependInstructions: [ComputeBudgetProgram.setComputeUnitLimit({ units: 600_000 })],
      confirm: (sig) => f.harness.confirmSignature(sig),
    });
    expect(signature.length).toBeGreaterThan(0);
    // The ALT actually holds the full proposer set.
    expect(lookupTableAccount.state.addresses.length).toBe(PROPOSER_COUNT);

    // --- assert the finalize took effect: Resolved with the agreed option ---
    o = decodeOracle(await fetchAccount(f, oracle));
    expect(o.phase).toBe(Phase.Resolved);
    expect(o.resolvedOption).toBe(agreedOption);
  }, 300_000);
});

// ---------------------------------------------------------------------------
// Helpers (self-contained; mirror lifecycle-e2e's drivers).
// ---------------------------------------------------------------------------

/** Build, sign, send + confirm a single-ix LEGACY tx over RPC. */
async function sendIx(
  f: Fixture,
  ix: TransactionInstruction,
  signers: Keypair[] = [],
): Promise<void> {
  const conn = f.harness.connection;
  const tx = new Transaction();
  tx.feePayer = f.payer.publicKey;
  tx.recentBlockhash = (await conn.getLatestBlockhash()).blockhash;
  tx.add(ix);
  await tx.sign(f.payer, ...signers);
  const sig = await conn.sendRawTransaction(await tx.serialize(), { skipPreflight: false });
  await f.harness.confirmSignature(sig);
}

/** Build + sign + SERIALIZE a legacy tx (serialize throws when it overflows the packet). */
async function buildLegacySerialized(f: Fixture, ix: TransactionInstruction): Promise<Uint8Array> {
  const conn = f.harness.connection;
  const tx = new Transaction();
  tx.feePayer = f.payer.publicKey;
  tx.recentBlockhash = (await conn.getLatestBlockhash()).blockhash;
  tx.add(ix);
  await tx.sign(f.payer);
  return tx.serialize();
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

/** Instantly fund a fresh keypair with SOL (system-owned) — faster than airdrop-poll. */
async function fundSol(f: Fixture, pubkey: Address, lamports: number): Promise<void> {
  await f.harness.setAccount(pubkey.toString(), {
    lamports,
    owner: SYSTEM_PROGRAM_ID.toString(),
    executable: false,
  });
}

/** Fabricate a funded KASS token account owned by `owner` (the bond source). */
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

/** create_oracle (real) at `nonce`; opens in Proposal after the deadline. */
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

/** Advance the clock past `deadline` (proposals open), staying in-window. */
async function openProposals(f: Fixture, oracle: Address): Promise<void> {
  const o = decodeOracle(await fetchAccount(f, oracle));
  await f.harness.advanceToUnix(o.deadline + 60n);
}

/** Advance the clock past the oracle's current `phase_ends_at`. */
async function advancePastPhaseEnd(f: Fixture, oracle: Address): Promise<void> {
  const o = decodeOracle(await fetchAccount(f, oracle));
  await f.harness.advanceToUnix(o.phaseEndsAt + 120n);
}

/** propose (real) from a fresh funded authority; returns the Proposer PDA. */
async function proposeReal(
  f: Fixture,
  oracle: Address,
  option: number,
  bond: bigint,
): Promise<Address> {
  const authority = await Keypair.generate();
  await fundSol(f, authority.publicKey, 2_000_000_000);
  const authorityKass = await fundKass(f, authority.publicKey, bond * 10n);
  await sendIx(
    f,
    await propose({ oracle, authority: authority.publicKey, authorityKass, option, bond }),
    [authority],
  );
  return (await pda.proposer(oracle, authority.publicKey)).address;
}

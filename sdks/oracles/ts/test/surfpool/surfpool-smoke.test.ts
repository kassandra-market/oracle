/**
 * T1 surfpool smoke test (GATED) — the make-or-break proof of the rig.
 *
 * Proves the whole E2E stack works together against a REAL RPC validator:
 *   1. surfpool boots headless;
 *   2. the local `kassandra_oracles_program.so` is deployed at the FIXED program id
 *      via the `surfnet_setAccount` cheatcode AND actually executes;
 *   3. an `initProtocol` instruction BUILT by the SDK is signed with web3.js v3
 *      and sent as a real RPC transaction the program ACCEPTS;
 *   4. the resulting Protocol PDA is fetched over RPC and decoded by the SDK
 *      decoder, with admin/mints matching what we submitted.
 *
 * GATING: this file is only included by vitest when `KASSANDRA_E2E=1` (see
 * `vitest.config.ts`); it additionally SKIPS (not fails) when surfpool or the
 * built `.so` are unavailable. The default `pnpm test` never runs it.
 */
import { Address, Keypair, Transaction } from "@solana/web3.js";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { decodeProtocol } from "../../src/accounts/index.js";
import { AccountType, TOKEN_PROGRAM_ID } from "../../src/constants.js";
import { initProtocol } from "../../src/instructions/index.js";
import * as pda from "../../src/pda.js";

import { SurfpoolHarness, mintBytes, surfpoolReady, toHex } from "./harness.js";

const ENABLED = process.env.KASSANDRA_E2E === "1" && surfpoolReady();

describe.skipIf(!ENABLED)("surfpool smoke: deploy + init_protocol over RPC", () => {
  let harness: SurfpoolHarness;

  beforeAll(async () => {
    harness = await SurfpoolHarness.start();
  }, 60_000);

  afterAll(async () => {
    await harness?.teardown();
  });

  it("deploys the program and accepts an SDK-built init_protocol tx", async () => {
    const conn = harness.connection;

    // A funded admin/payer.
    const payer = await Keypair.generate();
    await harness.airdrop(payer.publicKey.toString(), 5_000_000_000);

    // Canonical KASS + USDC mints, written token-program-owned (init_protocol
    // only requires the recorded mints to be SPL-token-program accounts).
    const kassMint = await Keypair.generate();
    const usdcMint = await Keypair.generate();
    for (const [mint, decimals] of [
      [kassMint, 9],
      [usdcMint, 6],
    ] as const) {
      await harness.setAccount(mint.publicKey.toString(), {
        lamports: 1_000_000_000,
        owner: TOKEN_PROGRAM_ID.toString(),
        executable: false,
        data: toHex(mintBytes(payer.publicKey.toBytes(), 0n, decimals)),
      });
    }

    // Build init_protocol via the SDK, sign with web3.js v3, send over RPC.
    const ix = await initProtocol({
      admin: payer.publicKey,
      kassMint: kassMint.publicKey,
      usdcMint: usdcMint.publicKey,
    });
    const tx = new Transaction();
    tx.feePayer = payer.publicKey;
    tx.recentBlockhash = (await conn.getLatestBlockhash()).blockhash;
    tx.add(ix);
    await tx.sign(payer);

    const sig = await conn.sendRawTransaction(await tx.serialize(), {
      skipPreflight: false,
    });
    expect(sig).toBeTruthy();

    // Wait for the Protocol PDA to materialize over RPC.
    const protocolPda = await pda.protocol();
    const data = await waitForAccount(conn, protocolPda.address, 10_000);

    // Decode with the SDK decoder + assert admin/mints round-trip.
    const p = decodeProtocol(data);
    expect(p.accountType).toBe(AccountType.Protocol);
    expect(p.admin.toString()).toBe(payer.publicKey.toString());
    expect(p.kassMint.toString()).toBe(kassMint.publicKey.toString());
    expect(p.usdcMint.toString()).toBe(usdcMint.publicKey.toString());
    expect(p.governanceSet).toBe(false);
  }, 30_000);
});

/** Poll `getAccountInfo` until the account exists, returning its raw bytes. */
async function waitForAccount(
  conn: SurfpoolHarness["connection"],
  address: Address,
  timeoutMs: number,
): Promise<Uint8Array> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const info = await conn.getAccountInfo(address);
    if (info && info.data.length > 0) return info.data;
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error(`account ${address} did not appear within ${timeoutMs}ms`);
}

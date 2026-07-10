/**
 * surfpool mainnet-fork smoke test (GATED) — the make-or-break proof of the rig.
 *
 * Proves the whole E2E stack works together against a REAL forked RPC validator:
 *   1. surfpool boots headless (mainnet fork);
 *   2. the local `kassandra_markets_program.so` is deployed at the FIXED program id
 *      via the `surfnet_setAccount` cheatcode AND actually executes;
 *   3. an `initConfig` instruction BUILT by the SDK is signed with web3.js v3 and
 *      sent as a real RPC transaction the program ACCEPTS;
 *   4. the resulting Config PDA is fetched over RPC and decoded by the SDK decoder,
 *      with authority/kassMint/minLiquidity matching what we submitted.
 *
 * GATING: only included by vitest when `KASSANDRA_MARKET_E2E=1`; additionally SKIPS
 * (not fails) when surfpool / the built `.so` are unavailable.
 */
import { Keypair } from "@solana/web3.js";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { decodeConfig } from "../../src/accounts/index.js";
import { AccountType } from "../../src/constants.js";
import { initConfig } from "../../src/instructions/index.js";
import * as pda from "../../src/pda.js";

import { MarketSurfpoolHarness, surfpoolReady } from "./harness/index.js";

const ENABLED = process.env.KASSANDRA_MARKET_E2E === "1" && surfpoolReady();
const PORT = 18899;
const MIN_LIQ = 1_000_000_000n; // 1 KASS (9 dp)

describe.skipIf(!ENABLED)("surfpool smoke: deploy + init_config over RPC", () => {
  let harness: MarketSurfpoolHarness;

  beforeAll(async () => {
    harness = await MarketSurfpoolHarness.start({ port: PORT, fork: "mainnet" });
  }, 90_000);

  afterAll(async () => {
    await harness?.teardown();
  });

  it("deploys the program and accepts an SDK-built init_config tx", async () => {
    // A funded payer + a distinct futarchy authority (recorded on the Config).
    const payer = await Keypair.generate();
    await harness.airdrop(payer.publicKey.toString());
    // The payer must be the program's upgrade authority for init_config.
    await harness.setUpgradeAuthority(payer.publicKey);
    const authority = (await Keypair.generate()).publicKey;

    // Canonical KASS mint, written token-program-owned.
    const kassMint = await harness.createMint(9, payer.publicKey);
    const feeDestination = await harness.createTokenAccount(kassMint, authority, 0n);

    // Build init_config via the SDK, sign with web3.js v3, send over RPC.
    const ix = await initConfig({
      payer: payer.publicKey,
      kassMint,
      authority,
      minLiquidity: MIN_LIQ,
      feeBps: 100,
      feeDestination,
    });
    await harness.sendIx(payer, [ix]);

    // Wait for the Config PDA to materialize over RPC, decode + assert round-trip.
    const configPda = (await pda.config()).address;
    const data = await harness.waitForAccount(configPda, 20_000);
    const cfg = decodeConfig(data);
    expect(cfg.accountType).toBe(AccountType.Config);
    expect(cfg.authority.toString()).toBe(authority.toString());
    expect(cfg.kassMint.toString()).toBe(kassMint.toString());
    expect(cfg.minLiquidity).toBe(MIN_LIQ);
  }, 60_000);
});

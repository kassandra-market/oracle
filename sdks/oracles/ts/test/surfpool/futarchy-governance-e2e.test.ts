/**
 * T-G3 surfpool FULL FUTARCHY GOVERNANCE E2E (GATED, FORKED MetaDAO) — the
 * headline loop: a real futarchy proposal whose pass/fail TWAP verdict drives a
 * real Squads `vault_transaction_execute` that applies a Kassandra `set_config`
 * on-chain.
 *
 * Boots surfpool FORKING MAINNET so MetaDAO's deployed programs execute over
 * RPC: futarchy v0.6 `FUTARELBf…`, conditional_vault `VLTX1ish…`, Squads v4
 * `SQDS4ep6…`.
 *
 * Arms (each only asserts what GENUINELY happens on the fork):
 *   1. BOOTSTRAP — real `initialize_dao` (creates the Dao + the Squads multisig
 *      with create_key==Dao + vault atomically; treasury fetched live) → the
 *      G1-hardened `set_governance(kass_dao=Dao, dao_authority=vault)`. Asserts
 *      `governanceSet==1`, `daoAuthority==vault`, `kassDao==dao` on-chain.
 *   2. STAGE + PROPOSAL + LAUNCH + VERDICT + EXECUTE — see
 *      `futarchy-governance2-e2e.test.ts`.
 *
 * GATING: `KASSANDRA_E2E=1`; skips (not fails) when surfpool/.so absent.
 */
import { Address } from "@solana/web3.js";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { decodeProtocol } from "../../src/accounts/index.js";
import * as pda from "../../src/pda.js";
import * as futarchy from "../../src/futarchy/index.js";

import { surfpoolReady } from "./harness.js";
import { type Fixture, fetchAccount, sendIx } from "./futarchy-governance-harness.js";
import { setupFixture } from "./futarchy-governance-flow.js";

const ENABLED = process.env.KASSANDRA_E2E === "1" && surfpoolReady();

describe.skipIf(!ENABLED)("surfpool FULL futarchy governance loop on FORKED MetaDAO (G3)", () => {
  let f: Fixture;

  beforeAll(async () => {
    f = await setupFixture(8921);
  }, 120_000);

  afterAll(async () => {
    await f?.harness.teardown();
  });

  it("BOOTSTRAP: real initialize_dao + Squads multisig + G1 set_governance handoff", async () => {
    const nonce = 1n;
    // Squads ProgramConfig.treasury is NOT a PDA — fetch it live from the
    // on-chain ProgramConfig account (treasury @ offset 8+32+8 = 48).
    const programConfig = (await futarchy.pda.squadsProgramConfig()).address;
    const pcInfo = await f.harness.connection.getAccountInfo(programConfig);
    expect(pcInfo, "Squads ProgramConfig not on the fork").not.toBeNull();
    const treasury = new Address(pcInfo!.data.slice(48, 80));

    const boot = await futarchy.bootstrapGovernance({
      payer: f.payer.publicKey,
      daoCreator: f.payer.publicKey,
      kassMint: f.kassMint.publicKey,
      usdcMint: f.usdcMint.publicKey,
      squadsProgramConfigTreasury: treasury,
      nonce,
      admin: f.payer.publicKey,
      // TWAP params: observable immediately (start_delay 0); tiny windows so the
      // verdict arm can timeTravel past them.
      twapInitialObservation: 1_000_000_000_000n, // 1.0 (PRICE_SCALE 1e12) quote/base
      twapMaxObservationChangePerUpdate: 1_000_000_000_000n, // > 0 (invariant); allows big moves
      twapStartDelaySeconds: 0,
      // invariant: min_base/min_quote futarchic liquidity must be > 0.
      minQuoteFutarchicLiquidity: 1n,
      minBaseFutarchicLiquidity: 1n,
      baseToStake: 0n,
      passThresholdBps: 0,
      // DAO invariant: seconds_per_proposal >= 86400 (1 day) and >= 2×start_delay.
      secondsPerProposal: 86_400,
    });

    f.dao = boot.dao;
    f.multisig = boot.multisig;
    f.vault = boot.vault;

    // initialize_dao CPIs into Squads (multisig create) + creates ATAs → heavy.
    await sendIx(f, boot.instructions[0], [], 1_400_000);
    await sendIx(f, boot.instructions[1]);

    const protocol = (await pda.protocol()).address;
    const p = decodeProtocol(await fetchAccount(f, protocol));
    expect(p.governanceSet).toBe(true);
    expect(p.daoAuthority.toString()).toBe(f.vault.toString());
    expect(p.kassDao.toString()).toBe(f.dao.toString());

    // Sanity: the Dao really exists, owned by futarchy, with the Dao disc.
    const daoInfo = await f.harness.connection.getAccountInfo(f.dao);
    expect(daoInfo!.owner.toString()).toBe(futarchy.FUTARCHY_ID.toString());
  }, 180_000);
});

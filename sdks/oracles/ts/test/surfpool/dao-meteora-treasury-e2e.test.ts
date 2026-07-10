/**
 * D1 — DAO-OWNED, ADMIN-FREE METEORA TREASURY-FEE CLAIM (GATED, FORKED MAINNET).
 *
 * The FIX for the F2a/F2b finding: a futarchy DAO collects its OWN Meteora
 * cp-amm LP fees WITHOUT any MetaDAO admin. The DAO's Squads vault OWNS the
 * Meteora position, and the fee claim is authorized by the DAO's own
 * governance — a real futarchy proposal whose PASS verdict CPI-approves a Squads
 * `vault_transaction` that `invoke_signed`s the cp-amm `claim_position_fee` as
 * the vault, sweeping the fee to the DAO's OWN token accounts. NO
 * `collect_meteora_damm_fees`, NO `metadao_admin` (`tSTp6B6k…`), NO MetaDAO
 * protocol vault (`6awyHMsh…`) anywhere in the flow.
 *
 * Boots surfpool FORKING MAINNET so the deployed programs execute over RPC:
 * futarchy v0.6.1 `FUTAREL…`, conditional_vault `VLTX1ish…`, Squads v4
 * `SQDS4ep6…`, Meteora DAMM v2 (cp-amm) `cpamd…`.
 *
 * Flow (all through the REAL programs, `skipPreflight:false`, confirm-throws):
 *   1. BOOTSTRAP — real `initialize_dao` → the `Dao` + its Squads multisig/vault
 *      (the DAO treasury authority). (Same bootstrap as F2b/G3.)
 *   2. VAULT-OWNED METEORA POSITION — clone a REAL public cp-amm `Config`,
 *      `initialize_pool` with `creator == the Squads vault` so the funded first
 *      position's NFT is minted straight to the vault (cp-amm `creator` is an
 *      unchecked non-signer; `token::authority = creator`), verified by decoding
 *      the position NFT account's authority == the vault. A payer-owned PROBE
 *      position + A→B swaps accrue a nonzero token-B (quote) LP fee; the probe is
 *      checkpointed to DECODE `fee_b_pending > 0` (proof the pool accrues real
 *      fees; the vault position — larger liquidity — accrues more).
 *   3. GOVERNANCE CLAIM — see `dao-meteora-treasury2-e2e.test.ts` (split out so
 *      no single file exceeds ~400 lines). It stages the cp-amm
 *      `claim_position_fee` (owner == the vault, recipients == the DAO's OWN
 *      vault-owned ATAs) as a Squads `vault_transaction_create` → `proposal_
 *      create`, runs a REAL futarchy proposal to a PASS TWAP verdict so
 *      `finalize_proposal` CPI-approves the Squads proposal, then `vault_
 *      transaction_execute` `invoke_signed`s the claim as the vault.
 *   4. ASSERT — the DAO's ATA received the accrued fee (NONZERO delta), the vault
 *      position's `fee_b_pending` cleared to 0, and NO MetaDAO admin/vault appears
 *      in ANY account of the claim / staged message / execute remaining-accounts.
 *
 * GATING: `KASSANDRA_E2E=1`; skips (not fails) when surfpool/.so absent. Forks
 * mainnet → needs network + is slower.
 */
import { afterAll, beforeAll, describe, it } from "vitest";

import type { Fixture } from "./dao-meteora-treasury-harness.js";
import { ENABLED } from "./dao-meteora-treasury-harness.js";
import { bootstrapFlow, positionFlow, startForkFixture } from "./dao-meteora-treasury-flow.js";

describe.skipIf(!ENABLED)("surfpool DAO-owned admin-free Meteora treasury-fee claim on FORKED mainnet (D1)", () => {
  let f: Fixture;

  beforeAll(async () => {
    f = await startForkFixture(8924);
  }, 120_000);

  afterAll(async () => {
    await f?.harness.teardown();
  });

  it("BOOTSTRAP: real initialize_dao → Dao + Squads multisig/vault (the DAO treasury authority)", async () => {
    await bootstrapFlow(f);
  }, 180_000);

  it("DAO's Squads vault OWNS a funded cp-amm position that accrues a nonzero token-B fee", async () => {
    await positionFlow(f);
  }, 300_000);
});

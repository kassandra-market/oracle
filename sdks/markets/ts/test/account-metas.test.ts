/**
 * Account-meta golden guard — pins each instruction's ACCOUNT CONTRACT.
 *
 * The parity/builder tests lock enums, discriminators, sizes, and payload bytes,
 * but a wrong account ORDER or a flipped isSigner/isWritable flag would still slip
 * past CI. This file closes that gap: for every kassandra-market instruction the SDK
 * exposes, it builds the instruction with DISTINCT placeholder addresses (deriving
 * every PDA the builder derives), then asserts the resulting
 * `[role, isSigner, isWritable]` list deep-equals a HARDCODED literal golden — a
 * hand-written frozen snapshot, NOT computed from the builder.
 *
 * The goldens are cross-checked against the program processors' `let [a, b, ..]`
 * destructures (`programs/markets/src/processor/*.rs`) and the CPI metas
 * (`activate.rs` split_metas, `collect_fee.rs` redeem_metas). Where the two SDKs
 * disagreed the PROGRAM wins; the labels + order here are IDENTICAL to the Rust
 * golden in `sdks/oracles/rust/tests/account_metas.rs`, so both SDKs encode ONE contract.
 * Any future account-order/flag drift in either SDK fails this test.
 *
 * This file covers the pre-activation funding-lifecycle instructions; the
 * activation/resolution/close instructions live in `account-metas-activation.test.ts`
 * and the MetaDAO builders in `account-metas-metadao.test.ts`. Shared fixtures
 * live in `helpers/account-metas.ts`.
 */
import { describe, expect, it } from "vitest";

import * as pda from "../src/pda.js";
import {
  cancel,
  contribute,
  createMarket,
  initConfig,
  refund,
  updateConfig,
} from "../src/instructions/market/index.js";
import {
  AUTHORITY,
  CONTRIB_ATA,
  CONTRIBUTOR,
  CREATOR,
  CREATOR_ATA,
  FEE_DEST,
  KASS_MINT,
  label,
  ORACLE,
  PAYER,
  PROGRAMS,
} from "./helpers/account-metas.js";

// ── kassandra-market: funding lifecycle ───────────────────────────────────────

describe("account-meta golden: kassandra-market funding lifecycle", () => {
  it("initConfig — processor init_config.rs destructure", async () => {
    const ix = await initConfig({
      payer: PAYER,
      kassMint: KASS_MINT,
      authority: AUTHORITY,
      minLiquidity: 5n,
      feeBps: 250,
      feeDestination: FEE_DEST,
    });
    const config = await pda.config();
    const programData = await pda.programData();
    expect(
      label(ix, [
        [config.address, "config"],
        [PAYER, "payer"],
        [KASS_MINT, "kassMint"],
        [FEE_DEST, "feeDestination"],
        [programData.address, "programData"],
        ...PROGRAMS,
      ]),
    ).toEqual([
      ["config", false, true],
      ["payer", true, true],
      ["kassMint", false, false],
      ["feeDestination", false, false],
      ["systemProgram", false, false],
      ["programData", false, false],
    ]);
  });

  it("updateConfig — processor update_config.rs destructure", async () => {
    const ix = await updateConfig({
      authority: AUTHORITY,
      minLiquidity: 42n,
      feeBps: 300,
      feeDestination: FEE_DEST,
    });
    const config = await pda.config();
    expect(
      label(ix, [
        [config.address, "config"],
        [AUTHORITY, "authority"],
        [FEE_DEST, "feeDestination"],
      ]),
    ).toEqual([
      ["config", false, true],
      ["authority", true, false],
      ["feeDestination", false, false],
    ]);
  });

  it("createMarket — processor create_market.rs destructure", async () => {
    const outcomeIndex = 0;
    const ix = await createMarket({
      creator: CREATOR,
      oracle: ORACLE,
      kassMint: KASS_MINT,
      creatorKassAta: CREATOR_ATA,
      seedAmount: 1000n,
      outcomeIndex,
    });
    const config = await pda.config();
    const market = await pda.market(ORACLE, outcomeIndex);
    const escrow = await pda.escrow(market.address);
    const contribution = await pda.contribution(market.address, CREATOR);
    expect(
      label(ix, [
        [config.address, "config"],
        [ORACLE, "oracle"],
        [market.address, "market"],
        [escrow.address, "escrow"],
        [KASS_MINT, "kassMint"],
        [CREATOR, "creator"],
        [CREATOR_ATA, "creatorKassAta"],
        [contribution.address, "contribution"],
        ...PROGRAMS,
      ]),
    ).toEqual([
      ["config", false, false],
      ["oracle", false, false],
      ["market", false, true],
      ["escrow", false, true],
      ["kassMint", false, false],
      ["creator", true, true],
      ["creatorKassAta", false, true],
      ["contribution", false, true],
      ["tokenProgram", false, false],
      ["systemProgram", false, false],
    ]);
  });

  it("contribute — processor contribute.rs destructure", async () => {
    const market = await pda.market(ORACLE, 0);
    const ix = await contribute({
      contributor: CONTRIBUTOR,
      market: market.address,
      contributorKassAta: CONTRIB_ATA,
      amount: 777n,
    });
    const escrow = await pda.escrow(market.address);
    const contribution = await pda.contribution(market.address, CONTRIBUTOR);
    expect(
      label(ix, [
        [market.address, "market"],
        [escrow.address, "escrow"],
        [CONTRIBUTOR, "contributor"],
        [CONTRIB_ATA, "contributorKassAta"],
        [contribution.address, "contribution"],
        ...PROGRAMS,
      ]),
    ).toEqual([
      ["market", false, true],
      ["escrow", false, true],
      ["contributor", true, true],
      ["contributorKassAta", false, true],
      ["contribution", false, true],
      ["tokenProgram", false, false],
      ["systemProgram", false, false],
    ]);
  });

  it("cancel — processor cancel.rs destructure", async () => {
    const market = await pda.market(ORACLE, 0);
    const ix = await cancel({ market: market.address, oracle: ORACLE });
    expect(
      label(ix, [
        [market.address, "market"],
        [ORACLE, "oracle"],
      ]),
    ).toEqual([
      ["market", false, true],
      ["oracle", false, false],
    ]);
  });

  it("refund — processor refund.rs destructure", async () => {
    const market = await pda.market(ORACLE, 0);
    const ix = await refund({
      market: market.address,
      contributor: CONTRIBUTOR,
      contributorKassAta: CONTRIB_ATA,
    });
    const escrow = await pda.escrow(market.address);
    const contribution = await pda.contribution(market.address, CONTRIBUTOR);
    expect(
      label(ix, [
        [market.address, "market"],
        [escrow.address, "escrow"],
        [contribution.address, "contribution"],
        [CONTRIB_ATA, "contributorKassAta"],
        [CONTRIBUTOR, "contributor"],
        ...PROGRAMS,
      ]),
    ).toEqual([
      ["market", false, true],
      ["escrow", false, true],
      ["contribution", false, true],
      ["contributorKassAta", false, true],
      ["contributor", false, true],
      ["tokenProgram", false, false],
    ]);
  });
});

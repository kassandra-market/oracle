/**
 * Account-meta golden guard — MetaDAO amm + conditional_vault builders the SDK
 * exposes. Sibling of `account-metas.test.ts` (which pins the kassandra-market
 * instructions); shared fixtures live in `helpers/account-metas.ts`.
 *
 * For each MetaDAO builder the SDK re-exports it builds the instruction with
 * DISTINCT placeholder addresses (deriving every PDA the builder derives), then
 * asserts the resulting `[role, isSigner, isWritable]` list deep-equals a
 * HARDCODED literal golden — a hand-written frozen snapshot, NOT computed from
 * the builder. The labels + order are IDENTICAL to the Rust golden in
 * `sdks/oracles/rust/tests/account_metas.rs`, so both SDKs encode ONE contract.
 */
import { Address } from "@solana/web3.js";
import { describe, expect, it } from "vitest";

import { addLiquidity, crankThatTwap, createAmm, swap } from "../src/metadao/amm.js";
import {
  initializeConditionalVault,
  initializeQuestion,
  mergeTokens,
  redeemTokens,
  resolveQuestion,
  splitTokens,
} from "../src/metadao/vault.js";
import { SwapType } from "../src/metadao/constants.js";
import * as md from "../src/metadao/pda.js";
import {
  A,
  AMM,
  AUTHORITY,
  type Meta,
  label,
  NO_MINT,
  ORACLE,
  PAYER,
  PROGRAMS,
  QUESTION,
  VAULT,
  VAULT_UNDERLYING_ATA,
  YES_MINT,
} from "./helpers/account-metas.js";

// ── MetaDAO builders the SDK exposes ──────────────────────────────────────────

describe("account-meta golden: MetaDAO amm + conditional_vault builders", () => {
  const BASE_MINT = A(40);
  const QUOTE_MINT = A(41);
  const UNDERLYING_MINT = A(42);
  const USER_LP = A(43);
  const USER_UNDERLYING = A(44);
  const USER_YES = A(45);
  const USER_NO = A(46);
  const QUESTION_ID = new Uint8Array(32).fill(47);

  it("createAmm — amm::create_amm (12 accounts)", async () => {
    const ix = await createAmm({
      payer: PAYER,
      baseMint: BASE_MINT,
      quoteMint: QUOTE_MINT,
      twapInitialObservation: 1n,
      twapMaxObservationChangePerUpdate: 2n,
    });
    const amm = (await md.amm(BASE_MINT, QUOTE_MINT)).address;
    const lpMint = (await md.ammLpMint(amm)).address;
    const eventAuth = (await md.ammEventAuthority()).address;
    expect(
      label(ix, [
        [PAYER, "payer"],
        [amm, "amm"],
        [lpMint, "lpMint"],
        [BASE_MINT, "baseMint"],
        [QUOTE_MINT, "quoteMint"],
        [await md.ata(amm, BASE_MINT), "ammVaultBase"],
        [await md.ata(amm, QUOTE_MINT), "ammVaultQuote"],
        [eventAuth, "ammEventAuthority"],
        ...PROGRAMS,
      ]),
    ).toEqual([
      ["payer", true, true],
      ["amm", false, true],
      ["lpMint", false, true],
      ["baseMint", false, false],
      ["quoteMint", false, false],
      ["ammVaultBase", false, true],
      ["ammVaultQuote", false, true],
      ["ataProgram", false, false],
      ["tokenProgram", false, false],
      ["systemProgram", false, false],
      ["ammEventAuthority", false, false],
      ["ammProgram", false, false],
    ]);
  });

  it("addLiquidity — amm::add_liquidity (11 accounts)", async () => {
    const ix = await addLiquidity({
      payer: PAYER,
      baseMint: BASE_MINT,
      quoteMint: QUOTE_MINT,
      quoteAmount: 1n,
      maxBaseAmount: 2n,
      minLpTokens: 3n,
    });
    const amm = (await md.amm(BASE_MINT, QUOTE_MINT)).address;
    const lpMint = (await md.ammLpMint(amm)).address;
    const eventAuth = (await md.ammEventAuthority()).address;
    expect(
      label(ix, [
        [PAYER, "payer"],
        [amm, "amm"],
        [lpMint, "lpMint"],
        [await md.ata(PAYER, lpMint), "userLp"],
        [await md.ata(PAYER, BASE_MINT), "userBase"],
        [await md.ata(PAYER, QUOTE_MINT), "userQuote"],
        [await md.ata(amm, BASE_MINT), "ammVaultBase"],
        [await md.ata(amm, QUOTE_MINT), "ammVaultQuote"],
        [eventAuth, "ammEventAuthority"],
        ...PROGRAMS,
      ]),
    ).toEqual([
      ["payer", true, true],
      ["amm", false, true],
      ["lpMint", false, true],
      ["userLp", false, true],
      ["userBase", false, true],
      ["userQuote", false, true],
      ["ammVaultBase", false, true],
      ["ammVaultQuote", false, true],
      ["tokenProgram", false, false],
      ["ammEventAuthority", false, false],
      ["ammProgram", false, false],
    ]);
  });

  it("swap — amm::swap (9 accounts)", async () => {
    const ix = await swap({
      payer: PAYER,
      baseMint: BASE_MINT,
      quoteMint: QUOTE_MINT,
      swapType: SwapType.Buy,
      inputAmount: 10n,
    });
    const amm = (await md.amm(BASE_MINT, QUOTE_MINT)).address;
    const eventAuth = (await md.ammEventAuthority()).address;
    expect(
      label(ix, [
        [PAYER, "payer"],
        [amm, "amm"],
        [await md.ata(PAYER, BASE_MINT), "userBase"],
        [await md.ata(PAYER, QUOTE_MINT), "userQuote"],
        [await md.ata(amm, BASE_MINT), "ammVaultBase"],
        [await md.ata(amm, QUOTE_MINT), "ammVaultQuote"],
        [eventAuth, "ammEventAuthority"],
        ...PROGRAMS,
      ]),
    ).toEqual([
      ["payer", true, true],
      ["amm", false, true],
      ["userBase", false, true],
      ["userQuote", false, true],
      ["ammVaultBase", false, true],
      ["ammVaultQuote", false, true],
      ["tokenProgram", false, false],
      ["ammEventAuthority", false, false],
      ["ammProgram", false, false],
    ]);
  });

  it("crankThatTwap — amm::crank_that_twap (3 accounts, TS-only)", async () => {
    const ix = await crankThatTwap({ amm: AMM });
    const eventAuth = (await md.ammEventAuthority()).address;
    expect(
      label(ix, [
        [AMM, "amm"],
        [eventAuth, "ammEventAuthority"],
        ...PROGRAMS,
      ]),
    ).toEqual([
      ["amm", false, true],
      ["ammEventAuthority", false, false],
      ["ammProgram", false, false],
    ]);
  });

  it("initializeQuestion — conditional_vault::initialize_question (5 accounts)", async () => {
    const ix = await initializeQuestion({
      payer: PAYER,
      questionId: QUESTION_ID,
      oracle: ORACLE,
      numOutcomes: 2,
    });
    const question = (await md.question(QUESTION_ID, ORACLE, 2)).address;
    const eventAuth = (await md.vaultEventAuthority()).address;
    expect(
      label(ix, [
        [question, "question"],
        [PAYER, "payer"],
        [eventAuth, "cvEventAuthority"],
        ...PROGRAMS,
      ]),
    ).toEqual([
      ["question", false, true],
      ["payer", true, true],
      ["systemProgram", false, false],
      ["cvEventAuthority", false, false],
      ["cvProgram", false, false],
    ]);
  });

  it("initializeConditionalVault — conditional_vault::initialize_conditional_vault (10 + 2 mints)", async () => {
    const ix = await initializeConditionalVault({
      payer: PAYER,
      question: QUESTION,
      underlyingMint: UNDERLYING_MINT,
      numOutcomes: 2,
    });
    const vault = (await md.conditionalVault(QUESTION, UNDERLYING_MINT)).address;
    const vaultUnderlying = await md.ata(vault, UNDERLYING_MINT);
    const eventAuth = (await md.vaultEventAuthority()).address;
    const condMint0 = (await md.conditionalTokenMint(vault, 0)).address;
    const condMint1 = (await md.conditionalTokenMint(vault, 1)).address;
    expect(
      label(ix, [
        [vault, "vault"],
        [QUESTION, "question"],
        [UNDERLYING_MINT, "underlyingMint"],
        [vaultUnderlying, "vaultUnderlyingAta"],
        [PAYER, "payer"],
        [eventAuth, "cvEventAuthority"],
        [condMint0, "condMint0"],
        [condMint1, "condMint1"],
        ...PROGRAMS,
      ]),
    ).toEqual([
      ["vault", false, true],
      ["question", false, false],
      ["underlyingMint", false, false],
      ["vaultUnderlyingAta", false, true],
      ["payer", true, true],
      ["tokenProgram", false, false],
      ["ataProgram", false, false],
      ["systemProgram", false, false],
      ["cvEventAuthority", false, false],
      ["cvProgram", false, false],
      ["condMint0", false, true],
      ["condMint1", false, true],
    ]);
  });

  /** The shared InteractWithVault golden (split/merge/redeem share this list). */
  const INTERACT_GOLDEN: Meta[] = [
    ["question", false, false],
    ["vault", false, true],
    ["vaultUnderlyingAta", false, true],
    // authority is a READONLY signer (matches program split/redeem metas + sdks/oracles/rust).
    ["authority", true, false],
    ["userUnderlyingAta", false, true],
    ["tokenProgram", false, false],
    ["cvEventAuthority", false, false],
    ["cvProgram", false, false],
    ["condMint0", false, true],
    ["condMint1", false, true],
    ["userCond0", false, true],
    ["userCond1", false, true],
  ];

  async function interactEntries(): Promise<Array<[Address, string]>> {
    const eventAuth = (await md.vaultEventAuthority()).address;
    return [
      [QUESTION, "question"],
      [VAULT, "vault"],
      [VAULT_UNDERLYING_ATA, "vaultUnderlyingAta"],
      [AUTHORITY, "authority"],
      [USER_UNDERLYING, "userUnderlyingAta"],
      [eventAuth, "cvEventAuthority"],
      [YES_MINT, "condMint0"],
      [NO_MINT, "condMint1"],
      [USER_YES, "userCond0"],
      [USER_NO, "userCond1"],
      ...PROGRAMS,
    ];
  }

  const interactArgs = {
    question: QUESTION,
    vault: VAULT,
    vaultUnderlyingAta: VAULT_UNDERLYING_ATA,
    authority: AUTHORITY,
    userUnderlyingAta: USER_UNDERLYING,
    conditionalMints: [YES_MINT, NO_MINT],
    userConditionalAtas: [USER_YES, USER_NO],
  };

  it("splitTokens — conditional_vault InteractWithVault (12 accounts)", async () => {
    const ix = await splitTokens({ ...interactArgs, amount: 1n });
    expect(label(ix, await interactEntries())).toEqual(INTERACT_GOLDEN);
  });

  it("mergeTokens — conditional_vault InteractWithVault (12 accounts)", async () => {
    const ix = await mergeTokens({ ...interactArgs, amount: 1n });
    expect(label(ix, await interactEntries())).toEqual(INTERACT_GOLDEN);
  });

  it("redeemTokens — conditional_vault InteractWithVault (12 accounts)", async () => {
    const ix = await redeemTokens(interactArgs);
    expect(label(ix, await interactEntries())).toEqual(INTERACT_GOLDEN);
  });

  it("resolveQuestion — conditional_vault::resolve_question (4 accounts, TS-only)", async () => {
    const ix = await resolveQuestion({
      question: QUESTION,
      oracle: ORACLE,
      payoutNumerators: [1, 0],
    });
    const eventAuth = (await md.vaultEventAuthority()).address;
    expect(
      label(ix, [
        [QUESTION, "question"],
        [ORACLE, "oracle"],
        [eventAuth, "cvEventAuthority"],
        ...PROGRAMS,
      ]),
    ).toEqual([
      ["question", false, true],
      ["oracle", true, false],
      ["cvEventAuthority", false, false],
      ["cvProgram", false, false],
    ]);
  });
});

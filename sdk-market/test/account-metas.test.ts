/**
 * Account-meta golden guard — pins each instruction's ACCOUNT CONTRACT.
 *
 * The parity/builder tests lock enums, discriminators, sizes, and payload bytes,
 * but a wrong account ORDER or a flipped isSigner/isWritable flag would still slip
 * past CI. This file closes that gap: for every kassandra-market instruction AND
 * every MetaDAO builder the SDK exposes, it builds the instruction with DISTINCT
 * placeholder addresses (deriving every PDA the builder derives), then asserts the
 * resulting `[role, isSigner, isWritable]` list deep-equals a HARDCODED literal
 * golden — a hand-written frozen snapshot, NOT computed from the builder.
 *
 * The goldens are cross-checked against the program processors' `let [a, b, ..]`
 * destructures (`programs/kassandra-market/src/processor/*.rs`) and the CPI metas
 * (`activate.rs` split_metas, `collect_fee.rs` redeem_metas). Where the two SDKs
 * disagreed the PROGRAM wins; the labels + order here are IDENTICAL to the Rust
 * golden in `sdk-rs/tests/account_metas.rs`, so both SDKs encode ONE contract.
 * Any future account-order/flag drift in either SDK fails this test.
 */
import { Address, TransactionInstruction } from "@solana/web3.js";
import { describe, expect, it } from "vitest";

import {
  EXTERNAL_PROGRAM_IDS,
  SYSTEM_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "../src/constants.js";
import * as pda from "../src/pda.js";
import {
  activate,
  cancel,
  claimLp,
  closeMarket,
  collectFee,
  contribute,
  createMarket,
  initConfig,
  refund,
  resolveMarket,
  updateConfig,
} from "../src/instructions/market.js";
import { addLiquidity, crankThatTwap, createAmm, swap } from "../src/metadao/amm.js";
import {
  initializeConditionalVault,
  initializeQuestion,
  mergeTokens,
  redeemTokens,
  resolveQuestion,
  splitTokens,
} from "../src/metadao/vault.js";
import { ATA_PROGRAM_ID, SwapType } from "../src/metadao/constants.js";
import * as md from "../src/metadao/pda.js";

const CV_PROGRAM = EXTERNAL_PROGRAM_IDS.conditionalVault;
const AMM_PROGRAM = EXTERNAL_PROGRAM_IDS.ammV04;

/** Deterministic distinct placeholder address (32 bytes of `n`). */
const A = (n: number): Address => new Address(new Uint8Array(32).fill(n));

/** One golden row: the account's role name + its signer/writable flags. */
type Meta = [name: string, isSigner: boolean, isWritable: boolean];

/**
 * Label each account meta by looking its pubkey up in `entries` (a reverse map of
 * `address -> role`). Throws on any unmapped account so a builder that grew/moved a
 * slot fails loudly rather than silently. Returns `[role, isSigner, isWritable]`.
 */
function label(ix: TransactionInstruction, entries: Array<[Address, string]>): Meta[] {
  const rev = new Map(entries.map(([a, n]) => [a.toString(), n]));
  return ix.keys.map((k) => {
    const name = rev.get(k.pubkey.toString());
    if (name === undefined) {
      throw new Error(`unmapped account ${k.pubkey.toString()} in ${ix.keys.length}-key ix`);
    }
    return [name, k.isSigner, k.isWritable];
  });
}

/** The fixed program-id accounts, by role. Spread into per-instruction maps. */
const PROGRAMS: Array<[Address, string]> = [
  [SYSTEM_PROGRAM_ID, "systemProgram"],
  [TOKEN_PROGRAM_ID, "tokenProgram"],
  [ATA_PROGRAM_ID, "ataProgram"],
  [CV_PROGRAM, "cvProgram"],
  [AMM_PROGRAM, "ammProgram"],
];

// Distinct placeholder args reused across the kassandra-market builders.
const PAYER = A(1);
const KASS_MINT = A(2);
const AUTHORITY = A(3);
const ORACLE = A(4);
const CREATOR = A(5);
const CREATOR_ATA = A(6);
const CONTRIBUTOR = A(7);
const CONTRIB_ATA = A(8);
const LP_ATA = A(9);
const QUESTION = A(10);
const FEE_DEST = A(11);
const VAULT = A(12);
const VAULT_UNDERLYING_ATA = A(13);
const YES_MINT = A(14);
const NO_MINT = A(15);
const MARKET_CYES = A(16);
const MARKET_CNO = A(17);
const AMM = A(18);
const LP_MINT = A(19);
const LP_VAULT_ACC = A(20);
const AMM_VAULT_BASE = A(21);
const AMM_VAULT_QUOTE = A(22);
const CV_EVENT_AUTH = A(23);
const AMM_EVENT_AUTH = A(24);

// ── kassandra-market (11 instructions) ────────────────────────────────────────

describe("account-meta golden: kassandra-market (11 instructions)", () => {
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

  it("activate — processor activate.rs destructure (22 accounts)", async () => {
    const market = await pda.market(ORACLE, 0);
    const ix = await activate({
      market: market.address,
      oracle: ORACLE,
      payer: PAYER,
      question: QUESTION,
      vault: VAULT,
      vaultUnderlyingAta: VAULT_UNDERLYING_ATA,
      yesMint: YES_MINT,
      noMint: NO_MINT,
      marketCyes: MARKET_CYES,
      marketCno: MARKET_CNO,
      amm: AMM,
      lpMint: LP_MINT,
      lpVault: LP_VAULT_ACC,
      ammVaultBase: AMM_VAULT_BASE,
      ammVaultQuote: AMM_VAULT_QUOTE,
      cvEventAuthority: CV_EVENT_AUTH,
      ammEventAuthority: AMM_EVENT_AUTH,
    });
    const escrow = await pda.escrow(market.address);
    expect(
      label(ix, [
        [market.address, "market"],
        [ORACLE, "oracle"],
        [PAYER, "payer"],
        [QUESTION, "question"],
        [VAULT, "vault"],
        [VAULT_UNDERLYING_ATA, "vaultUnderlyingAta"],
        [escrow.address, "escrow"],
        [YES_MINT, "yesMint"],
        [NO_MINT, "noMint"],
        [MARKET_CYES, "marketCyes"],
        [MARKET_CNO, "marketCno"],
        [AMM, "amm"],
        [LP_MINT, "lpMint"],
        [LP_VAULT_ACC, "lpVault"],
        [AMM_VAULT_BASE, "ammVaultBase"],
        [AMM_VAULT_QUOTE, "ammVaultQuote"],
        [CV_EVENT_AUTH, "cvEventAuthority"],
        [AMM_EVENT_AUTH, "ammEventAuthority"],
        ...PROGRAMS,
      ]),
    ).toEqual([
      ["market", false, true],
      ["oracle", false, false],
      ["payer", true, true],
      ["question", false, false],
      ["vault", false, true],
      ["vaultUnderlyingAta", false, true],
      ["escrow", false, true],
      ["yesMint", false, true],
      ["noMint", false, true],
      ["marketCyes", false, true],
      ["marketCno", false, true],
      ["amm", false, true],
      ["lpMint", false, true],
      ["lpVault", false, true],
      ["ammVaultBase", false, true],
      ["ammVaultQuote", false, true],
      ["cvEventAuthority", false, false],
      ["cvProgram", false, false],
      ["ammEventAuthority", false, false],
      ["ammProgram", false, false],
      ["tokenProgram", false, false],
      ["systemProgram", false, false],
    ]);
  });

  it("claimLp — processor claim_lp.rs destructure", async () => {
    const market = await pda.market(ORACLE, 0);
    const ix = await claimLp({
      market: market.address,
      contributor: CONTRIBUTOR,
      contributorLpAta: LP_ATA,
    });
    const lpVault = await pda.lpVault(market.address);
    const contribution = await pda.contribution(market.address, CONTRIBUTOR);
    expect(
      label(ix, [
        [market.address, "market"],
        [lpVault.address, "lpVault"],
        [contribution.address, "contribution"],
        [LP_ATA, "contributorLpAta"],
        [CONTRIBUTOR, "contributor"],
        ...PROGRAMS,
      ]),
    ).toEqual([
      ["market", false, true],
      ["lpVault", false, true],
      ["contribution", false, true],
      ["contributorLpAta", false, true],
      ["contributor", false, true],
      ["tokenProgram", false, false],
    ]);
  });

  it("resolveMarket — processor resolve_market.rs destructure", async () => {
    const market = await pda.market(ORACLE, 0);
    const ix = await resolveMarket({
      market: market.address,
      oracle: ORACLE,
      question: QUESTION,
      cvEventAuthority: CV_EVENT_AUTH,
    });
    expect(
      label(ix, [
        [market.address, "market"],
        [ORACLE, "oracle"],
        [QUESTION, "question"],
        [CV_EVENT_AUTH, "cvEventAuthority"],
        ...PROGRAMS,
      ]),
    ).toEqual([
      ["market", false, true],
      ["oracle", false, false],
      ["question", false, true],
      ["cvEventAuthority", false, false],
      ["cvProgram", false, false],
    ]);
  });

  it("collectFee — processor collect_fee.rs destructure (21 accounts)", async () => {
    const market = await pda.market(ORACLE, 0);
    const ix = await collectFee({
      market: market.address,
      feeDestination: FEE_DEST,
      question: QUESTION,
      vault: VAULT,
      vaultUnderlyingAta: VAULT_UNDERLYING_ATA,
      yesMint: YES_MINT,
      noMint: NO_MINT,
      marketCyes: MARKET_CYES,
      marketCno: MARKET_CNO,
      amm: AMM,
      lpMint: LP_MINT,
      lpVault: LP_VAULT_ACC,
      ammVaultBase: AMM_VAULT_BASE,
      ammVaultQuote: AMM_VAULT_QUOTE,
      cvEventAuthority: CV_EVENT_AUTH,
      ammEventAuthority: AMM_EVENT_AUTH,
    });
    const config = await pda.config();
    const escrow = await pda.escrow(market.address);
    expect(
      label(ix, [
        [market.address, "market"],
        [config.address, "config"],
        [FEE_DEST, "feeDestination"],
        [QUESTION, "question"],
        [VAULT, "vault"],
        [VAULT_UNDERLYING_ATA, "vaultUnderlyingAta"],
        [escrow.address, "escrow"],
        [YES_MINT, "yesMint"],
        [NO_MINT, "noMint"],
        [MARKET_CYES, "marketCyes"],
        [MARKET_CNO, "marketCno"],
        [AMM, "amm"],
        [LP_MINT, "lpMint"],
        [LP_VAULT_ACC, "lpVault"],
        [AMM_VAULT_BASE, "ammVaultBase"],
        [AMM_VAULT_QUOTE, "ammVaultQuote"],
        [CV_EVENT_AUTH, "cvEventAuthority"],
        [AMM_EVENT_AUTH, "ammEventAuthority"],
        ...PROGRAMS,
      ]),
    ).toEqual([
      ["market", false, true],
      ["config", false, false],
      ["feeDestination", false, true],
      ["question", false, false],
      ["vault", false, true],
      ["vaultUnderlyingAta", false, true],
      ["escrow", false, true],
      ["yesMint", false, true],
      ["noMint", false, true],
      ["marketCyes", false, true],
      ["marketCno", false, true],
      ["amm", false, true],
      ["lpMint", false, true],
      ["lpVault", false, true],
      ["ammVaultBase", false, true],
      ["ammVaultQuote", false, true],
      ["cvEventAuthority", false, false],
      ["cvProgram", false, false],
      ["ammEventAuthority", false, false],
      ["ammProgram", false, false],
      ["tokenProgram", false, false],
    ]);
  });

  it("closeMarket — processor close_market.rs destructure", async () => {
    const market = await pda.market(ORACLE, 0);
    const ix = await closeMarket({ market: market.address, creator: CREATOR });
    const escrow = await pda.escrow(market.address);
    const cyes = await pda.cyes(market.address);
    const cno = await pda.cno(market.address);
    const lpVault = await pda.lpVault(market.address);
    expect(
      label(ix, [
        [market.address, "market"],
        [CREATOR, "creator"],
        [escrow.address, "escrow"],
        [cyes.address, "cyes"],
        [cno.address, "cno"],
        [lpVault.address, "lpVault"],
        ...PROGRAMS,
      ]),
    ).toEqual([
      ["market", false, true],
      ["creator", false, true],
      ["escrow", false, true],
      ["cyes", false, true],
      ["cno", false, true],
      ["lpVault", false, true],
      ["tokenProgram", false, false],
    ]);
  });
});

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
    // authority is a READONLY signer (matches program split/redeem metas + sdk-rs).
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

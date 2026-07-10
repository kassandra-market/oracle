/**
 * Account-meta golden guard — kassandra-market activation / resolution / close.
 *
 * Sibling of `account-metas.test.ts` (which pins the pre-activation funding
 * lifecycle). Covers the instructions that CPI into MetaDAO (`activate`,
 * `collectFee`) plus `claimLp`, `resolveMarket`, and `closeMarket`. Each builds
 * the instruction with DISTINCT placeholder addresses (deriving every PDA the
 * builder derives), then asserts the resulting `[role, isSigner, isWritable]`
 * list deep-equals a HARDCODED literal golden. The labels + order are IDENTICAL
 * to the Rust golden in `sdks/oracles/rust/tests/account_metas.rs`. Shared
 * fixtures live in `helpers/account-metas.ts`.
 */
import { describe, expect, it } from "vitest";

import * as pda from "../src/pda.js";
import {
  activate,
  claimLp,
  closeMarket,
  collectFee,
  resolveMarket,
} from "../src/instructions/market/index.js";
import {
  AMM,
  AMM_EVENT_AUTH,
  AMM_VAULT_BASE,
  AMM_VAULT_QUOTE,
  CONTRIBUTOR,
  CREATOR,
  CV_EVENT_AUTH,
  FEE_DEST,
  label,
  LP_ATA,
  LP_MINT,
  LP_VAULT_ACC,
  MARKET_CNO,
  MARKET_CYES,
  NO_MINT,
  ORACLE,
  PAYER,
  PROGRAMS,
  QUESTION,
  VAULT,
  VAULT_UNDERLYING_ATA,
  YES_MINT,
} from "./helpers/account-metas.js";

// ── kassandra-market: activation / resolution / close ─────────────────────────

describe("account-meta golden: kassandra-market activation/resolution/close", () => {
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

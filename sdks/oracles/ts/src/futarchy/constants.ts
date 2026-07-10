/**
 * Futarchy v0.6 + Squads v4 + conditional_vault wire constants.
 *
 * Discriminators mirror the binary-validated Rust CPI modules
 * (`programs/kassandra/src/cpi/metadao_v06.rs` for futarchy/Squads,
 * `…/metadao.rs` for the conditional_vault). Account orderings + arg layouts are
 * documented in `./NOTES.md` with their authoritative source. See NOTES.md for
 * the CRITICAL `create_key == Dao PDA` finding.
 */
import { Address } from "@solana/web3.js";

import { EXTERNAL_PROGRAM_IDS } from "../constants.js";

/** futarchy v0.6 governance/proposal program. */
export const FUTARCHY_ID = EXTERNAL_PROGRAM_IDS.futarchyV06;
/** conditional_vault program (v0.4 == v0.6). */
export const CONDITIONAL_VAULT_ID = EXTERNAL_PROGRAM_IDS.conditionalVault;
/** Squads v4 multisig program (the DAO execution-authority host). */
export const SQUADS_V4_ID = EXTERNAL_PROGRAM_IDS.squadsV4;
/** Meteora DAMM v2 (cp-amm). Pinned for completeness; no builders (see NOTES.md). */
export const METEORA_DAMM_V2_ID = EXTERNAL_PROGRAM_IDS.meteoraDammV2;

/**
 * The Squads multisig's second member, hard-coded in futarchy `initialize_dao`
 * (`permissionless_account::id()`), with Initiate|Execute permissions.
 */
export const SQUADS_PERMISSIONLESS_MEMBER = new Address(
  "EP3SoC2SvR3d4c2eXVBvhEMWSr2j3YtoCY3UMiQV7BPD",
);

// ── collect_meteora_damm_fees fixed addresses (v0.6.1) ────────────────────────
// Sourced from the DEPLOYED v0.6.1 handler
// (metaDAOproject/programs@c1000ed `programs/futarchy/src/instructions/collect_meteora_damm_fees.rs`).

/**
 * MetaDAO protocol multisig vault — the AUTHORITY of the `token_a_account` /
 * `token_b_account` fee-recipient ATAs in `collect_meteora_damm_fees`
 * (`metadao_multisig_vault::ID`, enforced by `associated_token::authority`). The
 * DAO's Meteora LP fees are swept HERE, not to the DAO's own vault.
 */
export const METADAO_MULTISIG_VAULT = new Address(
  "6awyHMshBGVjJ3ozdSJdyyDE1CTAXUwrpNMaRGMsb4sf",
);

/**
 * The `admin` signer `collect_meteora_damm_fees` requires under the `production`
 * feature (`metadao_admin::ID`; a non-Squads signer, chosen to stay under the
 * CPI depth limit). Default `admin` for the builder.
 */
export const METADAO_ADMIN = new Address(
  "tSTp6B6kE9o6ZaTmHm2ZwnJBBtgd3x112tapxFhmBEQ",
);

/**
 * Meteora DAMM v2 pool-authority PDA (`[b"pool_authority"]` under the cp-amm
 * program) — hard-coded as `pool_authority::ID` in the collect handler. Equal to
 * the derived cp-amm PDA (verified).
 */
export const DAMM_V2_POOL_AUTHORITY = new Address(
  "HLnpSz9h2S4hiLQ43rnSD9XkcUThA7B8hQMKmDaiTLcC",
);

const d = (bytes: number[]): Uint8Array => Uint8Array.from(bytes);

/** Anchor instruction discriminators — `sha256("global:<name>")[..8]`. */
export const DISC = {
  // futarchy v0.6
  initializeDao: d([0x80, 0xe2, 0x60, 0x5a, 0x27, 0x38, 0x18, 0xc4]),
  initializeProposal: d([0x32, 0x49, 0x9c, 0x62, 0x81, 0x95, 0x15, 0x9e]),
  launchProposal: d([0x10, 0xd3, 0xbd, 0x77, 0xf5, 0x48, 0x00, 0xe5]),
  finalizeProposal: d([0x17, 0x44, 0x33, 0xa7, 0x6d, 0xad, 0xbb, 0xa4]),
  updateDao: d([0x83, 0x48, 0x4b, 0x19, 0x70, 0xd2, 0x6d, 0x02]),
  spotSwap: d([0xa7, 0x61, 0x0c, 0xe7, 0xed, 0x4e, 0xa6, 0xfb]),
  conditionalSwap: d([0xc2, 0x88, 0xdc, 0x59, 0xf2, 0xa9, 0x82, 0x9d]),
  // provide_liquidity (sha256("global:provide_liquidity")[..8]) — v0.6.1 deployed.
  provideLiquidity: d([0x28, 0x6e, 0x6b, 0x74, 0xae, 0x7f, 0x61, 0xcc]),
  // collect_meteora_damm_fees (sha256("global:collect_meteora_damm_fees")[..8]) —
  // v0.6.1 deployed; PINNED from metaDAOproject/programs@c1000ed source + the
  // on-chain Anchor IDL (both agree; NO args). See NOTES.md ("F2a").
  collectMeteoraDammFees: d([0x8b, 0xd4, 0x69, 0x76, 0x7e, 0x36, 0xd6, 0x8f]),
  // conditional_vault
  initializeQuestion: d([0xf5, 0x97, 0x6a, 0xbc, 0x58, 0x2c, 0x41, 0xd4]),
  resolveQuestion: d([0x34, 0x20, 0xe0, 0xb3, 0xb4, 0x08, 0x00, 0xf6]),
  initializeConditionalVault: d([0x25, 0x58, 0xfa, 0xd4, 0x36, 0xda, 0xe3, 0xaf]),
  splitTokens: d([0x4f, 0xc3, 0x74, 0x00, 0x8c, 0xb0, 0x49, 0xb3]),
  mergeTokens: d([0xe2, 0x59, 0xfb, 0x79, 0xe1, 0x82, 0xb4, 0x0e]),
  redeemTokens: d([0xf6, 0x62, 0x86, 0x29, 0x98, 0x21, 0x78, 0x45]),
  // Squads v4
  multisigCreateV2: d([0x32, 0xdd, 0xc7, 0x5d, 0x28, 0xf5, 0x8b, 0xe9]),
  vaultTransactionCreate: d([0x30, 0xfa, 0x4e, 0xa8, 0xd0, 0xe2, 0xda, 0xd3]),
  vaultTransactionExecute: d([0xc2, 0x08, 0xa1, 0x57, 0x99, 0xa4, 0x19, 0xab]),
  proposalCreate: d([0xdc, 0x3c, 0x49, 0xe0, 0x1e, 0x6c, 0x4f, 0x9f]),
} as const;

/** Anchor account discriminators — `sha256("account:<Type>")[..8]`. */
export const ACCOUNT_DISC = {
  /** futarchy `Dao` (also enforced by G1 `set_governance`). */
  dao: d([0xa3, 0x09, 0x2f, 0x1f, 0x34, 0x55, 0xc5, 0x31]),
  /** futarchy `Proposal`. */
  proposal: d([0x1a, 0x5e, 0xbd, 0xbb, 0x74, 0x88, 0x35, 0x21]),
} as const;

const e = (s: string): Uint8Array => new TextEncoder().encode(s);

/** PDA seed-prefix byte strings. */
export const SEED = {
  // futarchy
  dao: e("dao"),
  proposal: e("proposal"),
  ammPosition: e("amm_position"),
  eventAuthority: e("__event_authority"),
  // conditional_vault
  question: e("question"),
  conditionalVault: e("conditional_vault"),
  conditionalToken: e("conditional_token"),
  // Squads v4 (Squads-Protocol/v4 state/seeds.rs)
  squadsPrefix: e("multisig"),
  squadsMultisig: e("multisig"),
  squadsVault: e("vault"),
  squadsTransaction: e("transaction"),
  squadsProposal: e("proposal"),
  squadsProgramConfig: e("program_config"),
  squadsSpendingLimit: e("spending_limit"),
} as const;

/** futarchy `Market` enum (Borsh tag). `conditional_swap` requires `!= Spot`. */
export enum Market {
  Spot = 0,
  Pass = 1,
  Fail = 2,
}

/** futarchy `SwapType` enum (Borsh tag). */
export enum SwapType {
  Buy = 0,
  Sell = 1,
}

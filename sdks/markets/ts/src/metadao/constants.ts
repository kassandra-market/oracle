/**
 * MetaDAO `conditional_vault` (v0.4.0) + `amm` (v0.4.2) wire constants.
 *
 * This is the TypeScript mirror of `kassandra-market/sdks/oracles/rust/src/metadao.rs`, the
 * single source of truth for the discriminators, PDA seeds, and account orders
 * the keeper/test harness uses to compose the MetaDAO market BEFORE calling
 * `kassandra-market::activate`. Every {@link DISC} value below is copied byte
 * for byte from that Rust module and guarded by `test/parity.test.ts`.
 *
 * The program only invokes `split_tokens` + `add_liquidity` (activate) and
 * `resolve_question` (resolve_market); the rest are composed client-side.
 */
import { Address } from "@solana/web3.js";

import { EXTERNAL_PROGRAM_IDS } from "../constants.js";

/** MetaDAO `conditional_vault` v0.4.0 program id (`sdks/oracles/rust` `CONDITIONAL_VAULT_ID`). */
export const CONDITIONAL_VAULT_ID = EXTERNAL_PROGRAM_IDS.conditionalVault;
/** MetaDAO `amm` v0.4.2 delayed-twap program id (`sdks/oracles/rust` `AMM_ID`). */
export const AMM_V04_ID = EXTERNAL_PROGRAM_IDS.ammV04;
/** SPL Associated-Token-Account program (`sdks/oracles/rust` `ASSOCIATED_TOKEN_PROGRAM_ID`). */
export const ATA_PROGRAM_ID = new Address("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

const d = (bytes: number[]): Uint8Array => Uint8Array.from(bytes);

/**
 * Anchor instruction discriminators — `sha256("global:<name>")[..8]`, copied
 * byte for byte from `sdks/oracles/rust/src/metadao.rs`.
 *
 * `crankThatTwap` is NOT declared in `sdks/oracles/rust` (the Rust side never composes it);
 * its value is the binary-validated one from the sibling AMM SDK.
 */
export const DISC = {
  // conditional_vault
  /** `conditional_vault::initialize_question` (`sdks/oracles/rust` `INITIALIZE_QUESTION_DISC`). */
  initializeQuestion: d([0xf5, 0x97, 0x6a, 0xbc, 0x58, 0x2c, 0x41, 0xd4]),
  /** `conditional_vault::initialize_conditional_vault` (`INITIALIZE_CONDITIONAL_VAULT_DISC`). */
  initializeConditionalVault: d([0x25, 0x58, 0xfa, 0xd4, 0x36, 0xda, 0xe3, 0xaf]),
  /** `conditional_vault::split_tokens` (`SPLIT_TOKENS_DISC`). */
  splitTokens: d([0x4f, 0xc3, 0x74, 0x00, 0x8c, 0xb0, 0x49, 0xb3]),
  /** `conditional_vault::merge_tokens` (`MERGE_TOKENS_DISC`). */
  mergeTokens: d([0xe2, 0x59, 0xfb, 0x79, 0xe1, 0x82, 0xb4, 0x0e]),
  /** `conditional_vault::redeem_tokens` (`REDEEM_TOKENS_DISC`). */
  redeemTokens: d([0xf6, 0x62, 0x86, 0x29, 0x98, 0x21, 0x78, 0x45]),
  /** `conditional_vault::resolve_question` (`RESOLVE_QUESTION_DISC`). */
  resolveQuestion: d([0x34, 0x20, 0xe0, 0xb3, 0xb4, 0x08, 0x00, 0xf6]),
  // amm v0.4
  /** `amm::create_amm` (`sdks/oracles/rust` `CREATE_AMM_DISC`). */
  createAmm: d([0xf2, 0x5b, 0x15, 0xaa, 0x05, 0x44, 0x7d, 0x40]),
  /** `amm::add_liquidity` (`ADD_LIQUIDITY_DISC`). */
  addLiquidity: d([0xb5, 0x9d, 0x59, 0x43, 0x8f, 0xb6, 0x34, 0x48]),
  /** `amm::swap` (`SWAP_DISC`). */
  swap: d([0xf8, 0xc6, 0x9e, 0x91, 0xe1, 0x75, 0x87, 0xc8]),
  /** `amm::crank_that_twap` — not in `sdks/oracles/rust`; sibling binary-validated value. */
  crankThatTwap: d([0xdc, 0x64, 0x19, 0xf9, 0x00, 0x5c, 0xc3, 0xc1]),
} as const;

/** Anchor `Amm` account discriminator — `sha256("account:Amm")[..8]` (`sdks/oracles/rust` `AMM_ACCOUNT_DISCRIMINATOR`). */
export const AMM_ACCOUNT_DISCRIMINATOR = d([0x8f, 0xf5, 0xc8, 0x11, 0x4a, 0xd6, 0xc4, 0x87]);

/** `Amm.base_amount: u64` byte offset (after the 8-byte discriminator + preceding fields). */
export const AMM_BASE_AMOUNT_OFFSET = 115;
/** `Amm.quote_amount: u64` byte offset. */
export const AMM_QUOTE_AMOUNT_OFFSET = 123;

const e = (s: string): Uint8Array => new TextEncoder().encode(s);

/** PDA seed-prefix byte strings (`sdks/oracles/rust/src/metadao.rs`). */
export const SEED = {
  /** `Question` PDA prefix. */
  question: e("question"),
  /** `ConditionalVault` PDA prefix. */
  conditionalVault: e("conditional_vault"),
  /** Conditional-token mint PDA prefix. */
  conditionalToken: e("conditional_token"),
  /** `Amm` PDA prefix — note the DOUBLE trailing underscore. */
  amm: e("amm__"),
  /** AMM LP-mint PDA prefix. */
  ammLpMint: e("amm_lp_mint"),
  /** Anchor `#[event_cpi]` event-authority prefix. */
  eventAuthority: e("__event_authority"),
} as const;

/** `amm::SwapType` Borsh tag (`Buy` = quote→base, `Sell` = base→quote). */
export enum SwapType {
  Buy = 0,
  Sell = 1,
}

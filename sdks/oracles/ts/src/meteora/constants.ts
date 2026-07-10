/**
 * Meteora **DAMM v2** (cp-amm, `cpamd…`) wire constants.
 *
 * cp-amm is Meteora's concentrated-liquidity constant-product AMM — the DAO's
 * SPOT-liquidity venue. It is POSITION-based (a position NFT per LP) and, unlike
 * the MetaDAO AMMs, has NO built-in oracle. Kassandra does NOT CPI it; this SDK
 * module is for the DAO treasury side (build/decode spot-path txs off-chain).
 *
 * ── Byte-sourced from `github.com/MeteoraAg/damm-v2`, program `cp-amm`, PINNED
 * at commit `bdd8a1e355f484b3cff131578a662c560b97b72f` (resolved off `main`
 * 2026-07-01). All raw URLs below are
 * `raw.githubusercontent.com/MeteoraAg/damm-v2/<that-commit>/programs/cp-amm/src/…`.
 *
 * Discriminators are Anchor `sha256("global:<name>")[..8]`. THREE are pinned
 * independently in the program (`programs/kassandra/src/cpi/metadao_v06.rs:126-134`)
 * and cross-checked here: `initialize_pool`, `swap`, `add_liquidity` (+ the `Pool`
 * account disc). The other three (`create_position`, `remove_liquidity`,
 * `claim_position_fee`) are computed the same way — the derivation is shown below.
 */
import { Address } from "@solana/web3.js";

import { EXTERNAL_PROGRAM_IDS } from "../constants.js";

/** Meteora DAMM v2 (cp-amm) program id (`lib.rs:41` `declare_id!`). */
export const METEORA_DAMM_V2_ID = EXTERNAL_PROGRAM_IDS.meteoraDammV2;

/** The SPL Associated Token Account program (shared with the rest of the SDK). */
export const ATA_PROGRAM_ID = new Address("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

/** SPL Token-2022 program — cp-amm mints every position NFT under Token-2022. */
export const TOKEN_2022_PROGRAM_ID = new Address("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

const d = (bytes: number[]): Uint8Array => Uint8Array.from(bytes);

/**
 * Anchor instruction discriminators — `sha256("global:<snake_case_name>")[..8]`.
 *
 * Derivation (reproduce with `sha256("global:<name>")` → first 8 bytes):
 *   initialize_pool     → 5fb40aac54aee828   (✓ metadao_v06.rs:127)
 *   swap                → f8c69e91e17587c8   (✓ metadao_v06.rs:129)
 *   add_liquidity       → b59d59438fb63448   (✓ metadao_v06.rs:131)
 *   create_position     → 30d7c59960cbb485   (computed; name from lib.rs:246)
 *   remove_liquidity    → 5055d14818ceb16c   (computed; name from lib.rs:257)
 *   claim_position_fee  → b4269a118521a2d3   (computed; name from lib.rs:294)
 */
export const DISC = {
  /** `cp_amm::initialize_pool` (lib.rs:225). */
  initializePool: d([0x5f, 0xb4, 0x0a, 0xac, 0x54, 0xae, 0xe8, 0x28]),
  /** `cp_amm::create_position` (lib.rs:246). */
  createPosition: d([0x30, 0xd7, 0xc5, 0x99, 0x60, 0xcb, 0xb4, 0x85]),
  /** `cp_amm::add_liquidity` (lib.rs:250). */
  addLiquidity: d([0xb5, 0x9d, 0x59, 0x43, 0x8f, 0xb6, 0x34, 0x48]),
  /** `cp_amm::remove_liquidity` (lib.rs:257). */
  removeLiquidity: d([0x50, 0x55, 0xd1, 0x48, 0x18, 0xce, 0xb1, 0x6c]),
  /** `cp_amm::swap` (lib.rs:286). */
  swap: d([0xf8, 0xc6, 0x9e, 0x91, 0xe1, 0x75, 0x87, 0xc8]),
  /** `cp_amm::claim_position_fee` (lib.rs:294). */
  claimPositionFee: d([0xb4, 0x26, 0x9a, 0x11, 0x85, 0x21, 0xa2, 0xd3]),
} as const;

/** Anchor `Pool` account discriminator — `sha256("account:Pool")[..8]` (✓ metadao_v06.rs:133). */
export const POOL_ACCOUNT_DISCRIMINATOR = d([0xf1, 0x9a, 0x6d, 0x04, 0x11, 0xb1, 0x6d, 0xbc]);

/** Anchor `Position` account discriminator — `sha256("account:Position")[..8]`. */
export const POSITION_ACCOUNT_DISCRIMINATOR = d([0xaa, 0xbc, 0x8f, 0xe4, 0x7a, 0x40, 0xf7, 0xd0]);

/**
 * `Pool` zero-copy struct size, WITHOUT the 8-byte Anchor discriminator
 * (`state/pool.rs` `const_assert_eq!(Pool::INIT_SPACE, 1104)`). On-chain the
 * account data is `8 + INIT_SPACE` bytes (see {@link POOL_ACCOUNT_SIZE}).
 */
export const POOL_INIT_SPACE = 1104;
/** On-chain `Pool` account length = `8 + Pool::INIT_SPACE` (`space = 8 + …` in ix_initialize_pool.rs:106). */
export const POOL_ACCOUNT_SIZE = 8 + POOL_INIT_SPACE; // 1112

/** `Position` zero-copy struct size, WITHOUT the disc (`state/position.rs` `INIT_SPACE == 400`). */
export const POSITION_INIT_SPACE = 400;
/** On-chain `Position` account length = `8 + Position::INIT_SPACE`. */
export const POSITION_ACCOUNT_SIZE = 8 + POSITION_INIT_SPACE; // 408

const e = (s: string): Uint8Array => new TextEncoder().encode(s);

/**
 * PDA seed-prefix byte strings — from `constants.rs` `mod seeds` (lines 173-197)
 * + `const_pda.rs`.
 */
export const SEED = {
  /** `config` (constants.rs:176). */
  config: e("config"),
  /** `pool` (constants.rs:182) — the Pool PDA prefix. */
  pool: e("pool"),
  /** `token_vault` (constants.rs:185). */
  tokenVault: e("token_vault"),
  /** `pool_authority` (constants.rs:188) — the const pool-authority PDA. */
  poolAuthority: e("pool_authority"),
  /** `position` (constants.rs:191). */
  position: e("position"),
  /** `position_nft_account` (constants.rs:194). */
  positionNftAccount: e("position_nft_account"),
  /** `__event_authority` (const_pda.rs:4) — Anchor `#[event_cpi]` prefix. */
  eventAuthority: e("__event_authority"),
} as const;

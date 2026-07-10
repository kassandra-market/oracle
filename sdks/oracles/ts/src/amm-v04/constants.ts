/**
 * MetaDAO **v0.4 standalone AMM** (`AMMyu265…`) wire constants.
 *
 * This is the FIRST-PARTY MetaDAO AMM with a built-in delayed-TWAP oracle — the
 * program `settle_challenge` reads its TWAP from. It is DISTINCT from the v0.6
 * futarchy *embedded* AMM (`src/futarchy`), which is a different program with a
 * different account/arg shape; do NOT cross-use the two.
 *
 * Discriminators are the binary-validated values from
 * `programs/kassandra/src/cpi/metadao.rs:82-94` (`sha256("global:<name>")[..8]`).
 * Account orderings + arg layouts are proven against the real `metadao_amm.so`
 * fixture in `programs/kassandra/tests/challenge_e2e.rs:631-769`.
 */
import { Address } from "@solana/web3.js";

import { EXTERNAL_PROGRAM_IDS } from "../constants.js";

/** MetaDAO v0.4 standalone AMM program id (`metadao.rs:59` `AMM_ID`). */
export const AMM_V04_ID = EXTERNAL_PROGRAM_IDS.ammV04;

/** The SPL Associated Token Account program (`challenge_e2e.rs` `ATA_PROGRAM_ID`). */
export const ATA_PROGRAM_ID = new Address("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

const d = (bytes: number[]): Uint8Array => Uint8Array.from(bytes);

/**
 * Anchor instruction discriminators — `sha256("global:<name>")[..8]`, copied
 * byte-for-byte from `metadao.rs:82-94`.
 */
export const DISC = {
  /** `amm::create_amm` (`metadao.rs:82` `CREATE_AMM`). */
  createAmm: d([0xf2, 0x5b, 0x15, 0xaa, 0x05, 0x44, 0x7d, 0x40]),
  /** `amm::add_liquidity` (`metadao.rs:85` `ADD_LIQUIDITY`). */
  addLiquidity: d([0xb5, 0x9d, 0x59, 0x43, 0x8f, 0xb6, 0x34, 0x48]),
  /** `amm::swap` (`metadao.rs:90` `SWAP`). */
  swap: d([0xf8, 0xc6, 0x9e, 0x91, 0xe1, 0x75, 0x87, 0xc8]),
  /** `amm::crank_that_twap` (`metadao.rs:94` `CRANK_THAT_TWAP`). */
  crankThatTwap: d([0xdc, 0x64, 0x19, 0xf9, 0x00, 0x5c, 0xc3, 0xc1]),
} as const;

/** Anchor `Amm` account discriminator — `sha256("account:Amm")[..8]` (`metadao.rs:161`). */
export const AMM_ACCOUNT_DISCRIMINATOR = d([0x8f, 0xf5, 0xc8, 0x11, 0x4a, 0xd6, 0xc4, 0x87]);

const e = (s: string): Uint8Array => new TextEncoder().encode(s);

/** PDA seed-prefix byte strings (`challenge_e2e.rs:641-646`). */
export const SEED = {
  /** `Amm` PDA prefix — note the DOUBLE trailing underscore. */
  amm: e("amm__"),
  /** LP-mint PDA prefix. */
  lpMint: e("amm_lp_mint"),
  /** Anchor `#[event_cpi]` event-authority prefix. */
  eventAuthority: e("__event_authority"),
} as const;

/** v0.4 `amm::SwapType` Borsh tag (`metadao.rs:88`). */
export enum SwapType {
  Buy = 0,
  Sell = 1,
}

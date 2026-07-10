/**
 * PDA / ATA derivers for Meteora **DAMM v2** (cp-amm).
 *
 * Every seed list is byte-sourced from the pinned handlers
 * (commit `bdd8a1e355f484b3cff131578a662c560b97b72f`):
 *   - Pool          `[b"pool", config, max(mint_a,mint_b), min(mint_a,mint_b)]`
 *                   (ix_initialize_pool.rs:98-104 — mints are SORTED by raw bytes,
 *                   larger first). NOTE: keyed by the `config` account.
 *   - Position      `[b"position", position_nft_mint]`               (ix_initialize_pool.rs:112-115)
 *   - Position NFT  `[b"position_nft_account", position_nft_mint]`   (ix_initialize_pool.rs:73)
 *   - Token vault   `[b"token_vault", token_mint, pool]`             (ix_initialize_pool.rs:138-142)
 *   - Pool authority`[b"pool_authority"]`   — a program CONSTANT     (const_pda.rs pool_authority)
 *   - Event auth    `[b"__event_authority"]`— the `#[event_cpi]` PDA (const_pda.rs:4)
 *
 * web3.js@3 exposes only async `Address.findProgramAddress`, so every derivation
 * is async and returns `{ address, bump }`.
 */
import { Address } from "@solana/web3.js";

import type { AddressInput, Pda } from "../pda.js";
import { METEORA_DAMM_V2_ID, SEED } from "./constants.js";

function bytes(a: AddressInput): Uint8Array {
  return (a instanceof Address ? a : new Address(a)).toBytes();
}

async function derive(seeds: Array<Uint8Array>, programId: Address = METEORA_DAMM_V2_ID): Promise<Pda> {
  const [address, bump] = await Address.findProgramAddress(seeds, programId);
  return { address, bump };
}

/**
 * Sort two mint pubkeys the way the program does — `max_key`/`min_key` in
 * ix_initialize_pool.rs:31-37 use Rust `Pubkey` `Ord`, i.e. raw-byte lexicographic
 * comparison. Returns `[larger, smaller]` (max first, min second).
 */
export function sortMints(mintA: AddressInput, mintB: AddressInput): [Uint8Array, Uint8Array] {
  const a = bytes(mintA);
  const b = bytes(mintB);
  for (let i = 0; i < 32; i++) {
    if (a[i] !== b[i]) return a[i] > b[i] ? [a, b] : [b, a];
  }
  return [a, b];
}

/**
 * `Pool` PDA — seeds `[b"pool", config, max(mint_a,mint_b), min(mint_a,mint_b)]`
 * (ix_initialize_pool.rs:98-104). The pool is keyed by the `config` account and
 * the SORTED mint pair, so the derivation is order-independent in the mints.
 */
export function pool(
  config: AddressInput,
  mintA: AddressInput,
  mintB: AddressInput,
): Promise<Pda> {
  const [hi, lo] = sortMints(mintA, mintB);
  return derive([SEED.pool, bytes(config), hi, lo]);
}

/** `Position` PDA — seeds `[b"position", position_nft_mint]` (ix_initialize_pool.rs:112-115). */
export function position(positionNftMint: AddressInput): Promise<Pda> {
  return derive([SEED.position, bytes(positionNftMint)]);
}

/**
 * Position-NFT token account PDA — seeds `[b"position_nft_account", position_nft_mint]`
 * (ix_initialize_pool.rs:73). Token-2022 account holding the single position NFT.
 */
export function positionNftAccount(positionNftMint: AddressInput): Promise<Pda> {
  return derive([SEED.positionNftAccount, bytes(positionNftMint)]);
}

/** Token vault PDA — seeds `[b"token_vault", token_mint, pool]` (ix_initialize_pool.rs:138-142). */
export function tokenVault(tokenMint: AddressInput, poolAddr: AddressInput): Promise<Pda> {
  return derive([SEED.tokenVault, bytes(tokenMint), bytes(poolAddr)]);
}

/** Pool-authority PDA — seeds `[b"pool_authority"]`, a program constant (const_pda.rs). */
export function poolAuthority(): Promise<Pda> {
  return derive([SEED.poolAuthority]);
}

/** `#[event_cpi]` event-authority PDA — seeds `[b"__event_authority"]` (const_pda.rs:4). */
export function eventAuthority(): Promise<Pda> {
  return derive([SEED.eventAuthority]);
}

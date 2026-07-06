/**
 * Program-derived-address (PDA) derivation for the kassandra-market program.
 *
 * Every seed list here is the program's PUBLIC CONTRACT, verified against
 * `sdk-rs/src/pda.rs`. Seed-byte encodings:
 *   - literal seeds (`"config"`, `"market"`, ...) are ASCII byte strings;
 *   - pubkey seeds are the 32 RAW bytes of the address.
 *
 * web3.js@3.0.0-rc.2 exposes only the ASYNC `Address.findProgramAddress`
 * (there is NO `findProgramAddressSync` in this version), so every derivation
 * here is async and returns `{ address, bump }`.
 */
import { Address } from "@solana/web3.js";

import { ATA_PROGRAM_ID, BPF_UPGRADEABLE_LOADER_ID, MARKET_PROGRAM_ID, TOKEN_PROGRAM_ID } from "./constants.js";

/** Anything that can name an account: a web3.js `Address`/`PublicKey` or a base58 string. */
export type AddressInput = Address | string;

/** A derived PDA: its address and the canonical bump seed. */
export interface Pda {
  address: Address;
  bump: number;
}

const enc = new TextEncoder();

/** 32 raw bytes of an address (the seed form of a pubkey). */
function pubkeyBytes(a: AddressInput): Uint8Array {
  return (a instanceof Address ? a : new Address(a)).toBytes();
}

async function derive(seeds: Array<Uint8Array>, programId: Address = MARKET_PROGRAM_ID): Promise<Pda> {
  const [address, bump] = await Address.findProgramAddress(seeds, programId);
  return { address, bump };
}

/** Config singleton PDA — seeds `[b"config"]`. */
export function config(programId?: Address): Promise<Pda> {
  return derive([enc.encode("config")], programId);
}

/**
 * The BPF-Upgradeable-Loader `ProgramData` account of `programId` — seeds
 * `[programId]`, derived under {@link BPF_UPGRADEABLE_LOADER_ID} (NOT the market
 * program). Stores the program's `upgrade_authority`, which `initConfig` requires
 * the caller to be. Mirror of `sdk-rs/src/pda.rs::program_data`.
 */
export function programData(programId: Address = MARKET_PROGRAM_ID): Promise<Pda> {
  return derive([pubkeyBytes(programId)], BPF_UPGRADEABLE_LOADER_ID);
}

/**
 * Per-outcome binary sub-market PDA — seeds `[b"market", oracle, [outcomeIndex]]`.
 * Binary markets use `outcomeIndex = 0`; a categorical oracle has one sub-market
 * per outcome (`0 <= outcomeIndex < oracle.options_count`).
 */
export function market(oracle: AddressInput, outcomeIndex: number, programId?: Address): Promise<Pda> {
  return derive([enc.encode("market"), pubkeyBytes(oracle), Uint8Array.of(outcomeIndex)], programId);
}

/** Market escrow vault PDA (SPL token account) — seeds `[b"escrow", market]`. */
export function escrow(marketAddr: AddressInput, programId?: Address): Promise<Pda> {
  return derive([enc.encode("escrow"), pubkeyBytes(marketAddr)], programId);
}

/** Contribution PDA — seeds `[b"contribution", market, contributor]`. */
export function contribution(
  marketAddr: AddressInput,
  contributor: AddressInput,
  programId?: Address,
): Promise<Pda> {
  return derive([enc.encode("contribution"), pubkeyBytes(marketAddr), pubkeyBytes(contributor)], programId);
}

/** Market-PDA-owned cYES holder — seeds `[b"cyes", market]`. */
export function cyes(marketAddr: AddressInput, programId?: Address): Promise<Pda> {
  return derive([enc.encode("cyes"), pubkeyBytes(marketAddr)], programId);
}

/** Market-PDA-owned cNO holder — seeds `[b"cno", market]`. */
export function cno(marketAddr: AddressInput, programId?: Address): Promise<Pda> {
  return derive([enc.encode("cno"), pubkeyBytes(marketAddr)], programId);
}

/** Market-PDA-owned LP token account — seeds `[b"lp_vault", market]`. */
export function lpVault(marketAddr: AddressInput, programId?: Address): Promise<Pda> {
  return derive([enc.encode("lp_vault"), pubkeyBytes(marketAddr)], programId);
}

/**
 * SPL associated-token-account address — seeds `[owner, TOKEN_PROGRAM, mint]`
 * under the {@link ATA_PROGRAM_ID}. NOTE: derived under the ATA program, NOT the
 * market program — there is no `programId` override.
 */
export function associatedTokenAccount(
  owner: AddressInput,
  mint: AddressInput,
): Promise<Pda> {
  return derive([pubkeyBytes(owner), TOKEN_PROGRAM_ID.toBytes(), pubkeyBytes(mint)], ATA_PROGRAM_ID);
}

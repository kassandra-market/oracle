/**
 * PDA / ATA derivers for the MetaDAO v0.4 standalone AMM.
 *
 * Seed lists mirror the binary-validated Rust derivations in
 * `programs/kassandra/tests/challenge_e2e.rs:641-650` (proven against the real
 * `metadao_amm.so`). web3.js@3 exposes only async `Address.findProgramAddress`,
 * so every derivation is async and returns `{ address, bump }`.
 */
import { Address } from "@solana/web3.js";

import { TOKEN_PROGRAM_ID } from "../constants.js";
import type { AddressInput, Pda } from "../pda.js";
import { AMM_V04_ID, ATA_PROGRAM_ID, SEED } from "./constants.js";

function bytes(a: AddressInput): Uint8Array {
  return (a instanceof Address ? a : new Address(a)).toBytes();
}

async function derive(seeds: Array<Uint8Array>, programId: Address): Promise<Pda> {
  const [address, bump] = await Address.findProgramAddress(seeds, programId);
  return { address, bump };
}

/** `Amm` PDA — seeds `[b"amm__", base_mint, quote_mint]` (`challenge_e2e.rs:641`). */
export function amm(baseMint: AddressInput, quoteMint: AddressInput): Promise<Pda> {
  return derive([SEED.amm, bytes(baseMint), bytes(quoteMint)], AMM_V04_ID);
}

/** LP-mint PDA — seeds `[b"amm_lp_mint", amm]` (`challenge_e2e.rs:646`). */
export function lpMint(ammAddr: AddressInput): Promise<Pda> {
  return derive([SEED.lpMint, bytes(ammAddr)], AMM_V04_ID);
}

/** AMM `#[event_cpi]` event-authority PDA — seeds `[b"__event_authority"]`. */
export function eventAuthority(): Promise<Pda> {
  return derive([SEED.eventAuthority], AMM_V04_ID);
}

/**
 * Associated token account — seeds `[owner, TOKEN_PROGRAM, mint]` under the ATA
 * program. The AMM's base/quote vaults are the ATAs of the `Amm` PDA
 * (`challenge_e2e.rs:647-648` `ata(&amm, &mint)`).
 */
export async function ata(owner: AddressInput, mint: AddressInput): Promise<Address> {
  const [a] = await Address.findProgramAddress(
    [bytes(owner), TOKEN_PROGRAM_ID.toBytes(), bytes(mint)],
    ATA_PROGRAM_ID,
  );
  return a;
}

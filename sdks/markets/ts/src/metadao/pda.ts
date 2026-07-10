/**
 * PDA / ATA derivers for the MetaDAO `conditional_vault` + `amm` v0.4 programs.
 *
 * Seed lists mirror `sdks/oracles/rust/src/metadao.rs` (the wire-format source of truth for
 * this repo). web3.js@3.0.0-rc.2 exposes only the async `Address.findProgramAddress`
 * (there is no sync variant), so every derivation is async and returns
 * `{ address, bump }`.
 */
import { Address } from "@solana/web3.js";

import { TOKEN_PROGRAM_ID } from "../constants.js";
import type { AddressInput, Pda } from "../pda.js";
import { AMM_V04_ID, ATA_PROGRAM_ID, CONDITIONAL_VAULT_ID, SEED } from "./constants.js";

function bytes(a: AddressInput): Uint8Array {
  return (a instanceof Address ? a : new Address(a)).toBytes();
}

function u8(value: number): Uint8Array {
  return Uint8Array.from([value & 0xff]);
}

async function derive(seeds: Array<Uint8Array>, programId: Address): Promise<Pda> {
  const [address, bump] = await Address.findProgramAddress(seeds, programId);
  return { address, bump };
}

// ── conditional_vault ─────────────────────────────────────────────────────────

/** `Question` PDA — seeds `[b"question", question_id[32], oracle, [num_outcomes]]`. */
export function question(
  questionId: Uint8Array,
  oracle: AddressInput,
  numOutcomes: number,
): Promise<Pda> {
  return derive(
    [SEED.question, questionId, bytes(oracle), u8(numOutcomes)],
    CONDITIONAL_VAULT_ID,
  );
}

/** `ConditionalVault` PDA — seeds `[b"conditional_vault", question, underlying_mint]`. */
export function conditionalVault(question: AddressInput, underlyingMint: AddressInput): Promise<Pda> {
  return derive(
    [SEED.conditionalVault, bytes(question), bytes(underlyingMint)],
    CONDITIONAL_VAULT_ID,
  );
}

/** Conditional-token mint PDA — seeds `[b"conditional_token", vault, [index]]`. */
export function conditionalTokenMint(vault: AddressInput, index: number): Promise<Pda> {
  return derive([SEED.conditionalToken, bytes(vault), u8(index)], CONDITIONAL_VAULT_ID);
}

/** conditional_vault `#[event_cpi]` event-authority PDA — seeds `[b"__event_authority"]`. */
export function vaultEventAuthority(): Promise<Pda> {
  return derive([SEED.eventAuthority], CONDITIONAL_VAULT_ID);
}

// ── amm v0.4 ──────────────────────────────────────────────────────────────────

/** `Amm` PDA — seeds `[b"amm__", base_mint, quote_mint]`. */
export function amm(baseMint: AddressInput, quoteMint: AddressInput): Promise<Pda> {
  return derive([SEED.amm, bytes(baseMint), bytes(quoteMint)], AMM_V04_ID);
}

/** AMM LP-mint PDA — seeds `[b"amm_lp_mint", amm]`. */
export function ammLpMint(ammAddr: AddressInput): Promise<Pda> {
  return derive([SEED.ammLpMint, bytes(ammAddr)], AMM_V04_ID);
}

/** AMM `#[event_cpi]` event-authority PDA — seeds `[b"__event_authority"]`. */
export function ammEventAuthority(): Promise<Pda> {
  return derive([SEED.eventAuthority], AMM_V04_ID);
}

/**
 * `#[event_cpi]` event-authority PDA under an arbitrary `programId` — seeds
 * `[b"__event_authority"]`. Prefer {@link vaultEventAuthority} /
 * {@link ammEventAuthority} for the two MetaDAO programs.
 */
export function eventAuthority(programId: Address): Promise<Pda> {
  return derive([SEED.eventAuthority], programId);
}

/**
 * Associated token account — seeds `[owner, TOKEN_PROGRAM, mint]` under the ATA
 * program. The AMM's per-mint vault is `ata(amm, conditionalMint)`; the vault's
 * underlying account is `ata(vault, underlyingMint)`.
 */
export async function ata(owner: AddressInput, mint: AddressInput): Promise<Address> {
  const [a] = await Address.findProgramAddress(
    [bytes(owner), TOKEN_PROGRAM_ID.toBytes(), bytes(mint)],
    ATA_PROGRAM_ID,
  );
  return a;
}

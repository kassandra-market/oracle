/**
 * PDA derivers for futarchy v0.6 + Squads v4 + conditional_vault.
 *
 * Seed lists mirror the binary-validated Rust derivers in `metadao_v06.rs` /
 * `metadao.rs` and the on-chain account constraints (see `./NOTES.md`).
 * web3.js@3 exposes only async `Address.findProgramAddress`, so every derivation
 * is async and returns `{ address, bump }`.
 */
import { Address } from "@solana/web3.js";

import type { AddressInput, Pda } from "../pda.js";
import {
  CONDITIONAL_VAULT_ID,
  FUTARCHY_ID,
  SEED,
  SQUADS_V4_ID,
} from "./constants.js";

function bytes(a: AddressInput): Uint8Array {
  return (a instanceof Address ? a : new Address(a)).toBytes();
}

function u64le(value: bigint | number): Uint8Array {
  const out = new Uint8Array(8);
  new DataView(out.buffer).setBigUint64(0, BigInt(value), true);
  return out;
}

async function derive(seeds: Array<Uint8Array>, programId: Address): Promise<Pda> {
  const [address, bump] = await Address.findProgramAddress(seeds, programId);
  return { address, bump };
}

// ── futarchy ────────────────────────────────────────────────────────────────

/** futarchy `Dao` PDA — `[b"dao", dao_creator, nonce:u64le]`. */
export function dao(daoCreator: AddressInput, nonce: bigint | number): Promise<Pda> {
  return derive([SEED.dao, bytes(daoCreator), u64le(nonce)], FUTARCHY_ID);
}

/** futarchy `Proposal` PDA — `[b"proposal", squads_proposal]`. */
export function proposal(squadsProposal: AddressInput): Promise<Pda> {
  return derive([SEED.proposal, bytes(squadsProposal)], FUTARCHY_ID);
}

/** futarchy `#[event_cpi]` event-authority PDA — `[b"__event_authority"]`. */
export function futarchyEventAuthority(): Promise<Pda> {
  return derive([SEED.eventAuthority], FUTARCHY_ID);
}

/** futarchy `AmmPosition` PDA — `[b"amm_position", dao, position_authority]`. */
export function ammPosition(dao: AddressInput, positionAuthority: AddressInput): Promise<Pda> {
  return derive([SEED.ammPosition, bytes(dao), bytes(positionAuthority)], FUTARCHY_ID);
}

// ── Squads v4 ─────────────────────────────────────────────────────────────────

/** Squads multisig PDA — `[b"multisig", b"multisig", create_key]` (create_key == Dao). */
export function squadsMultisig(createKey: AddressInput): Promise<Pda> {
  return derive([SEED.squadsPrefix, SEED.squadsMultisig, bytes(createKey)], SQUADS_V4_ID);
}

/** Squads vault PDA — `[b"multisig", multisig, b"vault", index:u8]` (DAO uses index 0). */
export function squadsVault(multisig: AddressInput, vaultIndex = 0): Promise<Pda> {
  return derive(
    [SEED.squadsPrefix, bytes(multisig), SEED.squadsVault, Uint8Array.from([vaultIndex & 0xff])],
    SQUADS_V4_ID,
  );
}

/** Squads program-config PDA — `[b"multisig", b"program_config"]`. */
export function squadsProgramConfig(): Promise<Pda> {
  return derive([SEED.squadsPrefix, SEED.squadsProgramConfig], SQUADS_V4_ID);
}

/** Squads spending-limit PDA — `[b"multisig", multisig, b"spending_limit", create_key]`. */
export function squadsSpendingLimit(multisig: AddressInput, createKey: AddressInput): Promise<Pda> {
  return derive(
    [SEED.squadsPrefix, bytes(multisig), SEED.squadsSpendingLimit, bytes(createKey)],
    SQUADS_V4_ID,
  );
}

/** Squads vault-transaction PDA — `[b"multisig", multisig, b"transaction", index:u64le]`. */
export function squadsTransaction(multisig: AddressInput, transactionIndex: bigint | number): Promise<Pda> {
  return derive(
    [SEED.squadsPrefix, bytes(multisig), SEED.squadsTransaction, u64le(transactionIndex)],
    SQUADS_V4_ID,
  );
}

/** Squads proposal PDA — `[b"multisig", multisig, b"transaction", index:u64le, b"proposal"]`. */
export function squadsProposal(multisig: AddressInput, transactionIndex: bigint | number): Promise<Pda> {
  return derive(
    [SEED.squadsPrefix, bytes(multisig), SEED.squadsTransaction, u64le(transactionIndex), SEED.squadsProposal],
    SQUADS_V4_ID,
  );
}

// ── conditional_vault ─────────────────────────────────────────────────────────

/** `Question` PDA — `[b"question", question_id[32], oracle, [num_outcomes]]`. */
export function question(questionId: Uint8Array, oracle: AddressInput, numOutcomes: number): Promise<Pda> {
  return derive(
    [SEED.question, questionId, bytes(oracle), Uint8Array.from([numOutcomes & 0xff])],
    CONDITIONAL_VAULT_ID,
  );
}

/** `ConditionalVault` PDA — `[b"conditional_vault", question, underlying_mint]`. */
export function conditionalVault(question: AddressInput, underlyingMint: AddressInput): Promise<Pda> {
  return derive([SEED.conditionalVault, bytes(question), bytes(underlyingMint)], CONDITIONAL_VAULT_ID);
}

/** Conditional-token mint PDA — `[b"conditional_token", vault, [index]]`. */
export function conditionalTokenMint(vault: AddressInput, index: number): Promise<Pda> {
  return derive(
    [SEED.conditionalToken, bytes(vault), Uint8Array.from([index & 0xff])],
    CONDITIONAL_VAULT_ID,
  );
}

/** conditional_vault `#[event_cpi]` event-authority PDA — `[b"__event_authority"]`. */
export function vaultEventAuthority(): Promise<Pda> {
  return derive([SEED.eventAuthority], CONDITIONAL_VAULT_ID);
}

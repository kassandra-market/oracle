/**
 * Instruction builders for the MetaDAO `conditional_vault` (v0.4.0).
 *
 * Each builder returns a web3.js (classic) `TransactionInstruction` whose
 * `data == [disc, ...borsh_args]` and whose `keys` are the EXACT account-meta
 * order pinned in `sdk-rs/src/metadao.rs` (the wire-format source of truth for
 * this repo). Every instruction is Anchor `#[event_cpi]`: the two trailing
 * accounts (event-authority PDA, program id) are appended by the builders.
 */
import { TransactionInstruction } from "@solana/web3.js";
import type { AccountMeta } from "@solana/web3.js";

import { SYSTEM_PROGRAM_ID, TOKEN_PROGRAM_ID } from "../constants.js";
import type { AddressInput } from "../pda.js";
import { ATA_PROGRAM_ID, CONDITIONAL_VAULT_ID, DISC } from "./constants.js";
import * as pda from "./pda.js";
import { addr, concat, ro, u8b, u32le, u64le, w } from "./util.js";

// ── initialize_question ───────────────────────────────────────────────────────

export interface InitializeQuestionArgs {
  /** Rent payer + signer. */
  payer: AddressInput;
  /** 32-byte question id. */
  questionId: Uint8Array;
  /** Oracle/resolver authority (kassandra-market uses the Market PDA). */
  oracle: AddressInput;
  /** Outcome count (binary markets use 2). */
  numOutcomes: number;
}

/**
 * `initialize_question` — 5 accounts (incl. the two `#[event_cpi]` trailers).
 * Data: `disc[8] ++ question_id[32] ++ oracle[32] ++ num_outcomes[1]`.
 */
export async function initializeQuestion(a: InitializeQuestionArgs): Promise<TransactionInstruction> {
  const question = (await pda.question(a.questionId, a.oracle, a.numOutcomes)).address;
  const eventAuthority = (await pda.vaultEventAuthority()).address;
  return new TransactionInstruction({
    programId: CONDITIONAL_VAULT_ID,
    keys: [
      w(question),
      w(a.payer, true),
      ro(SYSTEM_PROGRAM_ID),
      ro(eventAuthority),
      ro(CONDITIONAL_VAULT_ID),
    ],
    data: concat([
      DISC.initializeQuestion,
      a.questionId,
      addr(a.oracle).toBytes(),
      u8b(a.numOutcomes),
    ]),
  });
}

// ── initialize_conditional_vault ──────────────────────────────────────────────

export interface InitializeConditionalVaultArgs {
  /** Rent payer + signer. */
  payer: AddressInput;
  /** The `Question` this vault settles against. */
  question: AddressInput;
  /** Underlying token mint (kassandra-market uses the KASS mint). */
  underlyingMint: AddressInput;
  /** Number of conditional-token mints created (default 2). */
  numOutcomes?: number;
}

/**
 * `initialize_conditional_vault` — 10 fixed accounts + `numOutcomes` trailing
 * conditional-token mints (w, PDA). Data: discriminator only.
 */
export async function initializeConditionalVault(
  a: InitializeConditionalVaultArgs,
): Promise<TransactionInstruction> {
  const n = a.numOutcomes ?? 2;
  const vault = (await pda.conditionalVault(a.question, a.underlyingMint)).address;
  const vaultUnderlying = await pda.ata(vault, a.underlyingMint);
  const eventAuthority = (await pda.vaultEventAuthority()).address;
  const condMints: AccountMeta[] = [];
  for (let i = 0; i < n; i++) {
    condMints.push(w((await pda.conditionalTokenMint(vault, i)).address));
  }
  return new TransactionInstruction({
    programId: CONDITIONAL_VAULT_ID,
    keys: [
      w(vault),
      ro(a.question),
      ro(a.underlyingMint),
      w(vaultUnderlying),
      w(a.payer, true),
      ro(TOKEN_PROGRAM_ID),
      ro(ATA_PROGRAM_ID),
      ro(SYSTEM_PROGRAM_ID),
      ro(eventAuthority),
      ro(CONDITIONAL_VAULT_ID),
      ...condMints,
    ],
    data: DISC.initializeConditionalVault,
  });
}

// ── split / merge / redeem (shared InteractWithVault layout) ──────────────────

export interface InteractWithVaultArgs {
  question: AddressInput;
  vault: AddressInput;
  /** The vault's underlying ATA (`ata(vault, underlyingMint)`). */
  vaultUnderlyingAta: AddressInput;
  /** Signer that owns the user token accounts. */
  authority: AddressInput;
  userUnderlyingAta: AddressInput;
  /** Conditional-token mints in outcome order (index 0..n). */
  conditionalMints: AddressInput[];
  /** User's conditional-token accounts in outcome order (index 0..n). */
  userConditionalAtas: AddressInput[];
}

/**
 * Shared `InteractWithVault` account list (`split`/`merge`/`redeem`):
 *   0 question(ro) 1 vault(w) 2 vault_underlying_ata(w) 3 authority(signer)
 *   4 user_underlying_ata(w) 5 token_program 6 event_authority 7 cv_program
 *   ...conditional mints(w) ...user conditional accounts(w)
 */
async function interactWithVault(
  disc: Uint8Array,
  a: InteractWithVaultArgs,
): Promise<TransactionInstruction> {
  const eventAuthority = (await pda.vaultEventAuthority()).address;
  return new TransactionInstruction({
    programId: CONDITIONAL_VAULT_ID,
    keys: [
      ro(a.question),
      w(a.vault),
      w(a.vaultUnderlyingAta),
      ro(a.authority, true),
      w(a.userUnderlyingAta),
      ro(TOKEN_PROGRAM_ID),
      ro(eventAuthority),
      ro(CONDITIONAL_VAULT_ID),
      ...a.conditionalMints.map((m) => w(m)),
      ...a.userConditionalAtas.map((u) => w(u)),
    ],
    data: disc,
  });
}

/** `split_tokens` — mints `amount` of each conditional token, pulls underlying in. */
export function splitTokens(
  a: InteractWithVaultArgs & { amount: bigint | number },
): Promise<TransactionInstruction> {
  return interactWithVault(concat([DISC.splitTokens, u64le(a.amount)]), a);
}

/** `merge_tokens` — burns `amount` of each conditional token, returns underlying. */
export function mergeTokens(
  a: InteractWithVaultArgs & { amount: bigint | number },
): Promise<TransactionInstruction> {
  return interactWithVault(concat([DISC.mergeTokens, u64le(a.amount)]), a);
}

/** `redeem_tokens` — burns the holder's full balances, pays out per resolution. */
export function redeemTokens(a: InteractWithVaultArgs): Promise<TransactionInstruction> {
  return interactWithVault(DISC.redeemTokens, a);
}

// ── resolve_question ──────────────────────────────────────────────────────────

export interface ResolveQuestionArgs {
  question: AddressInput;
  /** The question's oracle (readonly signer; kassandra-market's Market PDA). */
  oracle: AddressInput;
  /** Binary payout numerators — `[1,0]` outcome-0 pays, `[0,1]` outcome-1 pays, `[1,1]` void. */
  payoutNumerators: [number, number];
}

/**
 * `resolve_question` (binary) — 4 accounts. Data:
 * `disc[8] ++ len:u32le(2) ++ numerators[0]:u32le ++ numerators[1]:u32le`.
 */
export async function resolveQuestion(a: ResolveQuestionArgs): Promise<TransactionInstruction> {
  const eventAuthority = (await pda.vaultEventAuthority()).address;
  return new TransactionInstruction({
    programId: CONDITIONAL_VAULT_ID,
    keys: [w(a.question), ro(a.oracle, true), ro(eventAuthority), ro(CONDITIONAL_VAULT_ID)],
    data: concat([
      DISC.resolveQuestion,
      u32le(2),
      u32le(a.payoutNumerators[0]),
      u32le(a.payoutNumerators[1]),
    ]),
  });
}

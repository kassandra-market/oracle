/**
 * Shared helpers for the funding-phase action builders (pure, NO React).
 *
 * The kassandra-market instructions transfer KASS out of / into the caller's
 * Associated Token Account. The SDK builders take that ATA address but do NOT
 * create it, so a first-time participant needs the account created in the same
 * transaction. {@link ensureKassAta} derives `ATA(owner, kassMint)` and, when the
 * account is absent, returns an idempotent create-ATA instruction to PREPEND.
 *
 * The create ix is the SDK's shared leaf builder (the ATA program's
 * `CreateIdempotent`, discriminant byte `1`).
 */
import { Address, TransactionInstruction } from "@solana/web3.js";
import { flows, pda } from "@kassandra-market/markets";
import type { IndexerClient } from "../../lib/indexer";
import { ValidationError } from "../writeAction";

/** Anything that names an account: a web3.js `Address` or a base58 string. */
export type AddressInput = Address | string;

/**
 * Coerce an {@link AddressInput} into an `Address`, re-typing a parse failure as a
 * typed {@link ValidationError} the form surfaces inline against `field`.
 */
export function toAddress(field: string, a: AddressInput): Address {
  if (a instanceof Address) return a;
  try {
    return new Address(a);
  } catch {
    throw new ValidationError(`${field} is not a valid base58 address.`);
  }
}

/**
 * Derive `ATA(owner, mint)` and, when the account is absent (`indexer.getAccount`
 * null), return an idempotent create-ATA ix to prepend (payer == owner). The
 * returned `createIx` is `undefined` when the ATA already exists, so callers
 * write `createIx ? [createIx, ix] : [ix]`. Used for the KASS escrow source and
 * the LP claim destination alike (any mint).
 */
export async function ensureAta(
  indexer: IndexerClient,
  owner: Address,
  mint: Address,
): Promise<{ ata: Address; createIx?: TransactionInstruction }> {
  const ata = (await pda.associatedTokenAccount(owner, mint)).address;
  const info = await indexer.getAccount(ata.toString());
  const createIx = info
    ? undefined
    : flows.createAtaIdempotentInstruction(owner, ata, owner, mint);
  return { ata, createIx };
}

/** {@link ensureAta} specialised to the KASS mint (the funding-form call sites). */
export function ensureKassAta(
  indexer: IndexerClient,
  owner: Address,
  kassMint: Address,
): Promise<{ ata: Address; createIx?: TransactionInstruction }> {
  return ensureAta(indexer, owner, kassMint);
}

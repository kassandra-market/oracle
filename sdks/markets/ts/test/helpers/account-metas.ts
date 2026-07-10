/**
 * Shared fixtures for the account-meta golden guard tests.
 *
 * Extracted verbatim from the original `account-metas.test.ts` so the
 * kassandra-market and MetaDAO golden suites can share the placeholder
 * addresses, the program-id table, and the `label()` helper without drift.
 */
import { Address, TransactionInstruction } from "@solana/web3.js";

import {
  EXTERNAL_PROGRAM_IDS,
  SYSTEM_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "../../src/constants.js";
import { ATA_PROGRAM_ID } from "../../src/metadao/constants.js";

export const CV_PROGRAM = EXTERNAL_PROGRAM_IDS.conditionalVault;
export const AMM_PROGRAM = EXTERNAL_PROGRAM_IDS.ammV04;

/** Deterministic distinct placeholder address (32 bytes of `n`). */
export const A = (n: number): Address => new Address(new Uint8Array(32).fill(n));

/** One golden row: the account's role name + its signer/writable flags. */
export type Meta = [name: string, isSigner: boolean, isWritable: boolean];

/**
 * Label each account meta by looking its pubkey up in `entries` (a reverse map of
 * `address -> role`). Throws on any unmapped account so a builder that grew/moved a
 * slot fails loudly rather than silently. Returns `[role, isSigner, isWritable]`.
 */
export function label(ix: TransactionInstruction, entries: Array<[Address, string]>): Meta[] {
  const rev = new Map(entries.map(([a, n]) => [a.toString(), n]));
  return ix.keys.map((k) => {
    const name = rev.get(k.pubkey.toString());
    if (name === undefined) {
      throw new Error(`unmapped account ${k.pubkey.toString()} in ${ix.keys.length}-key ix`);
    }
    return [name, k.isSigner, k.isWritable];
  });
}

/** The fixed program-id accounts, by role. Spread into per-instruction maps. */
export const PROGRAMS: Array<[Address, string]> = [
  [SYSTEM_PROGRAM_ID, "systemProgram"],
  [TOKEN_PROGRAM_ID, "tokenProgram"],
  [ATA_PROGRAM_ID, "ataProgram"],
  [CV_PROGRAM, "cvProgram"],
  [AMM_PROGRAM, "ammProgram"],
];

// Distinct placeholder args reused across the kassandra-market builders.
export const PAYER = A(1);
export const KASS_MINT = A(2);
export const AUTHORITY = A(3);
export const ORACLE = A(4);
export const CREATOR = A(5);
export const CREATOR_ATA = A(6);
export const CONTRIBUTOR = A(7);
export const CONTRIB_ATA = A(8);
export const LP_ATA = A(9);
export const QUESTION = A(10);
export const FEE_DEST = A(11);
export const VAULT = A(12);
export const VAULT_UNDERLYING_ATA = A(13);
export const YES_MINT = A(14);
export const NO_MINT = A(15);
export const MARKET_CYES = A(16);
export const MARKET_CNO = A(17);
export const AMM = A(18);
export const LP_MINT = A(19);
export const LP_VAULT_ACC = A(20);
export const AMM_VAULT_BASE = A(21);
export const AMM_VAULT_QUOTE = A(22);
export const CV_EVENT_AUTH = A(23);
export const AMM_EVENT_AUTH = A(24);

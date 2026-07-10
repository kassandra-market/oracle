/**
 * Decoder for the `Proposer` account (`state.rs::Proposer`, 96 bytes) — a
 * proposer's commitment within an oracle. Field offsets pinned in
 * `programs/oracles/tests/state_layout.rs`.
 */
import { Address } from "@solana/web3.js";

import { AccountType, ACCOUNT_SIZES } from "../constants.js";
import { assertAccount, readBool, readPubkey, readU64LE, readU8, view } from "./common.js";

/** Decoded `Proposer`. */
export interface Proposer {
  accountType: AccountType.Proposer;
  oracle: Address;
  authority: Address;
  /** Locked KASS bond. */
  bond: bigint;
  /** Option value at proposal time. */
  originalOption: number;
  /** Option value after AI claim; `0xFF` (CLAIM_OPTION_NONE) = not yet submitted. */
  claimOption: number;
  disqualified: boolean;
  slashed: boolean;
  /** `claim_option != original_option`. */
  flipped: boolean;
  bump: number;
  /** Settled by `finalize_ai_claims` (idempotency marker). */
  aiFinalized: boolean;
  /** KASS slashed from this proposer into the oracle's `bond_pool`. */
  slashedAmount: bigint;
}

/** Decode a `Proposer` account from its raw bytes. Throws on wrong size or tag. */
export function decodeProposer(data: Uint8Array): Proposer {
  assertAccount(data, AccountType.Proposer, ACCOUNT_SIZES.Proposer, "Proposer");
  const dv = view(data);
  return {
    accountType: AccountType.Proposer,
    oracle: readPubkey(data, 8),
    authority: readPubkey(data, 40),
    bond: readU64LE(dv, 72),
    originalOption: readU8(dv, 80),
    claimOption: readU8(dv, 81),
    disqualified: readBool(dv, 82),
    slashed: readBool(dv, 83),
    flipped: readBool(dv, 84),
    bump: readU8(dv, 85),
    aiFinalized: readBool(dv, 86),
    slashedAmount: readU64LE(dv, 88),
  };
}

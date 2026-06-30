/**
 * Decoder for the `FactVote` account (`state.rs::FactVote`, 88 bytes) — a
 * stake-weighted vote on a fact. Field offsets pinned in
 * `programs/kassandra/tests/state_layout.rs` (`stake` @72).
 */
import { Address } from "@solana/web3.js";

import { AccountType, ACCOUNT_SIZES, VOTE_APPROVE, VOTE_DUPLICATE } from "../constants.js";
import { assertAccount, readPubkey, readU64LE, readU8, view } from "./common.js";

/** Vote kind discriminant (`FactVote.kind`). */
export enum VoteKind {
  Approve = VOTE_APPROVE,
  Duplicate = VOTE_DUPLICATE,
}

/** Decoded `FactVote`. */
export interface FactVote {
  accountType: AccountType.FactVote;
  fact: Address;
  voter: Address;
  stake: bigint;
  /** Raw `kind` discriminant. */
  kindRaw: number;
  /** Decoded {@link VoteKind} (or undefined if the discriminant is unknown). */
  kind: VoteKind | undefined;
  bump: number;
}

/** Decode a `FactVote` account from its raw bytes. Throws on wrong size or tag. */
export function decodeFactVote(data: Uint8Array): FactVote {
  assertAccount(data, AccountType.FactVote, ACCOUNT_SIZES.FactVote, "FactVote");
  const dv = view(data);
  const kindRaw = readU8(dv, 80);
  return {
    accountType: AccountType.FactVote,
    fact: readPubkey(data, 8),
    voter: readPubkey(data, 40),
    stake: readU64LE(dv, 72),
    kindRaw,
    kind: kindRaw === VOTE_APPROVE || kindRaw === VOTE_DUPLICATE ? (kindRaw as VoteKind) : undefined,
    bump: readU8(dv, 81),
  };
}

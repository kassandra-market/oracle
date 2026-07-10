/**
 * Decoder for the `Fact` account (`state.rs::Fact`, 336 bytes) — a fact
 * submitted in support of an option. Carries a fixed `uri: [u8;200]` whose
 * meaningful prefix is the first `uri_len` bytes (UTF-8). Field offsets pinned
 * in `programs/oracles/tests/state_layout.rs` (`uri` @136).
 */
import { Address } from "@solana/web3.js";

import { AccountType, ACCOUNT_SIZES } from "../constants.js";
import {
  assertAccount,
  readBool,
  readBytes,
  readPubkey,
  readU16LE,
  readU64LE,
  readU8,
  view,
} from "./common.js";

const URI_OFFSET = 136;
const URI_CAP = 200;

/** Decoded `Fact`. */
export interface Fact {
  accountType: AccountType.Fact;
  oracle: Address;
  /** Who submitted the fact. */
  proposer: Address;
  /** 32-byte content hash. */
  contentHash: Uint8Array;
  stake: bigint;
  /** Running tally of "approve" votes. */
  approveStake: bigint;
  /** Running tally of "duplicate" votes. */
  duplicateStake: bigint;
  /** Length of the meaningful URI prefix (bytes). */
  uriLen: number;
  /** Set at finalize: accepted. */
  agreed: boolean;
  /** Set at finalize: duplicate-dominant. */
  duplicate: boolean;
  settled: boolean;
  bump: number;
  /** The decoded URI (first `uri_len` bytes of the fixed `[u8;200]`, UTF-8). */
  uri: string;
  /** The full raw 200-byte URI field (for callers that need the untruncated bytes). */
  uriRaw: Uint8Array;
}

/** Decode a `Fact` account from its raw bytes. Throws on wrong size or tag. */
export function decodeFact(data: Uint8Array): Fact {
  assertAccount(data, AccountType.Fact, ACCOUNT_SIZES.Fact, "Fact");
  const dv = view(data);
  const uriLen = readU16LE(dv, 128);
  const uriRaw = readBytes(data, URI_OFFSET, URI_CAP);
  // Clamp to the buffer cap so a corrupt uri_len can't read past the field.
  const sliceLen = Math.min(uriLen, URI_CAP);
  const uri = new TextDecoder().decode(uriRaw.subarray(0, sliceLen));
  return {
    accountType: AccountType.Fact,
    oracle: readPubkey(data, 8),
    proposer: readPubkey(data, 40),
    contentHash: readBytes(data, 72, 32),
    stake: readU64LE(dv, 104),
    approveStake: readU64LE(dv, 112),
    duplicateStake: readU64LE(dv, 120),
    uriLen,
    agreed: readBool(dv, 130),
    duplicate: readBool(dv, 131),
    settled: readBool(dv, 132),
    bump: readU8(dv, 133),
    uri,
    uriRaw,
  };
}

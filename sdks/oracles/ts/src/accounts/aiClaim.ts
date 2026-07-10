/**
 * Decoder for the `AiClaim` account (`state.rs::AiClaim`, 208 bytes) — a
 * pinned-model AI claim for a proposer's option. `authority` was appended at
 * offset 176 (S4). Field offsets pinned in
 * `programs/oracles/tests/state_layout.rs` (`io_hash` @136, `authority` @176).
 */
import { Address } from "@solana/web3.js";

import { AccountType, ACCOUNT_SIZES } from "../constants.js";
import {
  assertAccount,
  readBool,
  readBytes,
  readPubkey,
  readU8,
  view,
} from "./common.js";

/** Decoded `AiClaim`. */
export interface AiClaim {
  accountType: AccountType.AiClaim;
  oracle: Address;
  proposer: Address;
  /** Hash/ident of the pinned model (32 bytes). */
  modelId: Uint8Array;
  /** Hash of declared params — temp, seed, ... (32 bytes). */
  paramsHash: Uint8Array;
  /** hash(prompt + agreed facts + raw response) (32 bytes). */
  ioHash: Uint8Array;
  option: number;
  challenged: boolean;
  bump: number;
  /** The proposer's human authority (== `proposer.authority`), stamped at submit. */
  authority: Address;
}

/** Decode an `AiClaim` account from its raw bytes. Throws on wrong size or tag. */
export function decodeAiClaim(data: Uint8Array): AiClaim {
  assertAccount(data, AccountType.AiClaim, ACCOUNT_SIZES.AiClaim, "AiClaim");
  const dv = view(data);
  return {
    accountType: AccountType.AiClaim,
    oracle: readPubkey(data, 8),
    proposer: readPubkey(data, 40),
    modelId: readBytes(data, 72, 32),
    paramsHash: readBytes(data, 104, 32),
    ioHash: readBytes(data, 136, 32),
    option: readU8(dv, 168),
    challenged: readBool(dv, 169),
    bump: readU8(dv, 170),
    authority: readPubkey(data, 176),
  };
}

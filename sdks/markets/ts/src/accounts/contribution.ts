/**
 * Decoder for the `Contribution` account (`state.rs::Contribution`, 88 bytes) —
 * one contributor's recorded KASS stake in a market, the source of both the
 * `refund` (Cancelled) and `claim_lp` (Active) pro-rata payouts.
 * Field offsets pinned in `programs/markets/tests/state_layout.rs`.
 */
import { Address } from "@solana/web3.js";

import { AccountType, ACCOUNT_SIZES } from "../constants.js";
import { assertAccount, readBool, readPubkey, readU64LE, readU8, view } from "./common.js";

/** Decoded `Contribution`. `u64` fields are `bigint`; keys are `Address`. */
export interface Contribution {
  accountType: AccountType.Contribution;
  /** The market this stake belongs to. */
  market: Address;
  /** The contributor who staked. */
  contributor: Address;
  /** KASS staked (raw base units). */
  amount: bigint;
  /** True once the refund/LP claim consumed this contribution. */
  claimed: boolean;
  /** Contribution PDA bump. */
  bump: number;
}

/** Decode a `Contribution` account from its raw bytes. Throws on wrong size or tag. */
export function decodeContribution(data: Uint8Array): Contribution {
  assertAccount(data, AccountType.Contribution, ACCOUNT_SIZES.Contribution, "Contribution");
  const dv = view(data);
  return {
    accountType: AccountType.Contribution,
    market: readPubkey(data, 8),
    contributor: readPubkey(data, 40),
    amount: readU64LE(dv, 72),
    claimed: readBool(dv, 80),
    bump: readU8(dv, 81),
  };
}

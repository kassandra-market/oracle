/**
 * Decoder for the `Config` singleton account (`state.rs::Config`, 120 bytes) — the
 * program's global record: the futarchy authority gating `update_config`, the
 * canonical KASS mint every market escrows, and the funding-target floor.
 * Field offsets pinned in `programs/kassandra-market/tests/state_layout.rs`.
 */
import { Address } from "@solana/web3.js";

import { AccountType, ACCOUNT_SIZES } from "../constants.js";
import { assertAccount, readPubkey, readU16LE, readU64LE, readU8, view } from "./common.js";

/** Decoded `Config`. `u64` fields are `bigint`; keys are `Address`. */
export interface Config {
  accountType: AccountType.Config;
  /** Futarchy authority permitted to run `update_config`. */
  authority: Address;
  /** Canonical KASS mint every market escrows and splits. */
  kassMint: Address;
  /** Minimum KASS a market must raise before it can be activated. */
  minLiquidity: bigint;
  /** Config PDA bump. */
  bump: number;
  /** Governance-set protocol fee in basis points (<= MAX_FEE_BPS). */
  feeBps: number;
  /** KASS token account protocol fees are routed to. */
  feeDestination: Address;
}

/** Decode a `Config` account from its raw bytes. Throws on wrong size or tag. */
export function decodeConfig(data: Uint8Array): Config {
  assertAccount(data, AccountType.Config, ACCOUNT_SIZES.Config, "Config");
  const dv = view(data);
  return {
    accountType: AccountType.Config,
    authority: readPubkey(data, 8),
    kassMint: readPubkey(data, 40),
    minLiquidity: readU64LE(dv, 72),
    bump: readU8(dv, 80),
    feeBps: readU16LE(dv, 82),
    feeDestination: readPubkey(data, 84),
  };
}

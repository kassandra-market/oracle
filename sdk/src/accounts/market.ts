/**
 * Decoder for the `Market` account (`state.rs::Market`, 416 bytes) — a challenge
 * decision-market binding for one `AiClaim`. Records the MetaDAO accounts the
 * challenger composed plus the on-chain escrow/destination accounts. Field
 * offsets pinned in `programs/kassandra/tests/state_layout.rs`.
 */
import { Address } from "@solana/web3.js";

import { AccountType, ACCOUNT_SIZES } from "../constants.js";
import { assertAccount, readBool, readI64LE, readPubkey, readU64LE, readU8, view } from "./common.js";

/** Decoded `Market`. */
export interface Market {
  accountType: AccountType.Market;
  oracle: Address;
  aiClaim: Address;
  proposer: Address;
  challenger: Address;
  /** MetaDAO binary question (resolver == oracle PDA). */
  question: Address;
  /** MetaDAO conditional vault, underlying == oracle.kass_mint. */
  kassVault: Address;
  /** MetaDAO conditional vault, underlying == oracle.usdc_mint. */
  usdcVault: Address;
  /** Outcome-0 (pass) AMM. */
  passAmm: Address;
  /** Outcome-1 (fail) AMM. */
  failAmm: Address;
  /** Oracle-PDA-owned conditional-KASS token account (pass). */
  oraclePassKass: Address;
  /** Oracle-PDA-owned conditional-KASS token account (fail). */
  oracleFailKass: Address;
  /** Market-owned USDC escrow holding the challenger's staked USDC. */
  challengerUsdcVault: Address;
  /** `now + oracle.twap_window`; settle allowed only after this. */
  twapEnd: bigint;
  /** Challenger's escrowed USDC (raw base units). */
  challengerUsdc: bigint;
  /** Set by `settle_challenge`. */
  settled: boolean;
  bump: number;
}

/** Decode a `Market` account from its raw bytes. Throws on wrong size or tag. */
export function decodeMarket(data: Uint8Array): Market {
  assertAccount(data, AccountType.Market, ACCOUNT_SIZES.Market, "Market");
  const dv = view(data);
  return {
    accountType: AccountType.Market,
    oracle: readPubkey(data, 8),
    aiClaim: readPubkey(data, 40),
    proposer: readPubkey(data, 72),
    challenger: readPubkey(data, 104),
    question: readPubkey(data, 136),
    kassVault: readPubkey(data, 168),
    usdcVault: readPubkey(data, 200),
    passAmm: readPubkey(data, 232),
    failAmm: readPubkey(data, 264),
    oraclePassKass: readPubkey(data, 296),
    oracleFailKass: readPubkey(data, 328),
    challengerUsdcVault: readPubkey(data, 360),
    twapEnd: readI64LE(dv, 392),
    challengerUsdc: readU64LE(dv, 400),
    settled: readBool(dv, 408),
    bump: readU8(dv, 409),
  };
}

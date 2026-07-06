/**
 * Decoder for the `Market` account (`state.rs::Market`, 400 bytes) — one binary
 * KASS prediction market bound to a Kassandra oracle. Records the funding-phase
 * escrow/accounting fields plus the Phase-2 MetaDAO composition bindings
 * (question / vault / cYES / cNO / amm / lp) written at `activate`.
 * Field offsets pinned in `programs/kassandra-market/tests/state_layout.rs`.
 */
import { Address } from "@solana/web3.js";

import { AccountType, ACCOUNT_SIZES, MarketStatus } from "../constants.js";
import {
  assertAccount,
  readBool,
  readPubkey,
  readU16LE,
  readU64LE,
  readU8,
  view,
} from "./common.js";

/** Decoded `Market`. `u64` fields are `bigint`; keys are `Address`. */
export interface Market {
  accountType: AccountType.Market;
  /** The Kassandra oracle this market resolves against. */
  oracle: Address;
  /** Creator (seeded the first contribution). */
  creator: Address;
  /** Canonical KASS mint (== `config.kass_mint`). */
  kassMint: Address;
  /** Market-PDA-owned KASS escrow holding all contributions. */
  escrowVault: Address;
  /** Funding-target floor snapshotted from `config.min_liquidity`. */
  minLiquidity: bigint;
  /** Running total of KASS contributed so far. */
  totalContributed: bigint;
  /**
   * Count of live `Contribution` accounts (`open_contributions`, u16 @152).
   * `create_market` sets it to 1; `contribute` increments it when it creates a new
   * Contribution; `claim_lp`/`refund` decrement it (and close the Contribution).
   * `close_market` requires this to be 0 (every contributor has exited).
   */
  openContributions: number;
  /** Lifecycle status. */
  status: MarketStatus;
  /** Market PDA bump. */
  bump: number;
  /** Escrow PDA bump. */
  escrowBump: number;
  // ---- Phase-2 MetaDAO bindings (zeroed until `activate`) --------------------
  /** MetaDAO Question (resolver == this market PDA). */
  question: Address;
  /** KASS conditional vault. */
  vault: Address;
  /** cYES conditional mint (idx 0). */
  yesMint: Address;
  /** cNO conditional mint (idx 1). */
  noMint: Address;
  /** cYES/cNO AMM pool. */
  amm: Address;
  /** The AMM pool's LP mint. */
  lpMint: Address;
  /** Market-PDA-owned LP holder seeded at `activate`. */
  lpVault: Address;
  /** Total LP minted into `lpVault` at `activate` (the `claim_lp` numerator). */
  lpTotal: bigint;
  /** Set once the market's MetaDAO question is resolved. */
  settled: boolean;
  /** Protocol fee in basis points, snapshotted from `config.fee_bps` at creation. */
  feeBps: number;
  /**
   * Set once the protocol fee has been collected (or found to be nil). `claim_lp`
   * is gated on this: resolve → collect_fee → claim_lp. Stamped by `resolve_market`
   * directly when `feeBps == 0 || lpTotal == 0`, else by the `collect_fee` crank.
   */
  feeCollected: boolean;
  /**
   * The oracle outcome this sub-market binds to (YES = the oracle resolves to
   * this index). Binary markets are `0`; snapshotted at create. Read from @397.
   */
  outcomeIndex: number;
}

/** Decode a `Market` account from its raw bytes. Throws on wrong size or tag. */
export function decodeMarket(data: Uint8Array): Market {
  assertAccount(data, AccountType.Market, ACCOUNT_SIZES.Market, "Market");
  const dv = view(data);
  return {
    accountType: AccountType.Market,
    oracle: readPubkey(data, 8),
    creator: readPubkey(data, 40),
    kassMint: readPubkey(data, 72),
    escrowVault: readPubkey(data, 104),
    minLiquidity: readU64LE(dv, 136),
    totalContributed: readU64LE(dv, 144),
    openContributions: readU16LE(dv, 152), // u16 @152 (live-Contribution counter)
    status: readU8(dv, 154) as MarketStatus,
    bump: readU8(dv, 155),
    escrowBump: readU8(dv, 156),
    question: readPubkey(data, 160),
    vault: readPubkey(data, 192),
    yesMint: readPubkey(data, 224),
    noMint: readPubkey(data, 256),
    amm: readPubkey(data, 288),
    lpMint: readPubkey(data, 320),
    lpVault: readPubkey(data, 352),
    lpTotal: readU64LE(dv, 384),
    settled: readBool(dv, 392),
    feeBps: readU16LE(dv, 394),
    feeCollected: readBool(dv, 396),
    outcomeIndex: readU8(dv, 397),
  };
}

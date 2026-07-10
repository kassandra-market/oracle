/**
 * Decoder + pure price/TWAP/margin helpers for the MetaDAO **v0.4 `Amm`**
 * account — the conditional-market pool the challenger composes for each
 * pass/fail outcome. Read-only: the app never writes these (CU2 does trade).
 *
 * The SDK ships v0.4 amm BUILDERS + PDAs but NO `Amm` DECODER, so the app adds
 * one here, mirroring the on-chain reader EXACTLY. The authoritative layout +
 * offset consts live in `programs/oracles/src/cpi/metadao.rs:145-178`
 * (verified against `metaDAOproject/programs/amm/src/state/amm.rs`, the deployed
 * delayed-twap v0.4.1/v0.4.2 binary). The account is an Anchor `#[account]`
 * (8-byte discriminator first), Borsh-encoded — sequential little-endian, no
 * alignment padding. Field map (byte offsets):
 *
 *   disc[8] | bump:u8 @8 | created_at_slot:u64 @9 | lp_mint:Pubkey @17
 *   | base_mint:Pubkey @49 | quote_mint:Pubkey @81 | base_mint_decimals:u8 @113
 *   | quote_mint_decimals:u8 @114 | base_amount:u64 @115 | quote_amount:u64 @123
 *   | oracle.last_updated_slot:u64 @131 | oracle.last_price:u128 @139
 *   | oracle.last_observation:u128 @155 | oracle.aggregator:u128 @171
 *   | oracle.max_observation_change_per_update:u128 @187
 *   | oracle.initial_observation:u128 @203 | oracle.start_delay_slots:u64 @219
 *   | seq_num:u64 @227
 *
 * `get_twap()` (v0.4.2 source, mirrored by `settle_challenge`) computes
 *   `aggregator / (last_updated_slot - (created_at_slot + start_delay_slots))`
 * — a slot-weighted average of the quote/base price already scaled by
 * PRICE_SCALE = 1e12. So the returned TWAP is a PRICE_SCALE-scaled integer.
 */
import type { Connection } from "@solana/web3.js";
import { Address } from "@solana/web3.js";
import { ammV04, type Market } from "@kassandra-market/oracles";

/**
 * Anchor account discriminator for `Amm` (`sha256("account:Amm")[..8]`).
 * Re-exported from the SDK so the value is single-sourced there.
 */
export const AMM_ACCOUNT_DISCRIMINATOR = ammV04.AMM_ACCOUNT_DISCRIMINATOR;

/** `Amm.created_at_slot: u64` — byte offset. */
export const AMM_CREATED_AT_SLOT_OFFSET = 9;
/** `Amm.base_mint: Pubkey` — byte offset. */
export const AMM_BASE_MINT_OFFSET = 49;
/** `Amm.quote_mint: Pubkey` — byte offset. */
export const AMM_QUOTE_MINT_OFFSET = 81;
/** `Amm.base_mint_decimals: u8` — byte offset. */
export const AMM_BASE_DECIMALS_OFFSET = 113;
/** `Amm.quote_mint_decimals: u8` — byte offset. */
export const AMM_QUOTE_DECIMALS_OFFSET = 114;
/** `Amm.base_amount: u64` — byte offset. */
export const AMM_BASE_AMOUNT_OFFSET = 115;
/** `Amm.quote_amount: u64` — byte offset. */
export const AMM_QUOTE_AMOUNT_OFFSET = 123;
/** `Amm.oracle.last_updated_slot: u64` — byte offset. */
export const AMM_LAST_UPDATED_SLOT_OFFSET = 131;
/** `Amm.oracle.aggregator: u128` — byte offset. */
export const AMM_AGGREGATOR_OFFSET = 171;
/** `Amm.oracle.start_delay_slots: u64` — byte offset (v0.4.1+ delayed-twap). */
export const AMM_START_DELAY_SLOTS_OFFSET = 219;
/** Smallest `Amm` data length covering every field the TWAP read touches. */
export const AMM_MIN_LEN = AMM_START_DELAY_SLOTS_OFFSET + 8; // 227

/**
 * The on-chain price fixed-point scale (`PRICE_SCALE = 1e12`). The AMM's
 * aggregator accumulates `price * PRICE_SCALE` per slot, so {@link twapPrice}
 * returns an integer already scaled by this. Divide by it for a human price.
 */
export const PRICE_SCALE = 1_000_000_000_000n;

/** A decoded v0.4 `Amm` account (only the fields the challenge viz needs). */
export interface AmmV04 {
  /** Base (conditional-KASS) mint. */
  baseMint: Address;
  /** Quote (conditional-USDC) mint. */
  quoteMint: Address;
  /** Base mint decimals. */
  baseDecimals: number;
  /** Quote mint decimals. */
  quoteDecimals: number;
  /** Base reserve (raw base units). */
  baseAmount: bigint;
  /** Quote reserve (raw base units). */
  quoteAmount: bigint;
  /** Slot the pool was created at. */
  createdAtSlot: bigint;
  /** Slot the TWAP oracle was last updated (cranked). */
  lastUpdatedSlot: bigint;
  /** Slots after creation before the TWAP starts accumulating. */
  startDelaySlots: bigint;
  /** Slot-weighted price accumulator (u128, PRICE_SCALE-scaled per slot). */
  aggregator: bigint;
}

/** Thrown by {@link decodeAmmV04} on a wrong discriminator or short buffer. */
export class AmmDecodeError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "AmmDecodeError";
  }
}

/** Read a little-endian u128 at `off` (low 64 ++ high 64). */
function readU128LE(dv: DataView, off: number): bigint {
  return dv.getBigUint64(off, true) | (dv.getBigUint64(off + 8, true) << 64n);
}

/** Read a 32-byte pubkey at `off` as a web3.js {@link Address}. */
function readPubkey(data: Uint8Array, off: number): Address {
  return new Address(data.slice(off, off + 32));
}

/**
 * Decode a v0.4 `Amm` account from raw bytes. Guards the 8-byte discriminator
 * ({@link AMM_ACCOUNT_DISCRIMINATOR}) + a `>= AMM_MIN_LEN` length before reading
 * any field, throwing {@link AmmDecodeError} on a mismatch (so a wrong / short
 * account is rejected instead of silently misread). Offsets mirror
 * `cpi/metadao.rs` + the test-local `decodeAmmTwap`.
 */
export function decodeAmmV04(data: Uint8Array): AmmV04 {
  if (data.length < AMM_MIN_LEN) {
    throw new AmmDecodeError(
      `Amm: buffer too short — need >= ${AMM_MIN_LEN} bytes, got ${data.length}.`,
    );
  }
  for (let i = 0; i < AMM_ACCOUNT_DISCRIMINATOR.length; i++) {
    if (data[i] !== AMM_ACCOUNT_DISCRIMINATOR[i]) {
      throw new AmmDecodeError("Amm: wrong account discriminator.");
    }
  }
  const dv = new DataView(data.buffer, data.byteOffset, data.byteLength);
  return {
    baseMint: readPubkey(data, AMM_BASE_MINT_OFFSET),
    quoteMint: readPubkey(data, AMM_QUOTE_MINT_OFFSET),
    baseDecimals: dv.getUint8(AMM_BASE_DECIMALS_OFFSET),
    quoteDecimals: dv.getUint8(AMM_QUOTE_DECIMALS_OFFSET),
    baseAmount: dv.getBigUint64(AMM_BASE_AMOUNT_OFFSET, true),
    quoteAmount: dv.getBigUint64(AMM_QUOTE_AMOUNT_OFFSET, true),
    createdAtSlot: dv.getBigUint64(AMM_CREATED_AT_SLOT_OFFSET, true),
    lastUpdatedSlot: dv.getBigUint64(AMM_LAST_UPDATED_SLOT_OFFSET, true),
    startDelaySlots: dv.getBigUint64(AMM_START_DELAY_SLOTS_OFFSET, true),
    aggregator: readU128LE(dv, AMM_AGGREGATOR_OFFSET),
  };
}

/**
 * Instantaneous spot price = quote per base, decimals-adjusted, as a display
 * `Number` (`quote/base` corrected for the mints' decimal scales). Returns
 * `null` when the base reserve is empty (price undefined). The RAW reserves stay
 * available on the {@link AmmV04} (never lose precision on those — show them via
 * `groupDigits` as bigints); this ratio is for the human "≈ X USDC/KASS" label.
 */
export function instantaneousPrice(amm: AmmV04): number | null {
  if (amm.baseAmount === 0n) return null;
  const raw = Number(amm.quoteAmount) / Number(amm.baseAmount);
  return raw * 10 ** (amm.baseDecimals - amm.quoteDecimals);
}

/**
 * Time-weighted price = `aggregator / (last_updated - (created_at + start_delay))`,
 * mirroring the AMM `get_twap()` + `settle_challenge`. The result is an integer
 * already scaled by {@link PRICE_SCALE} (1e12). Returns `null` when the TWAP is
 * not yet meaningful — pre-start-delay / no elapsed slots (div-by-zero guard) or
 * a zero aggregator (no observations yet). Mirrors `decodeAmmTwap` exactly.
 */
export function twapPrice(amm: AmmV04): bigint | null {
  const startSlot = amm.createdAtSlot + amm.startDelaySlots;
  const slots = amm.lastUpdatedSlot - startSlot;
  if (slots <= 0n || amm.aggregator <= 0n) return null;
  return amm.aggregator / slots;
}

/**
 * How close the FAIL TWAP is to clearing the disqualify margin over the PASS
 * TWAP, as a `0..1+` progress for the bar. `settle_challenge` disqualifies the
 * proposer (fraud) iff
 *
 *   `fail_twap * marginDen > pass_twap * (marginDen + marginNum)`
 *   ⟺  `(fail_twap - pass_twap) * marginDen > pass_twap * marginNum`
 *
 * i.e. FAIL must exceed PASS by the relative margin `marginNum/marginDen`. So a
 * meaningful `0..1+` progress is the divergence over that required margin:
 *
 *   progress = `(fail_twap - pass_twap) * marginDen / (pass_twap * marginNum)`
 *
 * — `0` at no divergence (fail ≤ pass), `1` exactly at the margin, `> 1` once
 * FAIL has cleared it (settle would disqualify). Guards nulls, a zero/absent
 * PASS TWAP (which ALWAYS survives on-chain), and a below-pass FAIL → `0`. Uses
 * `Number` only for the final ratio; the bigint products keep full precision.
 */
export function marginProgress(
  failTwap: bigint | null,
  passTwap: bigint | null,
  marginNum: bigint,
  marginDen: bigint,
): number {
  if (failTwap === null || passTwap === null) return 0;
  if (passTwap <= 0n) return 0; // zero pass price always survives (settle guard)
  if (failTwap <= passTwap) return 0; // no divergence toward disqualify
  const need = passTwap * marginNum;
  if (need <= 0n) return 0;
  const diverge = (failTwap - passTwap) * marginDen;
  return Number(diverge) / Number(need);
}

/**
 * The settle verdict, mirroring `settle_challenge.rs` EXACTLY: disqualify iff
 * `fail_twap * DEN > pass_twap * (DEN + NUM)` (strict `>`). Prefer this over
 * thresholding the float {@link marginProgress} at `>= 1` — the bigint compare
 * matches the on-chain boundary precisely, so at exact equality (measure-zero on
 * PRICE_SCALE integers) the proposer SURVIVES, as the program decides. Returns
 * `false` when a TWAP is unavailable or the PASS price is zero (the settle guard
 * — a market with no pass signal always survives).
 */
export function willDisqualify(
  failTwap: bigint | null,
  passTwap: bigint | null,
  marginNum: bigint,
  marginDen: bigint,
): boolean {
  if (failTwap === null || passTwap === null) return false;
  if (passTwap <= 0n) return false; // zero pass price always survives (settle guard)
  return failTwap * marginDen > passTwap * (marginDen + marginNum);
}

/** The decoded pass/fail pools of one {@link Market} (`null` when absent). */
export interface MarketAmms {
  pass: AmmV04 | null;
  fail: AmmV04 | null;
}

/**
 * Fetch + decode a market's pass/fail v0.4 `Amm` accounts over `connection`.
 * A missing account (or one that fails the discriminator/size guard) yields
 * `null` for that side rather than throwing, so the panel degrades gracefully.
 * RPC errors propagate to the caller (a hook renders the error/empty state).
 */
export async function fetchMarketAmms(
  connection: Connection,
  market: Market,
): Promise<MarketAmms> {
  const decodeOrNull = (data: Uint8Array | undefined): AmmV04 | null => {
    if (!data || data.length === 0) return null;
    try {
      return decodeAmmV04(data);
    } catch {
      return null;
    }
  };
  const [passInfo, failInfo] = await Promise.all([
    connection.getAccountInfo(market.passAmm),
    connection.getAccountInfo(market.failAmm),
  ]);
  return {
    pass: decodeOrNull(passInfo?.data),
    fail: decodeOrNull(failInfo?.data),
  };
}

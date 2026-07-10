/**
 * Zero-copy decoders for Meteora **DAMM v2** (cp-amm) `Pool` + `Position`.
 *
 * Both are Anchor `#[account(zero_copy)]` `#[repr(C)]` structs: an 8-byte
 * discriminator prefix, then C-layout fields at FIXED little-endian offsets. The
 * offsets below are RE-DERIVED from the pinned field order
 * (commit `bdd8a1e355f484b3cff131578a662c560b97b72f`, `state/{pool,position}.rs`);
 * every `u128` sits on a 16-byte boundary via the explicit `padding_*` fields, so
 * the arithmetic is exact.
 *
 * Pool absolute offsets (struct-relative + 8 for the disc):
 *   pool_fees(160) @8 · token_a_mint @168 · token_b_mint @200 · token_a_vault @232
 *   · token_b_vault @264 · whitelisted_vault @296 · padding_0[32] @328
 *   · liquidity @360 · padding_1 @376 · protocol_a_fee @392 · protocol_b_fee @400
 *   · padding_2 @408 · sqrt_min_price @424 · sqrt_max_price @440 · SQRT_PRICE @456
 *   · activation_point @472 · … · creator @648 · token_a_amount @680
 *   · token_b_amount @688 · … · reward_infos[2] @728..1112.  (INIT_SPACE 1104; on-chain 1112.)
 *   => sqrt_price is at STRUCT-offset 448 / ABSOLUTE 456, as pinned in the plan.
 *
 * Position absolute offsets:
 *   pool @8 · nft_mint @40 · fee_a_checkpoint[32] @72 · fee_b_checkpoint[32] @104
 *   · fee_a_pending @136 · fee_b_pending @144 · unlocked_liquidity @152
 *   · vested_liquidity @168 · permanent_locked_liquidity @184.  (INIT_SPACE 400; on-chain 408.)
 */
import { Address } from "@solana/web3.js";

import { readPubkey, readU64LE, readU128LE, view } from "../accounts/common.js";
import {
  POOL_ACCOUNT_DISCRIMINATOR,
  POOL_ACCOUNT_SIZE,
  POSITION_ACCOUNT_DISCRIMINATOR,
  POSITION_ACCOUNT_SIZE,
} from "./constants.js";

/**
 * Validate an Anchor account buffer: exact `size` (`8 + INIT_SPACE`) and a
 * matching 8-byte discriminator prefix. Wrong size/disc → throw (type-confusion
 * guard), mirroring `assertAccount` for the Kassandra Pod accounts.
 */
function assertAnchorAccount(
  data: Uint8Array,
  disc: Uint8Array,
  size: number,
  name: string,
): void {
  if (data.length !== size) {
    throw new Error(`${name}: wrong account size — expected ${size} bytes, got ${data.length}.`);
  }
  for (let i = 0; i < 8; i++) {
    if (data[i] !== disc[i]) {
      throw new Error(`${name}: wrong account discriminator (first 8 bytes).`);
    }
  }
}

/** Decoded cp-amm `Pool` (spot-relevant fields). */
export interface MeteoraPool {
  tokenAMint: Address;
  tokenBMint: Address;
  tokenAVault: Address;
  tokenBVault: Address;
  /** Total liquidity share (u128). */
  liquidity: bigint;
  /** Min price bound (u128, Q64.64). */
  sqrtMinPrice: bigint;
  /** Max price bound (u128, Q64.64). */
  sqrtMaxPrice: bigint;
  /** Current price as sqrt(token_b/token_a) Q64.64 (u128) — struct-offset 448 / abs 456. */
  sqrtPrice: bigint;
  /** Unclaimed protocol fee in token A (u64). */
  protocolAFee: bigint;
  /** Unclaimed protocol fee in token B (u64). */
  protocolBFee: bigint;
  /** Pool creator. */
  creator: Address;
  /** Tracked token-A reserve (u64; meaningful when layout_version >= 1). */
  tokenAAmount: bigint;
  /** Tracked token-B reserve (u64; meaningful when layout_version >= 1). */
  tokenBAmount: bigint;
}

/** Decode a cp-amm `Pool` account. Throws on wrong size or discriminator. */
export function decodePool(data: Uint8Array): MeteoraPool {
  assertAnchorAccount(data, POOL_ACCOUNT_DISCRIMINATOR, POOL_ACCOUNT_SIZE, "Pool");
  const dv = view(data);
  return {
    tokenAMint: readPubkey(data, 168),
    tokenBMint: readPubkey(data, 200),
    tokenAVault: readPubkey(data, 232),
    tokenBVault: readPubkey(data, 264),
    liquidity: readU128LE(dv, 360),
    sqrtMinPrice: readU128LE(dv, 424),
    sqrtMaxPrice: readU128LE(dv, 440),
    sqrtPrice: readU128LE(dv, 456),
    protocolAFee: readU64LE(dv, 392),
    protocolBFee: readU64LE(dv, 400),
    creator: readPubkey(data, 648),
    tokenAAmount: readU64LE(dv, 680),
    tokenBAmount: readU64LE(dv, 688),
  };
}

/** Decoded cp-amm `Position` (spot-relevant fields). */
export interface MeteoraPosition {
  /** The Pool this position belongs to. */
  pool: Address;
  /** The position NFT mint (ownership token). */
  nftMint: Address;
  /** Pending (unclaimed) fee in token A (u64). */
  feeAPending: bigint;
  /** Pending (unclaimed) fee in token B (u64). */
  feeBPending: bigint;
  /** Withdrawable liquidity (u128). */
  unlockedLiquidity: bigint;
  /** Liquidity still vesting (u128). */
  vestedLiquidity: bigint;
  /** Permanently-locked liquidity (u128). */
  permanentLockedLiquidity: bigint;
}

/** Decode a cp-amm `Position` account. Throws on wrong size or discriminator. */
export function decodePosition(data: Uint8Array): MeteoraPosition {
  assertAnchorAccount(data, POSITION_ACCOUNT_DISCRIMINATOR, POSITION_ACCOUNT_SIZE, "Position");
  const dv = view(data);
  return {
    pool: readPubkey(data, 8),
    nftMint: readPubkey(data, 40),
    feeAPending: readU64LE(dv, 136),
    feeBPending: readU64LE(dv, 144),
    unlockedLiquidity: readU128LE(dv, 152),
    vestedLiquidity: readU128LE(dv, 168),
    permanentLockedLiquidity: readU128LE(dv, 184),
  };
}

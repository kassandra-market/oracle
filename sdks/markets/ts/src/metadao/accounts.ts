/**
 * MetaDAO account readers — byte-offset decoders over raw account data.
 *
 * These accounts are Anchor `#[account]`s (8-byte discriminator + Borsh
 * little-endian fields), owned by the MetaDAO programs, so kassandra-market only
 * ever READS them. Offsets are pinned against the deployed v0.4.x `amm`
 * (`base_amount:u64 @115`, `quote_amount:u64 @123`); see {@link constants}.
 */
import {
  AMM_ACCOUNT_DISCRIMINATOR,
  AMM_BASE_AMOUNT_OFFSET,
  AMM_QUOTE_AMOUNT_OFFSET,
} from "./constants.js";

/** The two reserve balances of a MetaDAO cYES/cNO `Amm` pool (raw base units). */
export interface AmmReserves {
  /** cYES (base) reserve. */
  base: bigint;
  /** cNO (quote) reserve. */
  quote: bigint;
}

/** Smallest `Amm` length covering both reserve reads. */
const AMM_MIN_LEN = AMM_QUOTE_AMOUNT_OFFSET + 8; // 131

/**
 * Decode a MetaDAO v0.4 `Amm` account's cYES/cNO reserves from its raw data.
 * Verifies the 8-byte account discriminator + a `>= AMM_MIN_LEN` length before
 * reading `base_amount` / `quote_amount` (little-endian u64). Returns `null` on a
 * wrong-discriminator or too-short buffer (the pool only exists once Active), so
 * callers degrade gracefully.
 *
 * base = cYES, quote = cNO (the AMM is created with `baseMint = yesMint`,
 * `quoteMint = noMint`), so the implied YES probability is `quote/(base+quote)`.
 */
export function decodeAmmReserves(data: Uint8Array): AmmReserves | null {
  if (data.length < AMM_MIN_LEN) return null;
  for (let i = 0; i < AMM_ACCOUNT_DISCRIMINATOR.length; i++) {
    if (data[i] !== AMM_ACCOUNT_DISCRIMINATOR[i]) return null;
  }
  const dv = new DataView(data.buffer, data.byteOffset, data.byteLength);
  return {
    base: dv.getBigUint64(AMM_BASE_AMOUNT_OFFSET, true),
    quote: dv.getBigUint64(AMM_QUOTE_AMOUNT_OFFSET, true),
  };
}

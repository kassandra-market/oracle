/** Shared helpers for the high-level flows. */

/**
 * Coerce an {@link AddressInput} (base58 string or `Address`) to an `Address`.
 * Re-exports the canonical coercion from the instruction builders so there is a
 * single implementation.
 */
export { addr as toAddr } from "../instructions/payload.js";

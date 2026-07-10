/**
 * Base58 (Bitcoin alphabet) byte codec, backed by the `bs58` package.
 *
 * `@solana/web3.js` (the `Address`-class 3.0 build this app targets) exposes no
 * byte-array base58 helper — an `Address` can only stringify itself — so the
 * account_type memcmp tag, the dev-wallet secret export, and the unit tests go
 * through `bs58` here rather than a hand-rolled alphabet.
 */
import bs58 from "bs58";

/** Base58-encode raw bytes. Leading zero bytes map to leading `"1"`s. */
export function base58Encode(bytes: Uint8Array): string {
  return bs58.encode(bytes);
}

/** Base58-decode a string back to bytes (inverse of {@link base58Encode}). */
export function base58Decode(s: string): Uint8Array {
  return bs58.decode(s);
}

/**
 * Interop bridge between `@solana/web3.js@3.0.0-rc.2` (the classic/legacy API)
 * and `litesvm` (which speaks `@solana/kit` types).
 *
 * See `NOTES-api.md` for the full recon. The short version:
 *
 * - `@solana/web3.js@3.0.0-rc.2` is the LEGACY v1-style API (`Transaction`,
 *   `TransactionInstruction`, `Keypair`, `Address`) reimplemented on top of
 *   `@solana/kit`. Its `Transaction` is a mutable builder class, signed and
 *   serialized to wire bytes (both `sign()` and `serialize()` are async).
 * - `litesvm.sendTransaction(tx)` expects a kit `Transaction`
 *   (`{ messageBytes, signatures }`), NOT the legacy `Transaction` class.
 *
 * The bridge: build + sign with web3.js, `serialize()` to wire bytes, then
 * decode those bytes into a kit `Transaction` with kit's `getTransactionDecoder`.
 * web3.js v3 and litesvm both resolve to the SAME `@solana/kit` instance, so the
 * decoded `Transaction` is structurally and nominally the type litesvm wants.
 */
import type { Transaction as LegacyTransaction } from "@solana/web3.js";
import { getTransactionDecoder, type Transaction as KitTransaction } from "@solana/kit";

/**
 * Convert a signed legacy web3.js v3 `Transaction` into the kit `Transaction`
 * object that `litesvm.sendTransaction` / `simulateTransaction` accept.
 *
 * The transaction must already have `feePayer`, `recentBlockhash`, and all
 * required signatures set (i.e. `await tx.sign(...)` has run).
 */
export async function toLiteSvmTransaction(tx: LegacyTransaction): Promise<KitTransaction> {
  const wireBytes = await tx.serialize();
  return getTransactionDecoder().decode(wireBytes);
}

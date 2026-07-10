/**
 * surfpool-specific instruction / encoding helpers.
 *
 * Extracted from the original `surfpool/harness.ts`.
 */
import { Address, TransactionInstruction } from "@solana/web3.js";

import { TOKEN_PROGRAM_ID } from "../../../src/constants.js";

/** Hex-encode a byte array for the `surfnet_setAccount` `data` field. */
export function toHex(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString("hex");
}

/** Build an SPL Token `Transfer` instruction (disc 3 ++ amount u64 LE). */
export function splTransfer(
  source: Address,
  dest: Address,
  authority: Address,
  amount: bigint,
): TransactionInstruction {
  const data = new Uint8Array(9);
  data[0] = 3; // Transfer
  new DataView(data.buffer).setBigUint64(1, amount, true);
  return new TransactionInstruction({
    programId: TOKEN_PROGRAM_ID,
    keys: [
      { pubkey: source, isSigner: false, isWritable: true },
      { pubkey: dest, isSigner: false, isWritable: true },
      { pubkey: authority, isSigner: true, isWritable: false },
    ],
    data,
  });
}

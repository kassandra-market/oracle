/**
 * The cancel write ACTION (pure ix-builder, NO React).
 *
 * {@link buildCancelIxs} marks an under-funded Funding market Cancelled once its
 * oracle is terminal. It is PERMISSIONLESS and moves no tokens, so there is no
 * ATA to derive — just the single SDK `cancel` ix.
 */
import { TransactionInstruction } from "@solana/web3.js";
import { cancel } from "@kassandra-market/sdk";
import { toAddress, type AddressInput } from "./ata";

export interface BuildCancelArgs {
  /** The market to cancel. */
  market: AddressInput;
  /** The market's Kassandra oracle (must be terminal). */
  oracle: AddressInput;
}

/** Assemble the (single-ix) cancel instruction list. */
export async function buildCancelIxs(
  args: BuildCancelArgs,
): Promise<TransactionInstruction[]> {
  const market = toAddress("Market", args.market);
  const oracle = toAddress("Oracle", args.oracle);
  const ix = await cancel({ market, oracle });
  return [ix];
}

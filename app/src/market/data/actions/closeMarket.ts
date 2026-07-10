/**
 * The close-market write ACTION (pure ix-builder, NO React).
 *
 * A permissionless rent-reclaim crank on a fully-settled market. Once the market
 * is terminal (Resolved/Void/Cancelled), its protocol fee is collected (activated
 * markets only), and every contributor has exited (`openContributions == 0`), the
 * program SPL-`CloseAccount`s the Market-PDA-owned token accounts (escrow always;
 * cyes/cno/lp_vault iff the market was activated) and closes the `Market` PDA,
 * routing ALL reclaimed rent back to the creator.
 *
 * {@link buildCloseMarketIxs} needs only the market PDA + its creator
 * (`market.creator`, decoded) — the SDK flow derives every token account. No ATA
 * prepend is needed (nothing is created); it only prepends a raised compute budget
 * for the several chained `CloseAccount` CPIs.
 */
import { type TransactionInstruction } from "@solana/web3.js";
import { flows } from "@kassandra-market/markets";
import { setComputeUnitLimitIx } from "./compute";
import { toAddress, type AddressInput } from "./ata";

/** Compute budget for close_market (up to four chained SPL CloseAccount CPIs + the PDA close). */
export const CLOSE_MARKET_COMPUTE_UNITS = 400_000;

export interface BuildCloseMarketArgs {
  /** The terminal Market PDA being closed (its account rent → the creator). */
  market: AddressInput;
  /** `market.creator` (decoded) — the recipient of ALL reclaimed rent. */
  creator: AddressInput;
}

/**
 * Assemble the close-market instruction list: `[computeBudget, closeMarket]`. The
 * market must be terminal + fee-collected (activated markets) + fully claimed
 * (`openContributions == 0`); the program is the guard for those preconditions.
 */
export async function buildCloseMarketIxs(
  args: BuildCloseMarketArgs,
): Promise<TransactionInstruction[]> {
  const market = toAddress("Market", args.market);
  const creator = toAddress("Creator", args.creator);
  const ix = await flows.closeMarketInstruction({ market, creator });
  return [setComputeUnitLimitIx(CLOSE_MARKET_COMPUTE_UNITS), ix];
}

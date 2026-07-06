/**
 * High-level `close_market` crank flow — the permissionless rent-reclaim on a
 * fully-settled market (status terminal + `feeCollected` for activated markets +
 * `openContributions == 0`). It SPL-`CloseAccount`s the Market-PDA-owned token
 * accounts and closes the `Market` PDA, routing ALL reclaimed rent back to the
 * creator (the original payer).
 *
 * The builder derives every account (escrow / cyes / cno / lp_vault) from the
 * `market` PDA, so this flow only needs the market + its creator — it works for
 * BOTH activated (Resolved/Void) and never-activated (Cancelled) markets (the
 * program simply skips the pool slots when the market was never activated).
 */
import { type TransactionInstruction } from "@solana/web3.js";

import { closeMarket as buildCloseMarket } from "../instructions/market.js";
import type { AddressInput } from "../pda.js";

export interface CloseMarketFlowParams {
  /** The terminal market being closed (its Market PDA is reaped). */
  market: AddressInput;
  /** `market.creator` — the recipient of ALL reclaimed rent (decode the Market to read it). */
  creator: AddressInput;
}

/**
 * Build the single `close_market` instruction. The market must be terminal
 * (Resolved/Void/Cancelled), have its fee collected (activated markets only), and
 * have `openContributions == 0` (every contributor has claimed/refunded).
 */
export function closeMarketInstruction(
  params: CloseMarketFlowParams,
): Promise<TransactionInstruction> {
  return buildCloseMarket({ market: params.market, creator: params.creator });
}

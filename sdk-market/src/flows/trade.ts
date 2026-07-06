/**
 * High-level trade flows — buy / sell a net YES or NO position without ever
 * hand-deriving conditional tokens.
 *
 * A binary market's payout tokens are the vault's cYES / cNO. To take a *net*
 * directional position a trader:
 *   • BUY  — `split_tokens(kassAmount)` mints an equal cYES+cNO pair from KASS,
 *            then `swap`s the unwanted leg on the AMM into more of the wanted leg.
 *            YES ⇒ sell the cNO leg (quote→base = `Buy`); NO ⇒ sell the cYES leg
 *            (base→quote = `Sell`).
 *   • SELL — the mirror: `swap` the held leg back toward a balanced pair, then
 *            `merge_tokens` the balanced set back into KASS.
 *
 * The AMM is created with `base = cYES`, `quote = cNO`, so:
 *   `SwapType.Buy`  = quote→base = cNO → cYES  (accumulate YES),
 *   `SwapType.Sell` = base→quote = cYES → cNO  (accumulate NO).
 *
 * User conditional-token accounts default to the canonical ATAs (derived from the
 * vault's `yesMint`/`noMint`), but may be overridden (e.g. for fabricated test
 * accounts). Swap `outputAmountMin` defaults to 0; production callers should pass
 * a slippage-guarded minimum from a pool quote.
 */
import { Address, type TransactionInstruction } from "@solana/web3.js";

import * as metadao from "../metadao/index.js";
import type { AddressInput } from "../pda.js";
import { SwapType } from "../metadao/index.js";
import type { MarketRefs } from "./compose.js";
import { toAddr } from "./util.js";

/** Resolve the user's cYES/cNO token accounts (ATAs by default; overridable). */
async function userConditionalAtas(
  refs: MarketRefs,
  user: AddressInput,
  userYesAta?: AddressInput,
  userNoAta?: AddressInput,
): Promise<{ yes: Address; no: Address }> {
  const yes = userYesAta ? toAddr(userYesAta) : await metadao.pda.ata(user, refs.yesMint);
  const no = userNoAta ? toAddr(userNoAta) : await metadao.pda.ata(user, refs.noMint);
  return { yes, no };
}

/** Outcome leg a trader wants exposure to. */
export type Outcome = "yes" | "no";

/** Narrow + validate `outcome`, throwing on anything but `"yes"`/`"no"`. */
function isYes(outcome: Outcome): boolean {
  if (outcome !== "yes" && outcome !== "no") {
    throw new Error(`outcome must be "yes" or "no", got ${JSON.stringify(outcome)}`);
  }
  return outcome === "yes";
}

// ── buy ─────────────────────────────────────────────────────────────────────────

export interface BuyParams {
  /** Composed market refs (from `composeMarketInstructions`). */
  refs: MarketRefs;
  /** Trader + signer (owns the KASS + conditional accounts). */
  user: AddressInput;
  /** Which leg to end up net-long. */
  outcome: Outcome;
  /** KASS to spend (raw base units); split 1:1 into a cYES+cNO pair. */
  kassAmount: bigint | number;
  /** Trader's KASS token account the split pulls from. */
  userKassAta: AddressInput;
  /** Trader's cYES account (defaults to the ATA on `refs.yesMint`). */
  userYesAta?: AddressInput;
  /** Trader's cNO account (defaults to the ATA on `refs.noMint`). */
  userNoAta?: AddressInput;
  /** Min output of the swap's wanted leg (slippage guard; default 0). */
  outputAmountMin?: bigint | number;
}

/**
 * Build `[split, swap]` for a net YES/NO buy. The swap sends the ENTIRE unwanted
 * leg (== `kassAmount`) into the pool; the trader is left holding
 * `kassAmount + swapOut` of the wanted leg and 0 of the other.
 *
 * PRECONDITION: the trader's cYES + cNO token accounts (the ATAs, or the overrides)
 * must ALREADY exist — the split + swap cannot create them. For a fresh wallet
 * prepend {@link ensureConditionalAtasInstructions}.
 *
 * COMPUTE: a `split_tokens` + AMM `swap` CPI can exceed the 200k default; prepend a
 * `SetComputeUnitLimit` (as with `activate`). The SAME resolved cYES/cNO accounts
 * are threaded into BOTH the split and the swap so they never disagree.
 */
export async function buyInstructions(
  params: BuyParams,
): Promise<{ instructions: TransactionInstruction[]; userYesAta: Address; userNoAta: Address }> {
  const { refs, user, kassAmount } = params;
  const yesOutcome = isYes(params.outcome);
  const { yes, no } = await userConditionalAtas(refs, user, params.userYesAta, params.userNoAta);

  const split = await metadao.splitTokens({
    question: refs.question,
    vault: refs.vault,
    vaultUnderlyingAta: refs.vaultUnderlyingAta,
    authority: user,
    userUnderlyingAta: params.userKassAta,
    conditionalMints: [refs.yesMint, refs.noMint],
    userConditionalAtas: [yes, no],
    amount: kassAmount,
  });

  // YES: dump the cNO leg for cYES (quote→base = Buy).
  // NO:  dump the cYES leg for cNO (base→quote = Sell).
  // Thread the SAME cYES/cNO accounts (base == yes, quote == no) into the swap.
  const swap = await metadao.swap({
    payer: user,
    baseMint: refs.yesMint,
    quoteMint: refs.noMint,
    userBase: yes,
    userQuote: no,
    swapType: yesOutcome ? SwapType.Buy : SwapType.Sell,
    inputAmount: kassAmount,
    outputAmountMin: params.outputAmountMin ?? 0,
  });

  return { instructions: [split, swap], userYesAta: yes, userNoAta: no };
}

// ── sell ────────────────────────────────────────────────────────────────────────

export interface SellParams {
  /** Composed market refs. */
  refs: MarketRefs;
  /** Trader + signer. */
  user: AddressInput;
  /** Which leg the trader currently holds. */
  outcome: Outcome;
  /**
   * Amount of the held leg to swap toward the opposite leg. Together with a pool
   * quote this rebalances the position into an equal cYES/cNO pair; the app
   * computes it from the current reserves.
   */
  swapAmount: bigint | number;
  /**
   * Size of the balanced cYES/cNO pair to `merge_tokens` back into KASS (== the
   * post-swap min(cYES, cNO)). Computed by the app from the swap's quoted output.
   */
  mergeAmount: bigint | number;
  /** Trader's KASS token account the merge pays back into. */
  userKassAta: AddressInput;
  /** Trader's cYES account (defaults to the ATA on `refs.yesMint`). */
  userYesAta?: AddressInput;
  /** Trader's cNO account (defaults to the ATA on `refs.noMint`). */
  userNoAta?: AddressInput;
  /** Min output of the rebalancing swap (slippage guard; default 0). */
  outputAmountMin?: bigint | number;
}

/**
 * Build `[swap, merge]` for closing a position back to KASS. Swaps `swapAmount`
 * of the held leg toward the opposite one, then merges a balanced `mergeAmount`
 * pair back into KASS. The two amounts are app-computed from a pool quote (the
 * SDK cannot know the AMM's exact output offline).
 *
 * PRECONDITION: the trader's cYES + cNO accounts must already exist (see
 * {@link ensureConditionalAtasInstructions}). COMPUTE: prepend a
 * `SetComputeUnitLimit` (swap + merge CPIs). The SAME cYES/cNO accounts are
 * threaded into BOTH the swap and the merge.
 */
export async function sellInstructions(
  params: SellParams,
): Promise<{ instructions: TransactionInstruction[]; userYesAta: Address; userNoAta: Address }> {
  const { refs, user } = params;
  const yesOutcome = isYes(params.outcome);
  const { yes, no } = await userConditionalAtas(refs, user, params.userYesAta, params.userNoAta);

  // Holding YES: swap cYES → cNO (base→quote = Sell) to rebalance.
  // Holding NO:  swap cNO → cYES (quote→base = Buy) to rebalance.
  const swap = await metadao.swap({
    payer: user,
    baseMint: refs.yesMint,
    quoteMint: refs.noMint,
    userBase: yes,
    userQuote: no,
    swapType: yesOutcome ? SwapType.Sell : SwapType.Buy,
    inputAmount: params.swapAmount,
    outputAmountMin: params.outputAmountMin ?? 0,
  });

  const merge = await metadao.mergeTokens({
    question: refs.question,
    vault: refs.vault,
    vaultUnderlyingAta: refs.vaultUnderlyingAta,
    authority: user,
    userUnderlyingAta: params.userKassAta,
    conditionalMints: [refs.yesMint, refs.noMint],
    userConditionalAtas: [yes, no],
    amount: params.mergeAmount,
  });

  return { instructions: [swap, merge], userYesAta: yes, userNoAta: no };
}

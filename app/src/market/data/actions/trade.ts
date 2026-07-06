/**
 * The active-market TRADE actions — buy / sell a net YES or NO position (pure
 * ix-builders, NO React).
 *
 * Both wrap the SDK's high-level `flows.buyInstructions` / `flows.sellInstructions`
 * (split→swap / swap→merge over the cYES/cNO AMM), sandwiched with the two things
 * the raw flows can't do for a fresh wallet: a prepended `SetComputeUnitLimit`
 * (the split + AMM-swap CPI exceeds the 200k default) and idempotent create-ATAs
 * for the trader's cYES/cNO accounts (the `InteractWithVault` + `swap` account
 * lists carry no ATA/System program, so the accounts must pre-exist). The
 * create-ATAs are idempotent — safe to prepend unconditionally, an existing ATA
 * is a no-op — so no per-account `getAccountInfo` probe is needed.
 *
 * SELL amounts are APP-computed (the SDK can't quote the AMM offline): to unwind
 * `positionAmount` of the held leg back to KASS we swap the pool-optimal fraction
 * toward the opposite leg so the post-swap cYES/cNO are ~balanced, then merge the
 * smaller balanced side. See {@link optimalUnwindSwap}.
 */
import { type TransactionInstruction } from "@solana/web3.js";
import { flows } from "@kassandra-market/sdk";
import type { flows as flowsNs } from "@kassandra-market/sdk";
import type { IndexerClient } from "../../lib/indexer";
import type { AmmReserves } from "../markets";
import { ValidationError } from "../writeAction";
import { setComputeUnitLimitIx } from "./compute";
import { toAddress, type AddressInput } from "./ata";

/** Which leg a trader wants exposure to (`"yes"` / `"no"`) — the flow's `Outcome`. */
export type Outcome = flowsNs.Outcome;

/** Compute budget for a buy/sell tx (split/merge + AMM swap CPI). */
export const TRADE_COMPUTE_UNITS = 400_000;

/**
 * The MetaDAO v0.4 AMM's LP swap fee, in basis points. The `amm` program
 * (`AMMyu265…`) takes a 1% cut of the swap INPUT (Uniswap-v2 style:
 * `in_after_fee = in·(1 - fee)`) before the constant-product curve. It is a
 * program constant — not stored on the `Amm` account, not exported by the SDK —
 * so it is pinned here. CRITICAL: the `outputAmountMin` floor MUST be computed
 * off a FEE-ADJUSTED output ({@link ammSwapOut}); a fee-less estimate floored by
 * only the slippage tolerance would sit at/above real output and revert every
 * swap once the fee (1%) meets or exceeds the slippage (also 1%).
 */
export const AMM_FEE_BPS = 100; // 1%

/**
 * Default slippage tolerance (basis points) applied ON TOP of the fee-adjusted
 * estimate, as headroom for reserve drift between quote and land.
 */
export const DEFAULT_SLIPPAGE_BPS = 100; // 1%

/**
 * The pure constant-product output for swapping `amountIn` of the input reserve
 * into the output reserve (`out = amountIn·outRes / (inRes + amountIn)`, NO fee).
 * The balancing heuristic uses this; the revert-critical `outputAmountMin` floor
 * uses {@link ammSwapOut} (fee-adjusted). Returns `0n` on an empty pool /
 * non-positive input.
 */
export function constantProductOut(amountIn: bigint, inReserve: bigint, outReserve: bigint): bigint {
  if (amountIn <= 0n || inReserve <= 0n || outReserve <= 0n) return 0n;
  return (amountIn * outReserve) / (inReserve + amountIn);
}

/**
 * The FEE-ADJUSTED swap output — the input reduced by {@link AMM_FEE_BPS} before
 * the constant-product curve, matching the on-chain `amm::swap`. Used for the
 * `outputAmountMin` floor + the buy preview so both reflect what the pool will
 * actually pay. Returns `0n` on an empty pool / non-positive input.
 */
export function ammSwapOut(amountIn: bigint, inReserve: bigint, outReserve: bigint): bigint {
  if (amountIn <= 0n || inReserve <= 0n || outReserve <= 0n) return 0n;
  const inAfterFee = (amountIn * BigInt(10_000 - AMM_FEE_BPS)) / 10_000n;
  return constantProductOut(inAfterFee, inReserve, outReserve);
}

/** Apply a slippage floor (bps) to an estimated output: `est·(10000-bps)/10000`. */
export function minOutFromSlippage(estimateOut: bigint, slippageBps: number): bigint {
  if (estimateOut <= 0n) return 0n;
  const bps = BigInt(Math.min(Math.max(Math.trunc(slippageBps), 0), 10_000));
  return (estimateOut * (10_000n - bps)) / 10_000n;
}

/**
 * The reserves oriented as (input, output) for a swap. The AMM is base=cYES,
 * quote=cNO. BUY-YES / SELL-NO push cNO→cYES (input=quote, output=base);
 * BUY-NO / SELL-YES push cYES→cNO (input=base, output=quote).
 */
function reservePair(
  reserves: AmmReserves,
  inputIsQuote: boolean,
): { inReserve: bigint; outReserve: bigint } {
  return inputIsQuote
    ? { inReserve: reserves.quote, outReserve: reserves.base }
    : { inReserve: reserves.base, outReserve: reserves.quote };
}

/**
 * The BUY preview: split `kassAmount` into a 1:1 cYES+cNO pair, then swap the
 * whole unwanted leg (== `kassAmount`) into the wanted one. Returns the estimated
 * wanted-leg received (`kassAmount + swapOut`) and the swap's `outputAmountMin`
 * floor. `null` reserves → no estimate (`0n` min = unbounded, tx still guards).
 */
export function previewBuy(
  reserves: AmmReserves | null | undefined,
  outcome: Outcome,
  kassAmount: bigint,
  slippageBps: number = DEFAULT_SLIPPAGE_BPS,
): { received: bigint; outputAmountMin: bigint } {
  if (!reserves || kassAmount <= 0n) return { received: kassAmount, outputAmountMin: 0n };
  // YES: dump cNO (quote) → cYES (base). NO: dump cYES (base) → cNO (quote).
  const { inReserve, outReserve } = reservePair(reserves, outcome === "yes");
  const swapOut = ammSwapOut(kassAmount, inReserve, outReserve);
  return { received: kassAmount + swapOut, outputAmountMin: minOutFromSlippage(swapOut, slippageBps) };
}

/**
 * The pool-optimal swap to unwind `held` units of a leg back to a balanced pair.
 * Swapping `s` of the held leg yields `out(s) = s·outRes/(inRes+s)`; we want the
 * remainder `held - s` to equal `out(s)` so both legs match for the merge. That
 * solves `s² + s(inRes + outRes - held) - held·inRes = 0`; we take the positive
 * root (computed in float — a preview estimate) and clamp to `[1, held-1]`.
 */
export function optimalUnwindSwap(held: bigint, inReserve: bigint, outReserve: bigint): bigint {
  if (held <= 1n) return 0n;
  if (inReserve <= 0n || outReserve <= 0n) return held / 2n; // no pool info: swap half.
  const h = Number(held);
  // s² + s·(inRes + outRes - held) - held·inRes = 0 → positive root.
  const linear = Number(inReserve) + Number(outReserve) - h;
  const disc = linear * linear + 4 * h * Number(inReserve);
  const s = (-linear + Math.sqrt(Math.max(disc, 0))) / 2;
  const clamped = Math.min(Math.max(Math.floor(s), 1), h - 1);
  return BigInt(clamped);
}

interface TradeCommon {
  indexer: IndexerClient;
  /** The composed refs for the Active market (from `marketRefs`). */
  refs: flowsNs.MarketRefs;
  /** Trader + signer. */
  user: AddressInput;
  /** Which leg to be net-long (buy) / currently hold (sell). */
  outcome: Outcome;
  /** Trader's KASS token account (buy source / sell payout). */
  userKassAta: AddressInput;
  /** Live pool reserves for the slippage estimate (optional; `null` → unbounded). */
  reserves?: AmmReserves | null;
  /** Slippage tolerance in basis points (default {@link DEFAULT_SLIPPAGE_BPS}). */
  slippageBps?: number;
}

export interface BuildBuyArgs extends TradeCommon {
  /** KASS to spend (raw base units, > 0); split 1:1 into a cYES+cNO pair. */
  kassAmount: bigint;
}

/**
 * Assemble a BUY: `[computeBudget, ...ensureConditionalAtas, split, swap]`. The
 * swap's `outputAmountMin` is derived from the live reserves + slippage when
 * given. The trader's KASS ATA is assumed to exist (they hold the KASS being
 * spent); only the cYES/cNO ATAs are ensured.
 */
export async function buildBuyIxs(args: BuildBuyArgs): Promise<TransactionInstruction[]> {
  const user = toAddress("Trader", args.user);
  if (args.kassAmount <= 0n) throw new ValidationError("Amount must be greater than zero.");

  const { outputAmountMin } = previewBuy(
    args.reserves,
    args.outcome,
    args.kassAmount,
    args.slippageBps ?? DEFAULT_SLIPPAGE_BPS,
  );

  const ensure = await flows.ensureConditionalAtasInstructions({ refs: args.refs, user });
  const buy = await flows.buyInstructions({
    refs: args.refs,
    user,
    outcome: args.outcome,
    kassAmount: args.kassAmount,
    userKassAta: toAddress("KASS ATA", args.userKassAta),
    // Thread the ATAs we just ensured so the split/swap can't re-derive differently.
    userYesAta: ensure.userYesAta,
    userNoAta: ensure.userNoAta,
    outputAmountMin,
  });

  return [setComputeUnitLimitIx(TRADE_COMPUTE_UNITS), ...ensure.instructions, ...buy.instructions];
}

export interface BuildSellArgs extends TradeCommon {
  /** Units of the held leg to unwind back to KASS (raw base units, > 0). */
  positionAmount: bigint;
}

/**
 * Assemble a SELL: `[computeBudget, ...ensureConditionalAtas, swap, merge]`. The
 * swap + merge amounts are computed from the live reserves via
 * {@link optimalUnwindSwap}: swap the pool-optimal fraction toward the opposite
 * leg, then merge the smaller balanced side (`min(remainder, outputAmountMin)`)
 * so the merge can never under-run its inputs. Reserves are REQUIRED — an
 * offline sell can't quote the AMM.
 */
export async function buildSellIxs(args: BuildSellArgs): Promise<TransactionInstruction[]> {
  const user = toAddress("Trader", args.user);
  if (args.positionAmount <= 0n) throw new ValidationError("Amount must be greater than zero.");
  if (!args.reserves) {
    throw new ValidationError("Live pool reserves are required to sell — try again in a moment.");
  }

  // Holding YES: swap cYES(base) → cNO(quote). Holding NO: swap cNO(quote) → cYES(base).
  const holdingYes = args.outcome === "yes";
  const { inReserve, outReserve } = reservePair(args.reserves, !holdingYes);
  const swapAmount = optimalUnwindSwap(args.positionAmount, inReserve, outReserve);
  if (swapAmount <= 0n) throw new ValidationError("Position is too small to unwind.");

  // Fee-adjusted output (matches on-chain) so the floor sits BELOW real output.
  const estOut = ammSwapOut(swapAmount, inReserve, outReserve);
  const outputAmountMin = minOutFromSlippage(estOut, args.slippageBps ?? DEFAULT_SLIPPAGE_BPS);
  const remainder = args.positionAmount - swapAmount;
  // Merge only the smaller guaranteed-present side: remainder of the held leg vs
  // the slippage-floored swap output. Both legs are >= this, so merge never fails.
  const mergeAmount = remainder < outputAmountMin ? remainder : outputAmountMin;
  if (mergeAmount <= 0n) throw new ValidationError("Position is too small to unwind.");

  const ensure = await flows.ensureConditionalAtasInstructions({ refs: args.refs, user });
  const sell = await flows.sellInstructions({
    refs: args.refs,
    user,
    outcome: args.outcome,
    swapAmount,
    mergeAmount,
    userKassAta: toAddress("KASS ATA", args.userKassAta),
    userYesAta: ensure.userYesAta,
    userNoAta: ensure.userNoAta,
    outputAmountMin,
  });

  return [setComputeUnitLimitIx(TRADE_COMPUTE_UNITS), ...ensure.instructions, ...sell.instructions];
}

/**
 * The collect-fee write ACTION (pure ix-builder, NO React).
 *
 * A permissionless, idempotent crank that — after a market resolves — cuts the
 * protocol `feeBps` share of the market's accrued LP earnings and routes it (as
 * KASS) to the futarchy-governed `Config.feeDestination`, via program-signed
 * `amm::remove_liquidity` → `conditional_vault::redeem_tokens` → SPL `transfer`.
 * `claim_lp` is gated on it, so ordering is forced: resolve → collect_fee →
 * claim_lp.
 *
 * {@link buildCollectFeeIxs} reconstructs the composed {@link flows.MarketRefs}
 * from the decoded Market (via {@link marketRefs}) and wires `Config.feeDestination`
 * into the SDK `flows.collectFeeInstruction`. No ATA prepend is needed (the fee
 * destination + every market account already exist); it only prepends a raised
 * compute budget for the three chained CPIs.
 */
import { type TransactionInstruction } from "@solana/web3.js";
import { flows, type Config, type Market } from "@kassandra-market/markets";
import { marketRefs } from "./marketRefs";
import { setComputeUnitLimitIx } from "./compute";
import { type AddressInput } from "./ata";

/** Compute budget for collect_fee (remove_liquidity + redeem_tokens + transfer). */
export const COLLECT_FEE_COMPUTE_UNITS = 800_000;

export interface BuildCollectFeeArgs {
  /** The resolved Market PDA (base58 or Address). */
  market: AddressInput;
  /** The decoded Market account (carries the composed MetaDAO bindings). */
  marketAccount: Market;
  /** The program `Config` (source of the fee destination). */
  config: Config;
}

/**
 * Assemble the collect-fee instruction list: `[computeBudget, collectFee]`. The
 * market's MetaDAO question must already be resolved (`resolveMarket`), which for
 * a Resolved/Void market it is.
 */
export async function buildCollectFeeIxs(
  args: BuildCollectFeeArgs,
): Promise<TransactionInstruction[]> {
  const refs = await marketRefs(args.market, args.marketAccount);
  const ix = await flows.collectFeeInstruction({
    refs,
    feeDestination: args.config.feeDestination,
  });
  return [setComputeUnitLimitIx(COLLECT_FEE_COMPUTE_UNITS), ix];
}

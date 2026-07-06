/**
 * The redeem write ACTION (pure ix-builder, NO React).
 *
 * After a market resolves, a holder burns their full cYES + cNO balances for the
 * resolved KASS payout. {@link buildRedeemIxs} prepends idempotent create-ATAs
 * for the holder's cYES/cNO AND destination KASS accounts (the `redeem_tokens`
 * account list carries no ATA/System program, so all three must pre-exist), plus
 * a compute-budget ix (the burn-2-mints + transfer CPI), then the SDK
 * `flows.redeemInstructions`. The market's MetaDAO question must already be
 * resolved (`resolveMarket`), which for a Resolved/Void market it is.
 */
import { type TransactionInstruction } from "@solana/web3.js";
import { flows } from "@kassandra-market/sdk";
import type { flows as flowsNs } from "@kassandra-market/sdk";
import type { IndexerClient } from "../../lib/indexer";
import { setComputeUnitLimitIx } from "./compute";
import { toAddress, type AddressInput } from "./ata";

/** Compute budget for redeem (burn cYES + cNO, transfer the KASS payout). */
export const REDEEM_COMPUTE_UNITS = 300_000;

export interface BuildRedeemArgs {
  indexer: IndexerClient;
  /** The composed refs for the resolved market (from `marketRefs`). */
  refs: flowsNs.MarketRefs;
  /** Holder + signer (owns the conditional accounts). */
  user: AddressInput;
}

/**
 * Assemble the redeem instruction list:
 * `[computeBudget, ...ensureConditionalAtas(+KASS), redeem]`. The KASS ATA the
 * payout lands in is the one `ensureConditionalAtasInstructions({ includeKass })`
 * derives, so it's threaded straight into the redeem.
 */
export async function buildRedeemIxs(args: BuildRedeemArgs): Promise<TransactionInstruction[]> {
  const user = toAddress("Holder", args.user);

  const ensure = await flows.ensureConditionalAtasInstructions({
    refs: args.refs,
    user,
    includeKass: true,
  });
  const redeem = await flows.redeemInstructions({
    refs: args.refs,
    user,
    userKassAta: ensure.userKassAta!,
    userYesAta: ensure.userYesAta,
    userNoAta: ensure.userNoAta,
  });

  return [setComputeUnitLimitIx(REDEEM_COMPUTE_UNITS), ...ensure.instructions, ...redeem.instructions];
}

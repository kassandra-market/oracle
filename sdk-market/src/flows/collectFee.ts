/**
 * High-level `collect_fee` crank flow — after a market resolves, a permissionless
 * crank cuts the protocol `fee_bps` share of the market's **accrued** LP earnings
 * (program-signed `amm::remove_liquidity` → `conditional_vault::redeem_tokens` →
 * SPL `transfer`) into the futarchy-governed `config.fee_destination`.
 *
 * `claim_lp` is gated on collection, so ordering is forced: **resolve →
 * collect_fee → claim_lp**. {@link collectFeeInstruction} wires a composed
 * {@link MarketRefs} (which already carries every MetaDAO binding + the
 * market-PDA-owned cYES/cNO/LP holders + escrow) plus the `Config.feeDestination`
 * into the Task-2 `collectFee` builder — a single instruction, no ATA prepend
 * (the fee destination + all market accounts already exist).
 */
import { type TransactionInstruction } from "@solana/web3.js";

import { EXTERNAL_PROGRAM_IDS } from "../constants.js";
import { collectFee as buildCollectFee } from "../instructions/market.js";
import type { AddressInput } from "../pda.js";
import type { MarketRefs } from "./compose.js";

export interface CollectFeeFlowParams {
  /** The composed refs for the resolved market (from `composeMarketInstructions` or `marketRefs`). */
  refs: MarketRefs;
  /** `config.feeDestination` — the KASS token account the accrued fee routes to. */
  feeDestination: AddressInput;
}

/**
 * Build the single `collect_fee` instruction. The market's MetaDAO question must
 * already be resolved (`resolveMarket`), which for a Resolved/Void market it is.
 * A raised compute budget is needed (the remove_liquidity + redeem + transfer
 * CPIs), so callers should prepend a `SetComputeUnitLimit`.
 */
export function collectFeeInstruction(
  params: CollectFeeFlowParams,
): Promise<TransactionInstruction> {
  const { refs, feeDestination } = params;
  return buildCollectFee({
    market: refs.market,
    feeDestination,
    question: refs.question,
    vault: refs.vault,
    vaultUnderlyingAta: refs.vaultUnderlyingAta,
    yesMint: refs.yesMint,
    noMint: refs.noMint,
    marketCyes: refs.marketCyes,
    marketCno: refs.marketCno,
    amm: refs.amm,
    lpMint: refs.lpMint,
    lpVault: refs.lpVault,
    ammVaultBase: refs.ammVaultBase,
    ammVaultQuote: refs.ammVaultQuote,
    cvEventAuthority: refs.cvEventAuthority,
    ammEventAuthority: refs.ammEventAuthority,
    cvProgram: EXTERNAL_PROGRAM_IDS.conditionalVault,
    ammProgram: EXTERNAL_PROGRAM_IDS.ammV04,
  });
}

/**
 * `SetComputeUnitLimit` helper (pure, NO React).
 *
 * The MetaDAO composition + `activate`, and the split/swap/merge trade CPIs,
 * exceed the 200k default compute-unit limit, so the active-market action
 * builders prepend a `SetComputeUnitLimit`. web3.js@3.0.0-rc.2 ships the
 * `ComputeBudgetProgram` helper, so we defer to it rather than hand-assembling
 * the ix.
 *
 * A raised limit only affects the fee when a per-CU price is also set (we set
 * none), so an over-estimate is harmless.
 */
import { Address, ComputeBudgetProgram, TransactionInstruction } from "@solana/web3.js";

/** The Solana ComputeBudget native program. */
export const COMPUTE_BUDGET_PROGRAM_ID = new Address(
  "ComputeBudget111111111111111111111111111111",
);

/** A `SetComputeUnitLimit(units)` instruction. */
export function setComputeUnitLimitIx(units: number): TransactionInstruction {
  return ComputeBudgetProgram.setComputeUnitLimit({ units });
}

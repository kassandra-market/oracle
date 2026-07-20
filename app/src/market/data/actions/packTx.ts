/**
 * Greedy instruction-PACKING (pure, NO React): given an ORDERED {@link ActivateStep}
 * list, merge as many consecutive steps as will fit into a single legacy
 * transaction, minimizing the number of transactions (and therefore wallet
 * approvals) a sequence needs.
 *
 * Packing is safe because instructions inside one atomic transaction execute in
 * the given order and see each PRIOR instruction's effects within that same
 * transaction — exactly the on-chain read-chain (question → vault → amm) the
 * unpacked one-tx-per-step sequence already relies on. So combining steps never
 * changes on-chain behavior, only how many transactions (and wallet approvals)
 * it costs.
 *
 * A step declares its compute-unit need either via `ActivateStep.computeUnits`
 * (the activate sequence) or an EMBEDDED `SetComputeUnitLimit` at the front of
 * its `ixs` (bulk-liquidity steps reuse the single-market builders, which
 * already prepend their own — see `addLiquidity.ts`). A step declaring neither
 * is assumed to need Solana's tx-wide DEFAULT (200k) — it was built to run alone
 * under that default, so packing must reserve at least that much for it too.
 * Combining steps therefore sums each step's requirement and emits exactly ONE
 * merged `SetComputeUnitLimit` per batch (the runtime rejects a transaction
 * carrying more than one), never the raw per-step instructions concatenated.
 */
import {
  Address,
  ComputeBudgetInstruction,
  PACKET_DATA_SIZE,
  Transaction,
  type Blockhash,
  type TransactionInstruction,
} from "@solana/web3.js";
import { COMPUTE_BUDGET_PROGRAM_ID, setComputeUnitLimitIx } from "./compute";
import type { ActivateStep } from "./activate";

/** The hard per-transaction compute-unit ceiling (`MAX_COMPUTE_UNIT_LIMIT` on-chain). */
export const MAX_TX_COMPUTE_UNITS = 1_400_000;
/** Solana's tx-wide compute-unit limit when no `SetComputeUnitLimit` is present. */
const DEFAULT_TX_COMPUTE_UNITS = 200_000;

/** One packed transaction: the steps it covers (in order) + its final instruction list. */
export interface PackedTx {
  steps: ActivateStep[];
  ixs: TransactionInstruction[];
}

/** A blockhash-shaped placeholder used ONLY to size a probe message — never sent on-chain. */
const SIZE_PROBE_BLOCKHASH = "11111111111111111111111111111111111111111" as unknown as Blockhash;

function isComputeUnitLimitIx(ix: TransactionInstruction): boolean {
  if (ix.programId.toString() !== COMPUTE_BUDGET_PROGRAM_ID.toString()) return false;
  try {
    return ComputeBudgetInstruction.decodeInstructionType(ix) === "SetComputeUnitLimit";
  } catch {
    return false;
  }
}

/**
 * A step's real compute-unit need + its non-compute-budget instructions. Pulls
 * an embedded `SetComputeUnitLimit` out of `ixs` (added to any explicit
 * `computeUnits` field) rather than leaving it in the body, so a batch never
 * carries more than one such instruction.
 */
function normalizeStep(step: ActivateStep): { units: number; body: TransactionInstruction[] } {
  let declared = step.computeUnits;
  const body: TransactionInstruction[] = [];
  for (const ix of step.ixs) {
    if (isComputeUnitLimitIx(ix)) {
      declared = (declared ?? 0) + ComputeBudgetInstruction.decodeSetComputeUnitLimit(ix).units;
    } else {
      body.push(ix);
    }
  }
  return { units: declared ?? DEFAULT_TX_COMPUTE_UNITS, body };
}

function totalComputeUnits(steps: ActivateStep[]): number {
  return steps.reduce((sum, s) => sum + normalizeStep(s).units, 0);
}

/** Flatten a batch's steps into its final ixs: one merged compute-budget ix (when needed) + every step's non-budget ixs, in order. */
function combinedIxs(steps: ActivateStep[]): TransactionInstruction[] {
  const normalized = steps.map(normalizeStep);
  const units = normalized.reduce((sum, n) => sum + n.units, 0);
  const body = normalized.flatMap((n) => n.body);
  // Only a lone step relying on the plain default needs no override — anything
  // else (an explicit declaration, or more than one step sharing the tx) must
  // state its real requirement explicitly.
  return units > DEFAULT_TX_COMPUTE_UNITS ? [setComputeUnitLimitIx(units), ...body] : body;
}

/** The legacy-transaction wire size (signatures + message) `ixs` would produce for `feePayer`. */
function wireSize(feePayer: Address, ixs: TransactionInstruction[]): number {
  const tx = new Transaction({ feePayer, blockhash: SIZE_PROBE_BLOCKHASH, lastValidBlockHeight: 0 });
  tx.add(...ixs);
  const message = tx.compileMessage();
  // 1-byte compact-u16 signature-count prefix (true for any realistic signer count) + 64 bytes/signature.
  return 1 + message.header.numRequiredSignatures * 64 + message.serialize().length;
}

/**
 * Greedily pack `steps` (in order) into as few {@link PackedTx} batches as
 * possible: each batch's combined instructions must fit `PACKET_DATA_SIZE`
 * (1232 bytes) and {@link MAX_TX_COMPUTE_UNITS}. A step that alone exceeds
 * either limit is left in its own batch (the same constraint the unpacked
 * one-tx-per-step sequence was already subject to — packing can only shrink the
 * transaction count, never grow a single step past what it already required).
 */
export function packSteps(feePayer: Address, steps: ActivateStep[]): PackedTx[] {
  const batches: PackedTx[] = [];
  let current: ActivateStep[] = [];

  for (const step of steps) {
    const attempt = [...current, step];
    if (
      current.length > 0 &&
      (totalComputeUnits(attempt) > MAX_TX_COMPUTE_UNITS ||
        wireSize(feePayer, combinedIxs(attempt)) > PACKET_DATA_SIZE)
    ) {
      batches.push({ steps: current, ixs: combinedIxs(current) });
      current = [step];
    } else {
      current = attempt;
    }
  }
  if (current.length > 0) batches.push({ steps: current, ixs: combinedIxs(current) });
  return batches;
}

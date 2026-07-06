/**
 * The React seam driving a STAGED, multi-tx SEQUENCE (the activate bring-up).
 *
 * `compose → activate` can't fit one transaction (see `data/actions/activate`),
 * so {@link buildActivateSequence} returns an ORDERED list of {@link ActivateStep}s
 * this hook sends as a sequence of wallet-signed `sendAndConfirm` calls — one tx
 * per step, each with its own `SetComputeUnitLimit` prepended — tracking a
 * per-step {@link StepStatus}. On a mid-sequence failure it stops and lets the
 * control RETRY from the failed step.
 *
 * The composition instructions are NOT idempotent (re-running a landed init
 * reverts "already in use"), so BEFORE sending each step the hook probes the
 * account that step creates ({@link stepAlreadyLanded}) and SKIPS it when it
 * already exists — making a resume after a confirm-timeout safe instead of fatal.
 *
 * It reuses the SAME wallet-adapter sender the single-write seam
 * ({@link useWriteAction}) uses.
 */
import { useCallback, useMemo, useState } from "react";
import { Address } from "@solana/web3.js";
import { useWallet } from "@solana/wallet-adapter-react";
import { useIndexer } from "../lib/indexer";
import { sendAndConfirm, signAndRelay, type TxSender } from "../data/send";
import { mapWriteError } from "../data/writeAction";
import { activateStepIxs, stepAlreadyLanded, type ActivateStep } from "../data/actions/activate";

/** The lifecycle of one step in the sequence. */
export type StepStatus =
  | { kind: "pending" }
  | { kind: "running" }
  | { kind: "done"; signature: string }
  | { kind: "error"; message: string; logs?: string[] };

function isRunning(statuses: StepStatus[]): boolean {
  return statuses.some((s) => s.kind === "running");
}

export interface ActionSequence {
  /** Per-step status, index-aligned to the last-run step list. */
  statuses: StepStatus[];
  /** Whether the sequence is currently sending a step. */
  busy: boolean;
  /** Whether a wallet is connected (the control gates on this). */
  connected: boolean;
  /** The connected wallet's base58 address (or null). */
  address: string | null;
  /** Whether every step has completed. */
  allDone: boolean;
  /**
   * Run the given ordered steps from the first not-yet-done step, stopping at the
   * first failure. Safe to call again to retry from the failure.
   */
  run: (steps: ActivateStep[]) => Promise<void>;
  /** Reset all step statuses (e.g. re-compute). */
  reset: () => void;
}

export function useActionSequence(onDone?: () => void): ActionSequence {
  const indexer = useIndexer();
  const { publicKey, connected, signTransaction } = useWallet();
  const [statuses, setStatuses] = useState<StepStatus[]>([]);

  const walletSender = useMemo<TxSender | null>(() => {
    if (!connected || !publicKey || !signTransaction) return null;
    const feePayer = new Address(publicKey.toBase58());
    return (ixs) => signAndRelay(indexer, feePayer, signTransaction, ixs);
  }, [connected, publicKey, signTransaction, indexer]);

  const allDone = statuses.length > 0 && statuses.every((s) => s.kind === "done");

  const reset = useCallback(() => setStatuses([]), []);

  const run = useCallback(
    async (steps: ActivateStep[]) => {
      if (isRunning(statuses)) return;
      if (!walletSender) {
        setStatuses(steps.map(() => ({ kind: "pending" as const })));
        return;
      }

      // Align the status array to the step list; resume at the first non-done.
      let statusArr: StepStatus[] =
        statuses.length === steps.length
          ? [...statuses]
          : steps.map(() => ({ kind: "pending" as const }));
      const found = statusArr.findIndex((s) => s.kind !== "done");
      const begin = found === -1 ? steps.length : found;
      // Clear any prior error at/after the resume point.
      statusArr = statusArr.map((s, i) => (i >= begin && s.kind === "error" ? { kind: "pending" } : s));
      setStatuses(statusArr);

      for (let i = begin; i < steps.length; i++) {
        setStatuses((prev) => {
          const next = [...prev];
          next[i] = { kind: "running" };
          return next;
        });
        // Skip a step whose non-idempotent instruction already landed (e.g. a
        // prior attempt's confirm timed out after the tx actually succeeded);
        // re-sending it would revert "already in use".
        if (await stepAlreadyLanded(indexer, steps[i])) {
          setStatuses((prev) => {
            const next = [...prev];
            next[i] = { kind: "done", signature: "already-landed" };
            return next;
          });
          continue;
        }
        try {
          const { signature } = await sendAndConfirm(indexer, walletSender, activateStepIxs(steps[i]));
          setStatuses((prev) => {
            const next = [...prev];
            next[i] = { kind: "done", signature };
            return next;
          });
        } catch (err) {
          const failure = mapWriteError(err);
          setStatuses((prev) => {
            const next = [...prev];
            next[i] = { kind: "error", ...failure };
            return next;
          });
          return; // stop at the first failure; the caller can retry from here.
        }
      }
      onDone?.();
    },
    [statuses, walletSender, indexer, onDone],
  );

  return {
    statuses,
    busy: isRunning(statuses),
    connected,
    address: publicKey ? publicKey.toBase58() : null,
    allDone,
    run,
    reset,
  };
}

export default useActionSequence;

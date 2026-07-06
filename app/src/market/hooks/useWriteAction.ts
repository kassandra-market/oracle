/**
 * The React seam over the pure write-action state machine.
 *
 * {@link useWriteAction} wires wallet-adapter (`useWallet`) + the
 * {@link IndexerClient} into {@link runWriteAction}: it exposes the current
 * {@link WriteStatus}, a `run(build)` that drives one wallet-signed write, the
 * `indexer`/`address`/`connected` the forms need to assemble their action builder
 * args, and a `reset`.
 *
 * The wallet-backed {@link TxSender} builds a legacy transaction, stamps it with a
 * fresh indexer blockhash, signs it via the wallet's `signTransaction`, and relays
 * it to the indexer ({@link signAndRelay}) — the app never touches an RPC.
 */
import { useCallback, useMemo, useState } from "react";
import { Address, type TransactionInstruction } from "@solana/web3.js";
import { useWallet } from "@solana/wallet-adapter-react";
import { useIndexer } from "../lib/indexer";
import type { IndexerClient } from "../lib/indexer";
import { signAndRelay, type TxSender } from "../data/send";
import { isBusy, runWriteAction, type WriteStatus } from "../data/writeAction";

export interface WriteAction {
  /** The current lifecycle status of the last-started write. */
  status: WriteStatus;
  /** True while building/signing/confirming. */
  busy: boolean;
  /** The connected wallet's base58 address, or `null` when disconnected. */
  address: string | null;
  /** Whether a wallet is connected (the forms gate on this). */
  connected: boolean;
  /** The indexer client the forms pass to their `build*Ixs` call (ATA-existence check). */
  indexer: IndexerClient;
  /** Drive one wallet-signed write from an ix-builder; no-op if already busy. */
  run: (build: () => Promise<TransactionInstruction[]>) => Promise<void>;
  /** Reset back to `idle` (e.g. after a success line is dismissed). */
  reset: () => void;
}

/**
 * @param onSuccess called with the confirmed signature (the form refetches so
 *   the new on-chain state appears).
 */
export function useWriteAction(onSuccess?: (signature: string) => void): WriteAction {
  const indexer = useIndexer();
  const { publicKey, connected, signTransaction } = useWallet();
  const [status, setStatus] = useState<WriteStatus>({ kind: "idle" });

  const walletSender = useMemo<TxSender | null>(() => {
    if (!connected || !publicKey || !signTransaction) return null;
    const feePayer = new Address(publicKey.toBase58());
    return (ixs) => signAndRelay(indexer, feePayer, signTransaction, ixs);
  }, [connected, publicKey, signTransaction, indexer]);

  const run = useCallback(
    async (build: () => Promise<TransactionInstruction[]>) => {
      if (isBusy(status)) return;
      if (!walletSender) {
        setStatus({ kind: "error", message: "Connect a wallet to participate." });
        return;
      }
      await runWriteAction({ build, indexer, walletSender, setStatus, onSuccess });
    },
    [status, walletSender, indexer, onSuccess],
  );

  const reset = useCallback(() => setStatus({ kind: "idle" }), []);

  return {
    status,
    busy: isBusy(status),
    address: publicKey ? publicKey.toBase58() : null,
    connected,
    indexer,
    run,
    reset,
  };
}

export default useWriteAction;

/**
 * A tiny read hook over the connected wallet's KASS ATA balance.
 *
 * {@link useKassBalance} resolves the connected wallet's KASS balance (raw base
 * units) for a given mint, mirroring the unmount-guarded `useEffect`+nonce
 * pattern in {@link useAsync} (TanStack Query is NOT a dep). It reads the KASS ATA
 * via the {@link IndexerClient} and the connected `publicKey` from wallet-adapter,
 * so switching wallet re-runs the fetch automatically.
 *
 * The KASS mint comes from the on-chain `Config` (`useConfig().data.kassMint`) —
 * the caller passes it in once it has loaded, and passes `undefined` until then
 * (which keeps the balance `null`).
 *
 * `balance` is `null` while disconnected, still loading, mint-unknown, or after a
 * transient fetch error — callers must NOT hard-block a form on a `null` balance
 * (the on-chain tx remains the ultimate guard). A resolved `0n` means the wallet
 * genuinely holds no KASS (absent ATA), which a form MAY gate on.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { useWallet } from "@solana/wallet-adapter-react";
import { pda } from "@kassandra-market/markets";
import { useIndexer } from "../lib/indexer";
import type { IndexerClient } from "../lib/indexer";

/** Byte offset of the `amount: u64` field in an SPL token account. */
const TOKEN_ACCOUNT_AMOUNT_OFFSET = 64;

/**
 * The owner's KASS balance in raw base units, or `0n` when the ATA is absent
 * (`indexer.getAccount` 404 → null). Reads the SPL token account's `amount`
 * (`u64` LE @64) straight off the raw bytes. Throws only on a genuinely
 * unexpected/transient indexer failure.
 */
async function fetchKassBalance(
  indexer: IndexerClient,
  owner: string,
  kassMint: string,
): Promise<bigint> {
  const ata = (await pda.associatedTokenAccount(owner, kassMint)).address;
  const acct = await indexer.getAccount(ata.toString());
  if (!acct || acct.data.length < TOKEN_ACCOUNT_AMOUNT_OFFSET + 8) return 0n;
  const dv = new DataView(acct.data.buffer, acct.data.byteOffset, acct.data.byteLength);
  return dv.getBigUint64(TOKEN_ACCOUNT_AMOUNT_OFFSET, true);
}

export interface KassBalanceState {
  /** Raw base-unit KASS balance, or `null` (disconnected / loading / mint-unknown / transient error). */
  balance: bigint | null;
  /** True while a fetch is in flight. */
  loading: boolean;
  /** Re-run the fetch (e.g. after a successful contribute/refund spends/returns KASS). */
  refetch: () => void;
}

export function useKassBalance(kassMint: string | undefined): KassBalanceState {
  const indexer = useIndexer();
  const { publicKey, connected } = useWallet();
  const owner = connected && publicKey ? publicKey.toBase58() : null;

  const [balance, setBalance] = useState<bigint | null>(null);
  const [loading, setLoading] = useState(false);
  const [nonce, setNonce] = useState(0);

  const refetch = useCallback(() => setNonce((n) => n + 1), []);

  const indexerRef = useRef(indexer);
  indexerRef.current = indexer;

  useEffect(() => {
    if (!owner || !kassMint) {
      setBalance(null);
      setLoading(false);
      return;
    }
    let active = true;
    setLoading(true);
    fetchKassBalance(indexerRef.current, owner, kassMint).then(
      (value) => {
        if (!active) return;
        setBalance(value);
        setLoading(false);
      },
      () => {
        // Transient/unexpected indexer error: treat softly — leave balance null so
        // the form doesn't hard-block; the tx is the ultimate guard.
        if (!active) return;
        setBalance(null);
        setLoading(false);
      },
    );
    return () => {
      active = false;
    };
  }, [owner, kassMint, indexer, nonce]);

  return { balance, loading, refetch };
}

export default useKassBalance;

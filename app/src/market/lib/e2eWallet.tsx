/**
 * The REAL-SIGNING e2e wallet provider (e2e mode ONLY — `VITE_E2E=1`).
 *
 * This provider drives a REAL funded `Keypair`: it SIGNS the transaction the UI
 * builds; the SEND goes through the indexer (`signAndRelay` → `POST
 * /api/transaction`), exactly like the real-wallet path — there is no `Connection`
 * in the app. The keypair's secret is injected into the page by the Playwright
 * harness (which generated + funded it on the surfpool validator) as
 * `window.__E2E_WALLET_SECRET__` (a 64-byte array), so the browser e2e exercises
 * the exact write path a real wallet would — sign + relay + confirm — with no
 * automatable browser extension.
 */
import { useEffect, useMemo, useState, type ReactNode } from 'react'
import { WalletContext, type WalletContextState } from '@solana/wallet-adapter-react'
import { Keypair } from '@solana/web3.js'

/** The global the Playwright harness sets (via `addInitScript`) before app JS runs. */
declare global {
  interface Window {
    __E2E_WALLET_SECRET__?: number[]
  }
}

/**
 * Reconstruct the injected funded keypair (async in this web3.js flavor) and
 * expose it as a connected wallet whose `signTransaction` signs for real; the
 * app relays the signed tx through the indexer.
 */
export function E2eWalletProvider({ children }: { children: ReactNode }) {
  const [keypair, setKeypair] = useState<Keypair | null>(null)

  useEffect(() => {
    const secret = typeof window !== 'undefined' ? window.__E2E_WALLET_SECRET__ : undefined
    if (!secret || secret.length === 0) return
    let cancelled = false
    void Keypair.fromSecretKey(new Uint8Array(secret)).then((kp) => {
      if (!cancelled) setKeypair(kp)
    })
    return () => {
      cancelled = true
    }
  }, [])

  const value = useMemo<WalletContextState>(() => {
    const connected = keypair !== null
    return {
      autoConnect: false,
      wallets: [],
      wallet: null,
      publicKey: (keypair ? keypair.publicKey : null) as WalletContextState['publicKey'],
      connecting: false,
      connected,
      disconnecting: false,
      select: () => {},
      connect: async () => {},
      disconnect: async () => {},
      // The app no longer sends via the wallet — it signs (below) then relays
      // through the indexer. `sendTransaction` is unused; keep a throwing stub to
      // satisfy the wallet-adapter contract.
      sendTransaction: (async () => {
        throw new Error('e2e wallet: use signTransaction + indexer relay, not sendTransaction')
      }) as unknown as WalletContextState['sendTransaction'],
      signTransaction: (async (tx: any) => {
        if (!keypair) throw new Error('e2e wallet not ready')
        await tx.sign(keypair)
        return tx
      }) as unknown as WalletContextState['signTransaction'],
      signAllTransactions: (async (txs: any[]) => {
        if (!keypair) throw new Error('e2e wallet not ready')
        for (const tx of txs) await tx.sign(keypair)
        return txs
      }) as unknown as WalletContextState['signAllTransactions'],
      signMessage: undefined,
      signIn: undefined,
    } as WalletContextState
  }, [keypair])

  return <WalletContext.Provider value={value}>{children}</WalletContext.Provider>
}

export default E2eWalletProvider

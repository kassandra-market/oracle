import { useMemo, type ReactNode } from 'react'
import { WalletProvider } from '@solana/wallet-adapter-react'
import { WalletModalProvider } from '@solana/wallet-adapter-react-ui'
import {
  PhantomWalletAdapter,
  SolflareWalletAdapter,
} from '@solana/wallet-adapter-wallets'

import { ClusterProvider } from '../lib/ClusterProvider'
import { isMockMode, isE2eMode } from '../data/mockOracles'
import { MockWalletProvider } from '../lib/mockWallet'
import { E2eWalletProvider } from '../lib/e2eWallet'

// Wallet-adapter base styles are imported + overridden to the Auros look in
// index.css (no default purple/dark leaks through — we never render the
// adapter's own connect button; the NavPill drives the modal).

/**
 * Provider nesting for the read-only dApp shell:
 *   ClusterProvider (RPC endpoint + ConnectionProvider)
 *     > WalletProvider (autoConnect; standard-wallet autodetect + Phantom/Solflare)
 *       > WalletModalProvider (the select-wallet modal)
 *         > app
 * Read-only: the wallet connects but nothing is ever signed or sent.
 */
/**
 * Surface the REAL cause of a wallet error. `WalletError` wraps the underlying
 * error in `.error` and defaults its message to "Unexpected error", which hides
 * what actually failed (e.g. a wallet-adapter ↔ web3.js incompatibility on
 * connect). Log both so the console shows the true cause.
 */
function logWalletError(err: Error): void {
  const cause = (err as { error?: unknown }).error
  // eslint-disable-next-line no-console
  console.error(`[wallet] ${err.name}: ${err.message}`, cause ? { cause } : '', err)
}

export function AppProviders({ children }: { children: ReactNode }) {
  // Standard-wallet wallets (Phantom, Solflare, Backpack, …) are auto-detected
  // by WalletProvider; these explicit adapters cover the non-standard fallback.
  const wallets = useMemo(
    () => [new PhantomWalletAdapter(), new SolflareWalletAdapter()],
    [],
  )

  // Render-only affordance: under mock mode a scripted wallet replaces the real
  // WalletProvider so the write-form STATES are headless-reviewable without an
  // automatable browser wallet (see `lib/mockWallet`). Never active live.
  const WalletShell = isMockMode()
    ? ({ children: c }: { children: ReactNode }) => <MockWalletProvider>{c}</MockWalletProvider>
    : isE2eMode()
      ? ({ children: c }: { children: ReactNode }) => <E2eWalletProvider>{c}</E2eWalletProvider>
      : ({ children: c }: { children: ReactNode }) => (
          <WalletProvider wallets={wallets} autoConnect onError={logWalletError}>
            {c}
          </WalletProvider>
        )

  return (
    <ClusterProvider>
      <WalletShell>
        <WalletModalProvider>{children}</WalletModalProvider>
      </WalletShell>
    </ClusterProvider>
  )
}

export default AppProviders

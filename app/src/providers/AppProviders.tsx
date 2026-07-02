import { useMemo, type ReactNode } from 'react'
import { WalletProvider } from '@solana/wallet-adapter-react'
import { WalletModalProvider } from '@solana/wallet-adapter-react-ui'
import {
  PhantomWalletAdapter,
  SolflareWalletAdapter,
} from '@solana/wallet-adapter-wallets'

import { ClusterProvider } from '../lib/ClusterProvider'

// Wallet-adapter base styles are imported + overridden to the Delphi look in
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
export function AppProviders({ children }: { children: ReactNode }) {
  // Standard-wallet wallets (Phantom, Solflare, Backpack, …) are auto-detected
  // by WalletProvider; these explicit adapters cover the non-standard fallback.
  const wallets = useMemo(
    () => [new PhantomWalletAdapter(), new SolflareWalletAdapter()],
    [],
  )

  return (
    <ClusterProvider>
      <WalletProvider wallets={wallets} autoConnect>
        <WalletModalProvider>{children}</WalletModalProvider>
      </WalletProvider>
    </ClusterProvider>
  )
}

export default AppProviders

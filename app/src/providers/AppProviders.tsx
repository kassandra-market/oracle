import { type ReactNode } from 'react'

import { ClusterProvider } from '../lib/ClusterProvider'
import { isMockMode, isE2eMode } from '../data/mockOracles'
import { MockWalletProvider } from '../lib/mockWallet'
import { E2eWalletProvider } from '../lib/e2eWallet'
import { StandardWalletProvider } from '../lib/standardWallet'
import { WalletMenu } from '../components/wallet/WalletMenu'
// Market side: its sole data + tx gateway (same-origin `/api/*`). Decoupled from
// the oracle's RPC Connection — market pages read/write purely through this
// client, so it just needs to be in the tree for `useIndexer()` consumers.
import { IndexerProvider as MarketIndexerProvider } from '../market/lib/IndexerProvider'

/**
 * Provider nesting for the dApp shell:
 *   ClusterProvider (RPC endpoint + ConnectionProvider)
 *     > wallet layer (mock | e2e | real)
 *       > app + the connect picker
 *
 * The REAL wallet uses {@link StandardWalletProvider} (driven directly by the
 * Wallet Standard), NOT the legacy `@solana/wallet-adapter` `WalletProvider`:
 * that adapter is built against `@solana/web3.js@3.x` while it peer-requires 1.x,
 * so it hands the wallet a malformed request and Phantom throws "Unexpected
 * error" on connect. mock/e2e keep their scripted providers (auto-connected, so
 * the picker is never opened). All three expose the same `WalletContextState`,
 * so `useWallet()` consumers are unchanged.
 */
export function AppProviders({ children }: { children: ReactNode }) {
  const WalletShell = isMockMode()
    ? ({ children: c }: { children: ReactNode }) => <MockWalletProvider>{c}</MockWalletProvider>
    : isE2eMode()
      ? ({ children: c }: { children: ReactNode }) => <E2eWalletProvider>{c}</E2eWalletProvider>
      : ({ children: c }: { children: ReactNode }) => (
          <StandardWalletProvider>{c}</StandardWalletProvider>
        )

  return (
    <ClusterProvider>
      <WalletShell>
        <MarketIndexerProvider>
          {children}
          <WalletMenu />
        </MarketIndexerProvider>
      </WalletShell>
    </ClusterProvider>
  )
}

export default AppProviders

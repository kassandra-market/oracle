import { createContext, useContext } from 'react'
import { useConnection } from '@solana/wallet-adapter-react'

/**
 * The RPC clusters the dApp can point at.
 *
 * TWO modes, so the browser NEVER holds a Solana RPC endpoint in production:
 *
 *  - Gateway mode (default; `VITE_RPC_URL` unset): the app talks to a SAME-ORIGIN
 *    gateway path (`/indexer/rpc`) that the app server proxies to the private
 *    indexer/backend, which alone knows the real RPC. Every chain call — reads,
 *    blockhash for building txs, sendRawTransaction — flows through it. The
 *    cluster is a fixed label from `VITE_CLUSTER` (for explorer links only).
 *
 *  - Direct mode (`VITE_RPC_URL` set — local dev / e2e): the app talks to that
 *    RPC directly and the cluster switcher is available. Never used in production.
 */
export type Cluster = 'localnet' | 'devnet' | 'mainnet-beta'

export const CLUSTERS: readonly Cluster[] = ['localnet', 'devnet', 'mainnet-beta'] as const

/** Human labels for the cluster selector. */
export const CLUSTER_LABELS: Record<Cluster, string> = {
  localnet: 'Localnet',
  devnet: 'Devnet',
  'mainnet-beta': 'Mainnet',
}

/** A direct RPC URL for local dev / e2e. When unset, the app is in gateway mode. */
const RPC_OVERRIDE = (import.meta.env.VITE_RPC_URL as string | undefined)?.trim() || undefined

/** Same-origin path the app server proxies to the private backend's RPC gateway. */
const GATEWAY_PATH = '/indexer/rpc'

/** True in production: no direct RPC URL, so all chain access goes via the gateway. */
export function isGatewayMode(): boolean {
  return RPC_OVERRIDE === undefined
}

/** The fixed cluster (gateway mode) — a display label for explorer links only. */
export function gatewayCluster(): Cluster {
  const c = (import.meta.env.VITE_CLUSTER as string | undefined)?.trim()
  return c === 'devnet' || c === 'localnet' || c === 'mainnet-beta' ? c : 'mainnet-beta'
}

/** The resolved RPC endpoint for a cluster. */
export function endpointFor(cluster: Cluster): string {
  // `RPC_OVERRIDE` folds to a constant `undefined` in a production (gateway)
  // build, so the entire direct-mode branch below — including every hardcoded RPC
  // URL — is tree-shaken out of the bundle. No RPC endpoint ships in the app.
  if (RPC_OVERRIDE === undefined) {
    // Absolute same-origin URL so `@solana/web3.js` posts JSON-RPC to our own
    // server, which proxies to the private backend.
    const origin = typeof window !== 'undefined' ? window.location.origin : ''
    return `${origin}${GATEWAY_PATH}`
  }
  switch (cluster) {
    case 'localnet':
      return RPC_OVERRIDE
    case 'devnet':
      return 'https://api.devnet.solana.com'
    case 'mainnet-beta':
      return 'https://api.mainnet-beta.solana.com'
  }
}

export const CLUSTER_STORAGE_KEY = 'kassandra:cluster'

export function readStoredCluster(): Cluster {
  if (isGatewayMode()) return gatewayCluster()
  if (typeof window === 'undefined') return 'localnet'
  const stored = window.localStorage.getItem(CLUSTER_STORAGE_KEY)
  return stored && (CLUSTERS as readonly string[]).includes(stored)
    ? (stored as Cluster)
    : 'localnet'
}

export interface ClusterContextValue {
  cluster: Cluster
  endpoint: string
  setCluster: (cluster: Cluster) => void
  clusters: readonly Cluster[]
}

export const ClusterContext = createContext<ClusterContextValue | null>(null)

/** Current cluster + setter + resolved endpoint. */
export function useCluster(): ClusterContextValue {
  const ctx = useContext(ClusterContext)
  if (!ctx) throw new Error('useCluster must be used within a ClusterProvider')
  return ctx
}

// Re-export wallet-adapter's connection hook so the app has a single import
// site; the underlying `Connection` reflects the selected cluster's endpoint.
export { useConnection }

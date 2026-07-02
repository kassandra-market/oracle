import { createContext, useContext } from 'react'
import { useConnection } from '@solana/wallet-adapter-react'

/**
 * The RPC clusters the dApp can point at. `localnet` is also the surfpool
 * default; its endpoint is overridable via `VITE_RPC_URL` (see `endpointFor`).
 */
export type Cluster = 'localnet' | 'devnet' | 'mainnet-beta'

export const CLUSTERS: readonly Cluster[] = ['localnet', 'devnet', 'mainnet-beta'] as const

/** Human labels for the cluster selector. */
export const CLUSTER_LABELS: Record<Cluster, string> = {
  localnet: 'Localnet',
  devnet: 'Devnet',
  'mainnet-beta': 'Mainnet',
}

/** `VITE_RPC_URL` overrides the localnet endpoint; defaults to the local validator. */
const LOCAL_RPC_URL =
  (import.meta.env.VITE_RPC_URL as string | undefined) ?? 'http://127.0.0.1:8899'

/** The resolved RPC endpoint for a cluster. */
export function endpointFor(cluster: Cluster): string {
  switch (cluster) {
    case 'localnet':
      return LOCAL_RPC_URL
    case 'devnet':
      return 'https://api.devnet.solana.com'
    case 'mainnet-beta':
      return 'https://api.mainnet-beta.solana.com'
  }
}

export const CLUSTER_STORAGE_KEY = 'kassandra:cluster'

export function readStoredCluster(): Cluster {
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

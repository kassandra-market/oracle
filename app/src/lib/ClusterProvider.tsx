import { useMemo, useState, type ReactNode } from 'react'
import { ConnectionProvider } from '@solana/wallet-adapter-react'
import {
  CLUSTERS,
  CLUSTER_STORAGE_KEY,
  ClusterContext,
  endpointFor,
  readStoredCluster,
  type Cluster,
  type ClusterContextValue,
} from './cluster'

/**
 * Reads the selected cluster (persisted in localStorage), resolves its RPC
 * endpoint, and wraps children in wallet-adapter's `ConnectionProvider` keyed
 * on that endpoint — so the memoized `Connection` returned by `useConnection()`
 * updates whenever the cluster changes.
 */
export function ClusterProvider({ children }: { children: ReactNode }) {
  const [cluster, setClusterState] = useState<Cluster>(readStoredCluster)

  const setCluster = (next: Cluster) => {
    setClusterState(next)
    if (typeof window !== 'undefined') window.localStorage.setItem(CLUSTER_STORAGE_KEY, next)
  }

  const endpoint = endpointFor(cluster)

  const value = useMemo<ClusterContextValue>(
    () => ({ cluster, endpoint, setCluster, clusters: CLUSTERS }),
    [cluster, endpoint],
  )

  return (
    <ClusterContext.Provider value={value}>
      <ConnectionProvider endpoint={endpoint}>{children}</ConnectionProvider>
    </ClusterContext.Provider>
  )
}

export default ClusterProvider

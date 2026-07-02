import { Card, SectionHeader } from '../components/ui'
import { useCluster } from '../lib/cluster'

/**
 * Placeholder oracle browser. The read data layer + the real list/detail views
 * land in FA2/FA3; for now this stub proves the route + shell + provider wiring
 * and reflects the currently selected RPC cluster.
 */
export default function Oracles() {
  const { cluster, endpoint } = useCluster()
  return (
    <main className="mx-auto max-w-[1200px] px-6 py-20">
      <SectionHeader
        eyebrow="Browse"
        eyebrowPill
        line1="Oracles"
        line2="coming in FA3"
        paragraph="The oracle browser and detail views arrive in the next slices. This route, the app shell, and the RPC/wallet providers are wired and ready."
      />
      <div className="mx-auto mt-10 max-w-[640px]">
        <Card>
          <p className="font-inter text-body text-bronze">
            Pointed at <span className="font-medium text-sepia">{cluster}</span>
          </p>
          <p className="mt-1 font-mono text-caption text-driftwood">{endpoint}</p>
        </Card>
      </div>
    </main>
  )
}

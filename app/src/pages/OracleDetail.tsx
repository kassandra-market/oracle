import { Link, useParams } from 'react-router-dom'
import { Card, SectionHeader } from '../components/ui'

/**
 * Placeholder oracle detail view at `/oracles/:pubkey`. The decoded oracle
 * state (facts, proposers, AI claims, market) lands in FA3; this stub proves
 * the parameterized route within the shell.
 */
export default function OracleDetail() {
  const { pubkey } = useParams<{ pubkey: string }>()
  return (
    <main className="mx-auto max-w-[1200px] px-6 py-20">
      <SectionHeader
        eyebrow="Oracle"
        eyebrowPill
        line1="Detail view"
        line2="coming in FA3"
        paragraph="This oracle's facts, proposers, AI claims, and market will be decoded and laid out here in the next slice."
      />
      <div className="mx-auto mt-10 max-w-[640px]">
        <Card>
          <p className="font-inter text-body text-bronze">Requested oracle</p>
          <p className="mt-1 break-all font-mono text-caption text-sepia">{pubkey}</p>
          <Link
            to="/oracles"
            className="mt-4 inline-block font-inter text-[14px] text-sepia underline decoration-pebble underline-offset-4 hover:text-ember-orange"
          >
            ← Back to oracles
          </Link>
        </Card>
      </div>
    </main>
  )
}

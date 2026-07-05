import { Link } from 'react-router-dom'
import { useWallet } from '@solana/wallet-adapter-react'
import { useWalletMenu } from '../../lib/standardWallet'
import { Button } from '../ui'
import { useCluster, CLUSTER_LABELS, isGatewayMode, type Cluster } from '../../lib/cluster'

// Left-side primary links. "Oracles" is a real route; the rest are landing
// section anchors (they resolve on the landing page).
const NAV_LINKS: { label: string; href: string; route?: boolean }[] = [
  { label: 'Oracles', href: '/oracles', route: true },
  { label: 'How it works', href: '/#how-it-works' },
  { label: 'Governance', href: '/#why-kassandra' },
  { label: 'Docs', href: '#' },
]

// On-brand focus ring (sepia, never default blue) for the plain text links.
const linkFocus =
  'rounded-sm focus-visible:outline-none focus-visible:ring-2 ' +
  'focus-visible:ring-sepia/40 focus-visible:ring-offset-2 focus-visible:ring-offset-soft-cream'

const linkClass = `font-inter text-[14px] text-bronze transition-colors hover:text-sepia ${linkFocus}`

function truncateAddress(addr: string): string {
  return `${addr.slice(0, 4)}…${addr.slice(-4)}`
}

/**
 * The cluster control. In gateway mode (production) the app talks to a fixed
 * backend, so there's nothing to switch — a static label is shown. In direct mode
 * (dev / e2e) the full RPC selector is available.
 */
function ClusterSelector() {
  const { cluster, setCluster, clusters } = useCluster()

  if (isGatewayMode()) {
    return (
      <span
        className="hidden items-center rounded-button border border-pebble bg-soft-cream px-3 py-2 font-inter text-[13px] text-sepia sm:inline-flex"
        aria-label="RPC cluster"
      >
        {CLUSTER_LABELS[cluster]}
      </span>
    )
  }

  return (
    <label className="hidden items-center sm:inline-flex">
      <span className="sr-only">RPC cluster</span>
      <select
        aria-label="RPC cluster"
        value={cluster}
        onChange={(e) => setCluster(e.target.value as Cluster)}
        className={
          'cursor-pointer rounded-button border border-pebble bg-soft-cream px-3 py-2 ' +
          'font-inter text-[13px] text-sepia hover:bg-pebble/60 ' +
          'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-pebble ' +
          'focus-visible:ring-offset-2 focus-visible:ring-offset-soft-cream'
        }
      >
        {clusters.map((c) => (
          <option key={c} value={c}>
            {CLUSTER_LABELS[c]}
          </option>
        ))}
      </select>
    </label>
  )
}

/**
 * Real wallet connect control — the Auros NavPill (never the adapter's purple
 * button). Disconnected → opens the wallet-adapter modal. Connected → shows the
 * truncated address; click to disconnect. Read-only: no transactions.
 */
function ConnectControl() {
  const { publicKey, connected, connecting, disconnect } = useWallet()
  const { setOpen } = useWalletMenu()

  if (connected && publicKey) {
    const addr = publicKey.toBase58()
    return (
      <Button
        variant="NavPill"
        title={`${addr} — click to disconnect`}
        aria-label={`Connected: ${addr}. Click to disconnect.`}
        onClick={() => void disconnect()}
      >
        {truncateAddress(addr)}
      </Button>
    )
  }

  return (
    <Button
      variant="NavPill"
      aria-label="Connect wallet"
      disabled={connecting}
      onClick={() => setOpen(true)}
    >
      {connecting ? 'Connecting…' : 'Connect wallet'}
    </Button>
  )
}

/**
 * Auros top bar — soft-cream, a single hairline bottom border, not sticky.
 * Left links · centered serif wordmark · right actions (cluster selector +
 * the real wallet connect control).
 */
export default function NavBar() {
  return (
    <nav aria-label="Primary" className="border-b border-pebble bg-soft-cream">
      <div className="mx-auto flex max-w-[1200px] items-center justify-between gap-4 px-6 py-4">
        {/* Left: primary links (hidden on small screens) */}
        <ul className="hidden flex-1 items-center gap-6 md:flex">
          {NAV_LINKS.map((l) => (
            <li key={l.label}>
              {l.route ? (
                <Link to={l.href} className={linkClass}>
                  {l.label}
                </Link>
              ) : (
                <a href={l.href} className={linkClass}>
                  {l.label}
                </a>
              )}
            </li>
          ))}
        </ul>

        {/* Center: wordmark */}
        <Link
          to="/"
          className={`font-serif text-[26px] font-light tracking-[-0.01em] text-sepia ${linkFocus}`}
        >
          Kassandra
        </Link>

        {/* Right: actions */}
        <div className="flex flex-1 items-center justify-end gap-3">
          <ClusterSelector />
          <ConnectControl />
        </div>
      </div>
    </nav>
  )
}

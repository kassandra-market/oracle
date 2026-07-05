import { useEffect, useState } from 'react'
import { Link, useLocation } from 'react-router-dom'
import { useWallet } from '@solana/wallet-adapter-react'
import { useWalletMenu } from '../../lib/standardWallet'
import { Button } from '../ui'
import { useCluster, CLUSTER_LABELS, isGatewayMode, type Cluster } from '../../lib/cluster'

// Left-side primary links: the two product routes, the Governance landing
// section, and the (external) docs site.
const NAV_LINKS: { label: string; href: string; route?: boolean; external?: boolean }[] = [
  { label: 'Oracles', href: '/oracles', route: true },
  { label: 'Markets', href: '/markets', route: true },
  { label: 'Governance', href: '/#why-kassandra' },
  { label: 'Docs', href: 'https://dodecahedr0x.github.io/kassandra/', external: true },
]

// On-brand focus ring (sepia, never default blue) for the plain text links.
const linkFocus =
  'rounded-sm focus-visible:outline-none focus-visible:ring-2 ' +
  'focus-visible:ring-sepia/40 focus-visible:ring-offset-2 focus-visible:ring-offset-soft-cream'

const navBase = `font-inter text-[14px] transition-colors ${linkFocus}`
const linkClass = `${navBase} text-bronze hover:text-sepia`
// Active route link: brighter text + a subtle chestnut underline hint (aria-current
// carries the semantic; color is never the only signal).
const activeLinkClass = `${navBase} text-sepia underline decoration-chestnut/70 underline-offset-[6px]`

// The mobile-menu row variant — a full-width, ≥44px-tall tap target (touch
// guideline) with the same on-brand hover + focus treatment as the inline links.
const mobileBase = `flex min-h-[44px] items-center font-inter text-[15px] transition-colors ${linkFocus}`
const mobileLinkClass = `${mobileBase} text-bronze hover:text-sepia`
const mobileActiveLinkClass = `${mobileBase} text-sepia`

/** True when a route link matches the current path (exact or a sub-route). */
function isActiveLink(link: { href: string; route?: boolean }, pathname: string): boolean {
  if (!link.route) return false
  return pathname === link.href || pathname.startsWith(`${link.href}/`)
}

function truncateAddress(addr: string): string {
  return `${addr.slice(0, 4)}…${addr.slice(-4)}`
}

/** Lucide-style hamburger glyph (currentColor, 2px stroke) — no emoji icons. */
function MenuIcon() {
  return (
    <svg
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      aria-hidden="true"
    >
      <line x1="3" y1="6" x2="21" y2="6" />
      <line x1="3" y1="12" x2="21" y2="12" />
      <line x1="3" y1="18" x2="21" y2="18" />
    </svg>
  )
}

/** Lucide-style close (×) glyph, matched to {@link MenuIcon}. */
function CloseIcon() {
  return (
    <svg
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      aria-hidden="true"
    >
      <line x1="18" y1="6" x2="6" y2="18" />
      <line x1="6" y1="6" x2="18" y2="18" />
    </svg>
  )
}

/**
 * The cluster control. In gateway mode (production) the app talks to a fixed
 * backend, so there's nothing to switch — a static label is shown. In direct mode
 * (dev / e2e) the full RPC selector is available.
 */
function ClusterSelector({ mobile = false }: { mobile?: boolean }) {
  const { cluster, setCluster, clusters } = useCluster()

  // Desktop: an inline control that only appears at ≥sm. In the mobile menu we
  // force it visible (and full-width) so the cluster is reachable on phones.
  const wrap = mobile ? 'flex w-full' : 'hidden sm:inline-flex'

  if (isGatewayMode()) {
    return (
      <span
        className={`${wrap} items-center rounded-button border border-pebble bg-soft-cream px-3 py-2 font-inter text-[13px] text-sepia`}
        aria-label="RPC cluster"
      >
        {CLUSTER_LABELS[cluster]}
      </span>
    )
  }

  return (
    <label className={`${wrap} items-center`}>
      <span className="sr-only">RPC cluster</span>
      <select
        aria-label="RPC cluster"
        value={cluster}
        onChange={(e) => setCluster(e.target.value as Cluster)}
        className={
          `${mobile ? 'w-full ' : ''}cursor-pointer rounded-button border border-pebble bg-soft-cream px-3 py-2 ` +
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
 * ≥md: left links · centered serif wordmark · right actions (cluster selector +
 * wallet connect). <md: a hamburger toggles a collapsible panel carrying the
 * primary links + the cluster selector, so navigation stays reachable on phones;
 * the wordmark + connect control stay in the bar.
 */
export default function NavBar() {
  const [open, setOpen] = useState(false)
  const location = useLocation()

  // Collapse the menu whenever the route (path or in-page anchor) changes, so a
  // tapped link never leaves the panel hanging open over the destination.
  useEffect(() => {
    setOpen(false)
  }, [location.pathname, location.hash])

  // Escape closes the open menu (a11y escape route).
  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open])

  return (
    <nav aria-label="Primary" className="border-b border-pebble bg-soft-cream">
      <div className="mx-auto flex max-w-[1200px] items-center justify-between gap-4 px-6 py-4">
        {/* Left: hamburger (<md) + primary links (≥md) */}
        <div className="flex flex-1 items-center">
          <button
            type="button"
            aria-label={open ? 'Close menu' : 'Open menu'}
            aria-expanded={open}
            aria-controls="mobile-nav-menu"
            onClick={() => setOpen((v) => !v)}
            className={`-ml-2 inline-flex h-11 w-11 items-center justify-center rounded-button text-sepia hover:bg-pebble/60 md:hidden ${linkFocus}`}
          >
            {open ? <CloseIcon /> : <MenuIcon />}
          </button>
          <ul className="hidden flex-1 items-center gap-6 md:flex">
            {NAV_LINKS.map((l) => {
              const active = isActiveLink(l, location.pathname)
              return (
                <li key={l.label}>
                  {l.route ? (
                    <Link
                      to={l.href}
                      className={active ? activeLinkClass : linkClass}
                      aria-current={active ? 'page' : undefined}
                    >
                      {l.label}
                    </Link>
                  ) : (
                    <a
                      href={l.href}
                      className={linkClass}
                      {...(l.external ? { target: '_blank', rel: 'noreferrer noopener' } : {})}
                    >
                      {l.label}
                    </a>
                  )}
                </li>
              )
            })}
          </ul>
        </div>

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

      {/* Mobile menu panel — in normal flow beneath the bar (the nav isn't
          sticky, so it pushes content down rather than overlaying). */}
      {open ? (
        <div id="mobile-nav-menu" className="border-t border-pebble md:hidden">
          <ul className="flex flex-col px-6 py-1">
            {NAV_LINKS.map((l) => {
              const active = isActiveLink(l, location.pathname)
              return (
                <li key={l.label}>
                  {l.route ? (
                    <Link
                      to={l.href}
                      className={active ? mobileActiveLinkClass : mobileLinkClass}
                      aria-current={active ? 'page' : undefined}
                      onClick={() => setOpen(false)}
                    >
                      {l.label}
                    </Link>
                  ) : (
                    <a
                      href={l.href}
                      className={mobileLinkClass}
                      onClick={() => setOpen(false)}
                      {...(l.external ? { target: '_blank', rel: 'noreferrer noopener' } : {})}
                    >
                      {l.label}
                    </a>
                  )}
                </li>
              )
            })}
          </ul>
          <div className="border-t border-pebble px-6 py-4">
            <ClusterSelector mobile />
          </div>
        </div>
      ) : null}
    </nav>
  )
}

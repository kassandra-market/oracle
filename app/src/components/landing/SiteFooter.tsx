const COLUMNS: { heading: string; links: string[] }[] = [
  { heading: 'Protocol', links: ['Oracles', 'Disputes', 'Bonds'] },
  { heading: 'Docs', links: ['Overview', 'Runner', 'SDK'] },
  { heading: 'Governance', links: ['MetaDAO', 'Parameters', 'Treasury'] },
  { heading: 'GitHub', links: ['Programs', 'Runner', 'App'] },
]

const linkFocus =
  'rounded-sm focus-visible:outline-none focus-visible:ring-2 ' +
  'focus-visible:ring-sepia/40 focus-visible:ring-offset-2 focus-visible:ring-offset-soft-cream'

/**
 * Auros footer — a translucent glass material (`chrome-glass`) with a hairline
 * top edge. Serif wordmark, minimal link columns (placeholder hrefs), and one
 * low-key editorial line.
 */
export default function SiteFooter() {
  return (
    <footer className="chrome-glass border-t border-pebble">
      <div className="mx-auto max-w-[1200px] px-6 py-16">
        <div className="grid grid-cols-2 gap-10 sm:grid-cols-3 lg:grid-cols-5">
          <div className="col-span-2 sm:col-span-3 lg:col-span-1">
            <span className="font-serif text-[24px] font-light text-sepia">Kassandra</span>
            <p className="mt-3 max-w-[28ch] font-inter text-[14px] text-driftwood">
              An optimistic oracle with a mind. Built on Solana.
            </p>
          </div>
          {COLUMNS.map((col) => (
            <nav key={col.heading} aria-label={col.heading}>
              <h2 className="font-inter text-[13px] font-medium uppercase tracking-[0.08em] text-sepia">
                {col.heading}
              </h2>
              <ul className="mt-3 flex flex-col gap-2">
                {col.links.map((l) => (
                  <li key={l}>
                    <a
                      href="#"
                      className={`font-inter text-[14px] text-bronze transition-colors hover:text-sepia ${linkFocus}`}
                    >
                      {l}
                    </a>
                  </li>
                ))}
              </ul>
            </nav>
          ))}
        </div>
        <p className="mt-12 border-t border-pebble pt-6 font-inter text-[13px] text-driftwood">
          © 2026 Kassandra · A decentralized, AI-assisted optimistic oracle.
        </p>
      </div>
    </footer>
  )
}

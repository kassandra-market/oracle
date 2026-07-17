import type { Oracle, Proposer } from '@kassandra-market/oracles'
import { Card } from '../ui'
import { formatKass } from '../../lib/oracleView'

/** A percent 0..100 of `value` against `max` (bigint-safe), floored to a visible sliver when nonzero. */
function barPct(value: bigint, max: bigint): number {
  if (max <= 0n || value <= 0n) return 0
  return Math.max(Number((value * 100n) / max), 2)
}

/** A compact headline figure: label, a big value with an optional `/ total`, and a
 *  thin proportion track when a total is given. */
function StatTile({
  label,
  value,
  total,
  accent = false,
}: {
  label: string
  value: number
  total?: number
  accent?: boolean
}) {
  const pct = total && total > 0 ? Math.min(Math.max(value / total, 0), 1) * 100 : 0
  return (
    <div className="flex flex-col gap-1.5">
      <span className="font-inter text-[11px] uppercase tracking-[0.06em] text-driftwood">
        {label}
      </span>
      <span className="flex items-baseline gap-1">
        <span className="font-serif text-heading-sm font-light tabular-nums text-sepia">{value}</span>
        {total != null ? (
          <span className="font-inter text-[12px] tabular-nums text-driftwood">/ {total}</span>
        ) : null}
      </span>
      {total != null ? (
        <div className="h-1 w-full overflow-hidden rounded-full bg-soft-cream">
          <div
            className={`h-full rounded-full transition-[width] duration-500 ${
              accent && value > 0 ? 'bg-chestnut' : 'bg-bronze/70'
            }`}
            style={{ width: `${pct}%` }}
          />
        </div>
      ) : (
        <div className="h-1" aria-hidden />
      )}
    </div>
  )
}

/** A labelled proportion bar (label + KASS value on top, a flat track+fill below). */
function Bar({ label, value, width, fill }: { label: string; value: bigint; width: number; fill: string }) {
  return (
    <div>
      <div className="flex items-baseline justify-between gap-3">
        <span className="font-inter text-[12px] text-driftwood">{label}</span>
        <span className="font-inter text-[12px] tabular-nums text-sepia">{formatKass(value)} KASS</span>
      </div>
      <div className="mt-1 h-2 w-full overflow-hidden rounded-full bg-soft-cream">
        <div className={`h-full rounded-full ${fill}`} style={{ width: `${width}%` }} />
      </div>
    </div>
  )
}

/**
 * The revamped Overview economics — ONE cohesive card replacing the separate
 * stat-meter grid, bond-pool tile, and proportion viz. Three tiers, top to bottom:
 *
 *   1. Headline counts — options, proposers (surviving of), facts (settled of),
 *      open challenges — each a compact figure with a thin proportion track.
 *   2. Bonds (KASS) — the bond pool headline over the bond-pool / dispute-bonds /
 *      total-stake vault bars, scaled to the largest of the three.
 *   3. Proposer bond by option — Σ bond per originally-proposed option, the leading
 *      option accented aqua.
 */
export function OracleEconomics({ oracle, proposers }: { oracle: Oracle; proposers: Proposer[] }) {
  const meters = [
    { label: 'Bond pool', value: oracle.bondPool },
    { label: 'Dispute bonds', value: oracle.disputeBondTotal },
    { label: 'Total stake', value: oracle.totalOracleStake },
  ]
  const meterMax = meters.reduce((m, x) => (x.value > m ? x.value : m), 0n)

  // Σ bond per originally-proposed option (empty options shown as 0).
  const byOption = new Map<number, bigint>()
  for (const p of proposers) {
    byOption.set(p.originalOption, (byOption.get(p.originalOption) ?? 0n) + p.bond)
  }
  const optionBonds = Array.from({ length: oracle.optionsCount }, (_, i) => ({
    option: i,
    bond: byOption.get(i) ?? 0n,
  }))
  const optionMax = optionBonds.reduce((m, o) => (o.bond > m ? o.bond : m), 0n)
  const leadingOption =
    optionMax > 0n ? optionBonds.reduce((a, b) => (b.bond > a.bond ? b : a)).option : -1

  return (
    <Card className="flex flex-col gap-6">
      {/* 1 — headline counts */}
      <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
        <StatTile label="Options" value={oracle.optionsCount} />
        <StatTile label="Proposers" value={oracle.survivingCount} total={oracle.proposerCount} />
        <StatTile label="Facts settled" value={oracle.settledCount} total={oracle.factCount} />
        <StatTile
          label="Open challenges"
          value={oracle.openChallengeCount}
          total={oracle.factCount}
          accent
        />
      </div>

      {/* 2 — bonds / vaults */}
      <div className="border-t border-pebble pt-5">
        <div className="flex items-baseline justify-between gap-3">
          <span className="font-inter text-[11px] uppercase tracking-[0.06em] text-driftwood">
            Bonds · KASS
          </span>
          <span className="font-serif text-subheading font-light tabular-nums text-sepia">
            {formatKass(oracle.bondPool)} KASS
          </span>
        </div>
        <div className="mt-3 flex flex-col gap-3">
          {meterMax > 0n ? (
            meters.map((m) => (
              <Bar key={m.label} label={m.label} value={m.value} width={barPct(m.value, meterMax)} fill="bg-bronze/70" />
            ))
          ) : (
            <p className="font-inter text-[13px] text-driftwood">No bonds staked yet.</p>
          )}
        </div>
      </div>

      {/* 3 — proposer bond by option */}
      <div className="border-t border-pebble pt-5">
        <span className="font-inter text-[11px] uppercase tracking-[0.06em] text-driftwood">
          Proposer bond by option
        </span>
        <div className="mt-3 flex flex-col gap-3">
          {optionMax > 0n ? (
            optionBonds.map((o) => (
              <Bar
                key={o.option}
                label={o.option === leadingOption ? `Option ${o.option} · leading` : `Option ${o.option}`}
                value={o.bond}
                width={barPct(o.bond, optionMax)}
                fill={o.option === leadingOption ? 'bg-cyan-phosphor' : 'bg-pebble'}
              />
            ))
          ) : (
            <p className="font-inter text-[13px] text-driftwood">No proposer bonds yet.</p>
          )}
        </div>
      </div>
    </Card>
  )
}

export default OracleEconomics

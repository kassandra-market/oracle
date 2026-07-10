import type { Oracle, Proposer } from '@kassandra-market/oracles'
import { formatKass } from '../../lib/oracleView'

/** A percent 0..100 of `value` against `max` (bigint-safe), floored to a visible sliver when nonzero. */
function pct(value: bigint, max: bigint): number {
  if (max <= 0n || value <= 0n) return 0
  const p = Number((value * 100n) / max)
  return Math.max(p, 2)
}

/** A single labelled quiet bar (label + raw value on top, a flat track+fill below). */
function Bar({
  label,
  value,
  width,
  fill,
}: {
  label: string
  value: bigint
  width: number
  fill: string
}) {
  return (
    <div>
      <div className="flex items-baseline justify-between gap-3">
        <span className="font-inter text-[12px] text-driftwood">{label}</span>
        <span className="font-inter text-[12px] tabular-nums text-sepia">{formatKass(value)}</span>
      </div>
      <div className="mt-1 h-2 w-full overflow-hidden rounded-full bg-soft-cream">
        <div className={`h-full rounded-full ${fill}`} style={{ width: `${width}%` }} />
      </div>
    </div>
  )
}

/**
 * The economic picture — a flat, chart-lib-free proportion viz (divs + tokens)
 * of an oracle's economics:
 *
 *   1. Vault meters — bond pool vs dispute bonds vs total stake, each a quiet
 *      bronze bar scaled to the largest of the three (raw KASS base units).
 *   2. Option bond split — Σ proposer bond per `originalOption`, each option a
 *      quiet bar; the leading option gets the single chestnut accent.
 *
 * Degrades to a muted note when the values / proposer set are empty. Compact.
 */
export function EconomicPanel({
  oracle,
  proposers,
}: {
  oracle: Oracle
  proposers: Proposer[]
}) {
  const meters = [
    { label: 'Bond pool', value: oracle.bondPool },
    { label: 'Dispute bonds', value: oracle.disputeBondTotal },
    { label: 'Total stake', value: oracle.totalOracleStake },
  ]
  const meterMax = meters.reduce((m, x) => (x.value > m ? x.value : m), 0n)
  const anyMeter = meterMax > 0n

  // Σ bond per originally-proposed option (empty options still shown as 0).
  const byOption = new Map<number, bigint>()
  for (const p of proposers) {
    byOption.set(p.originalOption, (byOption.get(p.originalOption) ?? 0n) + p.bond)
  }
  const optionBonds = Array.from({ length: oracle.optionsCount }, (_, i) => ({
    option: i,
    bond: byOption.get(i) ?? 0n,
  }))
  const optionMax = optionBonds.reduce((m, o) => (o.bond > m ? o.bond : m), 0n)
  // The leading option (single chestnut accent) — only when there is real bond.
  const leadingOption =
    optionMax > 0n ? optionBonds.reduce((a, b) => (b.bond > a.bond ? b : a)).option : -1

  return (
    <section
      aria-label="Economic picture"
      className="mt-4 rounded-card border border-pebble bg-pure-card p-5"
    >
      <span className="font-inter text-[11px] uppercase tracking-[0.06em] text-driftwood">
        Economics · KASS
      </span>

      {/* Vault meters */}
      <div className="mt-3 flex flex-col gap-3">
        {anyMeter ? (
          meters.map((m) => (
            <Bar
              key={m.label}
              label={m.label}
              value={m.value}
              width={pct(m.value, meterMax)}
              fill="bg-bronze/70"
            />
          ))
        ) : (
          <p className="font-inter text-[13px] text-driftwood">No bonds staked yet.</p>
        )}
      </div>

      {/* Option bond split */}
      <div className="mt-6">
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
                width={pct(o.bond, optionMax)}
                fill={o.option === leadingOption ? 'bg-cyan-phosphor' : 'bg-pebble'}
              />
            ))
          ) : (
            <p className="font-inter text-[13px] text-driftwood">No proposer bonds yet.</p>
          )}
        </div>
      </div>
    </section>
  )
}

export default EconomicPanel

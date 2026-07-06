import { Link } from "react-router-dom";
import { Card } from "../ui";
import { StatusChip } from "./StatusChip";
import type { OracleGroup } from "../../market/data/markets";
import { formatKass, formatProbability, outcomeRow, truncateMiddle } from "../../market/lib/marketView";

const focusRing =
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40 " +
  "focus-visible:ring-offset-2 focus-visible:ring-offset-parchment";

/**
 * A categorical (N>2) oracle rendered as ONE grouped Delphi card: its N outcome
 * sub-markets listed side by side, each showing that outcome's implied chance
 * (its sub-market's YES probability from the pool reserves) plus a status chip,
 * and linking to that sub-market's detail. There are no on-chain labels, so each
 * outcome reads as "Outcome i".
 */
export function CategoricalCard({ group }: { group: OracleGroup }) {
  const outcomes = group.markets.map((summary) => outcomeRow(summary));
  const optionsCount = group.optionsCount ?? group.markets.length;
  const tvl = group.markets.reduce((sum, m) => sum + m.market.totalContributed, 0n);

  return (
    <Card className="flex h-full flex-col gap-3">
      <div className="flex items-center justify-between gap-2">
        <span className="inline-flex items-center rounded-tag border border-pebble bg-soft-cream px-2.5 py-1 font-inter text-[12px] font-medium text-bronze">
          Categorical · {optionsCount} outcomes
        </span>
        <span className="font-inter text-[12px] text-driftwood" title={group.oracle}>
          Oracle {truncateMiddle(group.oracle, 4, 4)}
        </span>
      </div>

      <h3 className="font-mono text-subheading font-light text-sepia">
        {group.markets.length} of {optionsCount} outcomes live
      </h3>

      <ul className="mt-1 flex flex-col divide-y divide-pebble/60">
        {outcomes.map((row) => (
          <li key={row.pubkey}>
            <Link
              to={`/markets/${row.pubkey}`}
              className={`group flex items-center justify-between gap-3 rounded-sm py-2 ${focusRing}`}
            >
              <span className="flex items-center gap-2">
                <span className="font-inter text-[13px] text-sepia group-hover:text-ember-orange">
                  {row.label}
                </span>
                <StatusChip status={row.status} />
              </span>
              <span className="font-inter text-[13px] font-medium text-ember-orange">
                {formatProbability(row.probability)}
              </span>
            </Link>
          </li>
        ))}
      </ul>

      <dl className="mt-auto flex flex-wrap gap-x-5 gap-y-1 pt-1 font-inter text-[13px] text-bronze">
        <div className="flex gap-1">
          <dt className="text-driftwood">TVL</dt>
          <dd className="font-medium text-sepia">{formatKass(tvl)} KASS</dd>
        </div>
      </dl>
    </Card>
  );
}

export default CategoricalCard;

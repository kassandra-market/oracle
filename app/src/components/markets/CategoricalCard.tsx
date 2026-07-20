import type { CSSProperties } from "react";
import { Link } from "react-router-dom";
import { MarketStatus } from "@kassandra-market/markets";
import { Card } from "../ui";
import { StatusChip } from "./StatusChip";
import { FundingBar } from "./FundingBar";
import type { OracleGroup } from "../../market/data/markets";
import { formatKass, formatProbability, outcomeRow, truncateMiddle } from "../../market/lib/marketView";
import type { OracleMetaView } from "../../hooks/useOracleMeta";

const focusRing =
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-platinum/40 " +
  "focus-visible:ring-offset-2 focus-visible:ring-offset-liquid-abyss";

/**
 * A categorical (N>2) oracle rendered as ONE grouped card: the QUESTION
 * (on-chain oracle subject, read best-effort via {@link OracleMetaView}) is the
 * title, and each outcome sub-market is listed by its option LABEL with that
 * outcome's implied chance (its sub-market's YES probability from the pool
 * reserves) plus a status chip, linking to that sub-market's detail. Without
 * metadata it degrades to a count title + "Outcome i" rows.
 */
export function CategoricalCard({
  group,
  meta,
  enterIndex,
}: {
  group: OracleGroup;
  meta?: OracleMetaView;
  /** First-load stagger index (undefined = no entrance animation). */
  enterIndex?: number;
}) {
  const outcomes = group.markets.map((summary) =>
    outcomeRow(summary, meta?.options?.[summary.market.outcomeIndex]),
  );
  const optionsCount = group.optionsCount ?? group.markets.length;
  const tvl = group.markets.reduce((sum, m) => sum + m.market.totalContributed, 0n);
  const subject = meta?.subject?.trim();
  const stagger = enterIndex !== undefined;

  // While any outcome is still Funding, ONE cumulative bar for the group's
  // combined raised/floor — not a bar per outcome (there is no per-outcome
  // funding bar anywhere in the group case, matching the detail page).
  const funding = group.markets.filter((m) => m.market.status === MarketStatus.Funding);
  const cumulativeFunding =
    funding.length > 0
      ? {
          totalContributed: funding.reduce((sum, m) => sum + m.market.totalContributed, 0n),
          minLiquidity: funding.reduce((sum, m) => sum + m.market.minLiquidity, 0n),
        }
      : null;

  return (
    <Card
      className={`flex h-full flex-col gap-3${stagger ? " stagger-in" : ""}`}
      style={
        stagger
          ? ({ "--stagger-delay": `${Math.min(enterIndex, 10) * 40}ms` } as CSSProperties)
          : undefined
      }
    >
      <div className="flex items-center justify-between gap-2">
        <span className="inline-flex items-center rounded-tag border border-hairline bg-liquid-deep px-2.5 py-1 font-inter text-[12px] font-medium text-silver">
          Categorical · {optionsCount} outcomes
        </span>
        <span className="font-inter text-[12px] text-silver" title={group.oracle}>
          Oracle {truncateMiddle(group.oracle, 4, 4)}
        </span>
      </div>

      {subject ? (
        <h3 className="text-balance font-serif text-subheading font-light text-platinum" title={subject}>
          {subject}
        </h3>
      ) : (
        <h3 className="font-mono text-subheading font-light text-platinum">
          {group.markets.length} of {optionsCount} outcomes live
        </h3>
      )}

      {cumulativeFunding ? (
        <div className="mt-1">
          <FundingBar market={cumulativeFunding} />
        </div>
      ) : null}

      <ul className="mt-1 flex flex-col divide-y divide-hairline/60">
        {outcomes.map((row) => (
          <li key={row.pubkey}>
            <Link
              to={`/markets/${row.pubkey}`}
              className={`group flex items-center justify-between gap-3 rounded-sm py-2 ${focusRing}`}
            >
              <span className="flex items-center gap-2">
                <span className="font-inter text-[13px] text-platinum group-hover:text-coral">
                  {row.label}
                </span>
                <StatusChip status={row.status} />
              </span>
              <span className="flex items-center gap-2">
                <span className="font-inter text-[13px] font-medium text-coral">
                  {formatProbability(row.probability)}
                </span>
                {/* An Active outcome sub-market is tradeable — point into its
                    trading interface (that sub-market's detail TradePanel). */}
                {row.status === MarketStatus.Active ? (
                  <span
                    aria-hidden="true"
                    className="font-inter text-[13px] text-coral transition-transform group-hover:translate-x-0.5"
                  >
                    →
                  </span>
                ) : null}
              </span>
            </Link>
          </li>
        ))}
      </ul>

      <dl className="mt-auto flex flex-wrap gap-x-5 gap-y-1 pt-1 font-inter text-[13px] text-silver">
        <div className="flex gap-1">
          <dt className="text-silver">TVL</dt>
          <dd className="font-medium text-platinum">{formatKass(tvl)} KASS</dd>
        </div>
      </dl>
    </Card>
  );
}

export default CategoricalCard;

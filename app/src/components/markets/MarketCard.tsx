import type { CSSProperties } from "react";
import { Link } from "react-router-dom";
import { MarketStatus } from "@kassandra-market/markets";
import { Card } from "../ui";
import { StatusChip } from "./StatusChip";
import { FundingBar } from "./FundingBar";
import { ProbabilityBar } from "./ProbabilityBar";
import type { MarketSummary } from "../../market/data/markets";
import { formatKass, impliedYesProbability, outcomeLabel, truncateMiddle } from "../../market/lib/marketView";
import type { OracleMetaView } from "../../hooks/useOracleMeta";

const focusRing =
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-platinum/40 " +
  "focus-visible:ring-offset-2 focus-visible:ring-offset-liquid-abyss";

/**
 * One market rendered as a clickable card. The on-chain oracle metadata
 * (subject + option labels, read best-effort via {@link OracleMetaView}) leads:
 * the title is the QUESTION and the sub-line is the outcome this market pays YES
 * on, in words. Without metadata it degrades to the short pubkey + numeric
 * outcome. The body shows a funding bar (Funding) or the live YES probability
 * (Active), plus TVL (totalContributed).
 */
export function MarketCard({
  summary,
  meta,
  enterIndex,
}: {
  summary: MarketSummary;
  meta?: OracleMetaView;
  /** First-load stagger index (undefined = no entrance animation). */
  enterIndex?: number;
}) {
  const { pubkey, market, reserves } = summary;
  const isFunding = market.status === MarketStatus.Funding;
  const isActive = market.status === MarketStatus.Active;
  const subject = meta?.subject?.trim();
  const boundLabel = meta?.options?.[market.outcomeIndex];
  const stagger = enterIndex !== undefined;

  return (
    <Link
      to={`/markets/${pubkey}`}
      className={`group block rounded-card ${focusRing}${stagger ? " stagger-in" : ""}`}
      style={
        stagger
          ? ({ "--stagger-delay": `${Math.min(enterIndex, 10) * 40}ms` } as CSSProperties)
          : undefined
      }
    >
      <Card className="flex h-full flex-col gap-3 transition-[transform,border-color] duration-200 ease-out group-hover:-translate-y-0.5 group-hover:border-cyan-phosphor/40 group-active:scale-[0.99] motion-reduce:group-hover:translate-y-0">
        <div className="flex items-center justify-between gap-2">
          <StatusChip status={market.status} />
          {/* Active markets are tradeable — surface the trade entry right on the
              card so every interface showing a tradeable market points into its
              trading interface (the detail's TradePanel). */}
          {isActive ? (
            <span className="inline-flex items-center gap-1 font-inter text-[12px] font-medium text-coral">
              Trade
              <span aria-hidden="true" className="transition-transform duration-200 ease-out group-hover:translate-x-0.5">
                →
              </span>
            </span>
          ) : null}
        </div>

        {subject ? (
          <h3 className="text-balance font-serif text-subheading font-light text-platinum" title={subject}>
            {subject}
          </h3>
        ) : (
          <h3 className="font-mono text-subheading font-light text-platinum" title={pubkey}>
            {truncateMiddle(pubkey, 6, 6)}
          </h3>
        )}
        <p className="font-inter text-[12px] text-silver">
          Pays <span className="font-medium text-coral">YES</span> on{" "}
          <span className="text-silver">
            {outcomeLabel(market.outcomeIndex, boundLabel)}
          </span>
        </p>

        <div className="mt-1">
          {isFunding ? <FundingBar market={market} /> : null}
          {isActive ? <ProbabilityBar probability={impliedYesProbability(reserves)} /> : null}
        </div>

        <dl className="mt-auto flex flex-wrap gap-x-5 gap-y-1 pt-1 font-inter text-[13px] text-silver">
          <div className="flex gap-1">
            <dt className="text-silver">TVL</dt>
            <dd className="font-medium text-platinum">{formatKass(market.totalContributed)} KASS</dd>
          </div>
        </dl>
      </Card>
    </Link>
  );
}

export default MarketCard;

import { Link } from "react-router-dom";
import { MarketStatus } from "@kassandra-market/sdk";
import { Card } from "../ui";
import { StatusChip } from "./StatusChip";
import { FundingBar } from "./FundingBar";
import { ProbabilityBar } from "./ProbabilityBar";
import type { MarketSummary } from "../../market/data/markets";
import { formatKass, impliedYesProbability, truncateMiddle } from "../../market/lib/marketView";

const focusRing =
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40 " +
  "focus-visible:ring-offset-2 focus-visible:ring-offset-parchment";

/**
 * One market rendered as a clickable Delphi card. There are no on-chain labels,
 * so the title is the market's short pubkey; the body shows a funding bar
 * (Funding) or the live YES probability (Active), plus TVL (totalContributed).
 */
export function MarketCard({ summary }: { summary: MarketSummary }) {
  const { pubkey, market, reserves } = summary;
  const isFunding = market.status === MarketStatus.Funding;
  const isActive = market.status === MarketStatus.Active;

  return (
    <Link to={`/markets/${pubkey}`} className={`group block rounded-card ${focusRing}`}>
      <Card className="flex h-full flex-col gap-3 transition-colors group-hover:border-driftwood">
        <div className="flex items-center justify-between gap-2">
          <StatusChip status={market.status} />
        </div>

        <h3 className="font-mono text-subheading font-light text-sepia" title={pubkey}>
          {truncateMiddle(pubkey, 6, 6)}
        </h3>
        <p className="font-inter text-[12px] text-driftwood" title={market.oracle.toString()}>
          Oracle {truncateMiddle(market.oracle.toString(), 4, 4)}
        </p>

        <div className="mt-1">
          {isFunding ? <FundingBar market={market} /> : null}
          {isActive ? <ProbabilityBar probability={impliedYesProbability(reserves)} /> : null}
        </div>

        <dl className="mt-auto flex flex-wrap gap-x-5 gap-y-1 pt-1 font-inter text-[13px] text-bronze">
          <div className="flex gap-1">
            <dt className="text-driftwood">TVL</dt>
            <dd className="font-medium text-sepia">{formatKass(market.totalContributed)} KASS</dd>
          </div>
        </dl>
      </Card>
    </Link>
  );
}

export default MarketCard;

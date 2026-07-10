import type { Market } from "@kassandra-market/markets";
import { formatKass, fundingProgress } from "../../market/lib/marketView";

/**
 * A funding-progress bar for a market in the Funding phase: a hairline track with
 * a chestnut fill at `totalContributed / minLiquidity`, plus the raw KASS figures
 * beneath. `funded` markets read as complete (full bar, "funded" note).
 */
export function FundingBar({ market }: { market: Pick<Market, "totalContributed" | "minLiquidity"> }) {
  const { pct, funded } = fundingProgress(market);
  const width = `${Math.round(pct * 100)}%`;
  return (
    <div className="flex flex-col gap-1.5">
      <div
        className="h-1.5 w-full overflow-hidden rounded-sm bg-soft-cream"
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(pct * 100)}
        aria-label="Funding progress"
      >
        <div className="h-full rounded-sm bg-chestnut transition-all" style={{ width }} />
      </div>
      <p className="font-inter text-[12px] text-driftwood">
        <span className="font-medium text-sepia">{formatKass(market.totalContributed)}</span>
        {" / "}
        {formatKass(market.minLiquidity)} KASS
        {funded ? <span className="text-chestnut"> · funded</span> : null}
      </p>
    </div>
  );
}

export default FundingBar;

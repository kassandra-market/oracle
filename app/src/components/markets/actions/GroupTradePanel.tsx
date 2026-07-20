import { useMemo, useState } from "react";
import { MarketStatus } from "@kassandra-market/markets";
import type { MarketDetail as MarketDetailData, MarketSummary } from "../../../market/data/markets";
import type { OracleGroupState } from "../../../market/hooks/useOracleGroup";
import { formatProbability, outcomeRow } from "../../../market/lib/marketView";
import { TradePanel } from "./TradePanel";

/** One outcome pill: label + live YES probability, doubling as the selector. */
function OutcomeTab({
  label,
  probability,
  selected,
  onSelect,
}: {
  label: string;
  probability: number | null;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={selected}
      onClick={onSelect}
      className={`flex shrink-0 items-center gap-2 rounded-tag border px-3 py-1.5 font-inter text-[13px] transition-colors ${
        selected
          ? "border-coral bg-coral/10 text-platinum"
          : "border-hairline bg-liquid-deep text-silver hover:text-platinum"
      }`}
    >
      <span>{label}</span>
      <span className="tabular-nums text-coral">{formatProbability(probability)}</span>
    </button>
  );
}

/**
 * The Trade tab's UNIFIED surface for a categorical group: an outcome selector
 * (one pill per tradable outcome, hidden for a lone/binary market) sitting above
 * the existing single-market {@link TradePanel} — swapping which outcome's
 * `pubkey`/`market`/`reserves` feed it as the user picks, entirely client-side
 * (no navigation). This is what lets "all the markets for conditional markets
 * be tradable in a single interface": each outcome is still its own AMM pool
 * under the hood, but the page never asks the user to think in terms of pools —
 * only outcomes.
 *
 * Tradable = every Active sibling with known reserves ({@link OracleGroupState.active}),
 * PLUS the CURRENT market itself when it's Active — preferring the page's own
 * fresher `useMarketDetail` copy over the list-level snapshot for that one, so a
 * just-landed trade/activation on THIS market is reflected immediately rather
 * than waiting on the siblings list's own refetch.
 *
 * Renders nothing when no outcome is tradable yet (mirrors `TradePanel`'s own
 * gating — the caller only mounts this once at least one outcome is Active).
 */
export function GroupTradePanel({
  detail,
  group,
  subject,
  options,
  refetch,
}: {
  /** The current page's own market detail (freshest data for its own outcome). */
  detail: MarketDetailData;
  /** The categorical group this market's oracle spans, from `useOracleGroup`. */
  group: OracleGroupState;
  /** The oracle question (header context; falls back to a generic label). */
  subject?: string;
  /** Per-outcome option labels, index-aligned to `outcomeIndex` (empty when unread). */
  options: string[];
  /** Called after a trade completes — refreshes both this page and the group's siblings. */
  refetch: () => void;
}) {
  const { pubkey, market, reserves } = detail;
  const isActive = market.status === MarketStatus.Active;

  const tradable = useMemo<MarketSummary[]>(() => {
    const current: MarketSummary[] = isActive
      ? [{ pubkey, market, reserves, oracleOptionsCount: null }]
      : [];
    const others = group.active.filter((m) => m.pubkey !== pubkey);
    return [...current, ...others].sort((a, b) => a.market.outcomeIndex - b.market.outcomeIndex);
  }, [group.active, pubkey, market, reserves, isActive]);

  // Default to the CURRENT market when it's itself tradable — landing on outcome
  // 2's own page and opening Trade should trade outcome 2, regardless of where a
  // lower-indexed sibling falls in the (outcome-ordered) selector list. Only
  // falls back to the first tradable sibling when the current one isn't Active.
  const [selected, setSelected] = useState<string | null>(null);
  const defaultPubkey = isActive ? pubkey : (tradable[0]?.pubkey ?? null);
  const picked = tradable.find((m) => m.pubkey === (selected ?? defaultPubkey)) ?? tradable[0] ?? null;

  if (!picked) return null;

  const boundLabel = options[picked.market.outcomeIndex]?.trim() || null;

  const onSuccess = () => {
    refetch();
    group.refetch();
  };

  return (
    <div className="flex flex-col gap-4">
      {tradable.length > 1 ? (
        <div role="tablist" aria-label="Outcome" className="flex gap-2 overflow-x-auto pb-1">
          {tradable.map((m) => {
            const row = outcomeRow(m, options[m.market.outcomeIndex]);
            return (
              <OutcomeTab
                key={m.pubkey}
                label={row.label}
                probability={row.probability}
                selected={m.pubkey === picked.pubkey}
                onSelect={() => setSelected(m.pubkey)}
              />
            );
          })}
        </div>
      ) : null}
      <TradePanel
        key={picked.pubkey}
        pubkey={picked.pubkey}
        market={picked.market}
        reserves={picked.reserves}
        onSuccess={onSuccess}
        question={subject}
        boundLabel={boundLabel}
      />
    </div>
  );
}

export default GroupTradePanel;

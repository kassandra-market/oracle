import { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { Button, Card, SectionHeader } from "../components/ui";
import { MarketCard } from "../components/markets/MarketCard";
import { CategoricalCard } from "../components/markets/CategoricalCard";
import { useMarkets } from "../market/hooks/useMarkets";
import { useConfig } from "../market/hooks/useMarketDetail";
import { groupByOracle, isCategorical, type MarketSummary } from "../market/data/markets";
import { fundingProgress, statusLabel } from "../market/lib/marketView";

type SortBy = "tvl" | "funding" | "status";

const SORT_LABELS: Record<SortBy, string> = {
  tvl: "TVL",
  funding: "Funding %",
  status: "Status",
};

const focusRing =
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40 " +
  "focus-visible:ring-offset-2 focus-visible:ring-offset-parchment";

/** Case-insensitive match against a market's address + status label. */
function matches(summary: MarketSummary, query: string): boolean {
  if (!query) return true;
  const q = query.toLowerCase();
  return (
    summary.pubkey.toLowerCase().includes(q) ||
    summary.market.oracle.toString().toLowerCase().includes(q) ||
    statusLabel(summary.market.status).toLowerCase().includes(q)
  );
}

function sortMarkets(list: MarketSummary[], by: SortBy): MarketSummary[] {
  const copy = [...list];
  switch (by) {
    case "tvl":
      return copy.sort((a, b) =>
        b.market.totalContributed > a.market.totalContributed
          ? 1
          : b.market.totalContributed < a.market.totalContributed
            ? -1
            : 0,
      );
    case "funding":
      return copy.sort(
        (a, b) => fundingProgress(b.market).pct - fundingProgress(a.market).pct,
      );
    case "status":
      return copy.sort((a, b) => a.market.status - b.market.status);
  }
}

function SkeletonCard() {
  return (
    <Card className="flex h-full animate-pulse flex-col gap-3" aria-hidden="true">
      <div className="flex items-center justify-between">
        <div className="h-6 w-20 rounded-tag bg-soft-cream" />
        <div className="h-4 w-16 rounded-sm bg-soft-cream" />
      </div>
      <div className="h-6 w-40 rounded-sm bg-soft-cream" />
      <div className="h-4 w-24 rounded-sm bg-soft-cream" />
      <div className="mt-2 h-4 w-full rounded-sm bg-soft-cream" />
    </Card>
  );
}

export default function Markets() {
  const { data, loading, error, refetch } = useMarkets();
  const config = useConfig();
  const [search, setSearch] = useState("");
  const [sortBy, setSortBy] = useState<SortBy>("tvl");

  // The program is "not set up" when its Config singleton is absent — surfaced in
  // the empty state so a missing deploy reads clearly rather than looking like an
  // empty-but-live program.
  const notInitialized = !config.loading && config.data === null;

  const visible = useMemo(() => {
    if (!data) return [];
    return sortMarkets(
      data.filter((m) => matches(m, search)),
      sortBy,
    );
  }, [data, search, sortBy]);

  // Collapse each oracle's sub-markets into a group: a categorical (N>2) oracle
  // renders as ONE grouped card; binary/single-outcome oracles render as today.
  const groups = useMemo(() => groupByOracle(visible), [visible]);

  return (
    <main className="mx-auto max-w-[1200px] px-6 py-20">
      <SectionHeader
        as="h1"
        eyebrow="Markets"
        line1="Open markets"
        paragraph="Every prediction market on the program — funding, active, and resolved."
      />

      <div className="mt-10 flex flex-col items-center gap-4 sm:flex-row sm:justify-between">
        <div className="flex w-full flex-wrap items-center gap-3 sm:w-auto">
          <input
            type="search"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search by address or status…"
            aria-label="Search markets"
            className={`w-full rounded-button border border-pebble bg-pure-card px-3 py-2 font-inter text-body text-sepia placeholder:text-driftwood sm:w-72 ${focusRing}`}
          />
          <label className="flex items-center gap-2 font-inter text-[13px] text-bronze">
            Sort
            <select
              value={sortBy}
              onChange={(e) => setSortBy(e.target.value as SortBy)}
              className={`rounded-button border border-pebble bg-pure-card px-2.5 py-2 font-inter text-body text-sepia ${focusRing}`}
            >
              {(Object.keys(SORT_LABELS) as SortBy[]).map((k) => (
                <option key={k} value={k}>
                  {SORT_LABELS[k]}
                </option>
              ))}
            </select>
          </label>
        </div>
        <Link to="/markets/new">
          <Button variant="GhostOutline">Create a market</Button>
        </Link>
      </div>

      <div className="mt-10">
        {loading ? (
          <div className="grid grid-cols-1 gap-6 sm:grid-cols-2 lg:grid-cols-3">
            {Array.from({ length: 6 }, (_, i) => (
              <SkeletonCard key={i} />
            ))}
          </div>
        ) : error ? (
          <div className="mx-auto max-w-[640px]">
            <Card className="flex flex-col items-center gap-4 text-center">
              <p className="font-inter text-body text-bronze">
                Could not load markets from the indexer.
                <br />
                <span className="font-mono text-[12px] text-driftwood">{error.message}</span>
              </p>
              <Button variant="PrimaryChestnut" onClick={refetch}>
                Retry
              </Button>
            </Card>
          </div>
        ) : visible.length === 0 ? (
          <div className="mx-auto max-w-[640px]">
            <Card className="flex flex-col items-center gap-4 text-center">
              {search ? (
                <p className="font-inter text-body text-bronze">No markets match your search.</p>
              ) : notInitialized ? (
                <>
                  <p className="font-inter text-body text-bronze">
                    The kassandra-market program is not set up — its on-chain Config account is
                    missing.
                  </p>
                  <p className="font-inter text-[13px] text-driftwood">
                    Deploy the program and initialize its Config before markets can appear.
                  </p>
                </>
              ) : (
                <p className="font-inter text-body text-bronze">
                  No markets found. The program is live but has no markets yet — create the first
                  one.
                </p>
              )}
              {!search && !notInitialized ? (
                <Link to="/markets/new">
                  <Button variant="PrimaryChestnut">Create a market</Button>
                </Link>
              ) : null}
            </Card>
          </div>
        ) : (
          <div className="grid grid-cols-1 gap-6 sm:grid-cols-2 lg:grid-cols-3">
            {groups.map((group) =>
              isCategorical(group) ? (
                <CategoricalCard key={group.oracle} group={group} />
              ) : (
                group.markets.map((summary) => (
                  <MarketCard key={summary.pubkey} summary={summary} />
                ))
              ),
            )}
          </div>
        )}
      </div>
    </main>
  );
}

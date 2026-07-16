import { useMemo, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { MarketStatus } from "@kassandra-market/markets";
import type { Address } from "@solana/web3.js";
import { Button, Card, EyebrowTag, SectionHeader, Tabs, TabPanel, type TabItem } from "../components/ui";
import { StatusChip } from "../components/markets/StatusChip";
import { FundingBar } from "../components/markets/FundingBar";
import { ProbabilityGauge } from "../components/markets/ProbabilityGauge";
import { Truncated } from "../components/markets/Truncated";
import { useOracleMeta } from "../hooks/useOracleMeta";
import { TradePanel } from "../components/markets/actions/TradePanel";
import {
  MarketLiquidityActions,
  MarketLifecycleActions,
} from "../components/markets/actions/MarketActions";
import { GroupLiquidityPanel } from "../components/markets/actions/GroupLiquidityPanel";
import { useMarketDetail } from "../market/hooks/useMarketDetail";
import { MarketNotFoundError, type MarketDetail as MarketDetailData } from "../market/data/markets";
import { explorerAddressUrl } from "../market/lib/explorer";
import {
  detailView,
  formatKass,
  impliedYesProbability,
  phaseLabel,
  outcomeResolutionText,
  truncateMiddle,
} from "../market/lib/marketView";

const ZERO_ADDRESS = "11111111111111111111111111111111";

const focusRing =
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40 " +
  "focus-visible:ring-offset-2 focus-visible:ring-offset-parchment";

/** A labelled address row: a copyable truncation + a Solana Explorer link. */
function AddressRow({ label, address }: { label: string; address: Address }) {
  const value = address.toString();
  const zero = value === ZERO_ADDRESS;
  return (
    <div className="flex items-center justify-between gap-3 py-1.5">
      <span className="font-inter text-[13px] text-driftwood">{label}</span>
      {zero ? (
        <span className="font-mono text-[12px] text-stone">set at activation</span>
      ) : (
        <span className="flex items-center gap-2">
          <Truncated value={value} label={label} copyable head={4} tail={4} />
          <a
            href={explorerAddressUrl(value)}
            target="_blank"
            rel="noreferrer"
            aria-label={`View ${label} on Solana Explorer`}
            className={`rounded-sm font-inter text-[11px] text-driftwood hover:text-ember-orange ${focusRing}`}
          >
            explorer ↗
          </a>
        </span>
      )}
    </div>
  );
}

/** A small titled Delphi card block. */
function Panel({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <Card className="flex flex-col gap-4">
      <h2 className="font-inter text-[13px] font-medium uppercase tracking-[0.08em] text-driftwood">
        {title}
      </h2>
      {children}
    </Card>
  );
}

/** One reserve figure in the implied-probability well. */
function ReserveFigure({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-3">
      <dt className="text-driftwood">{label}</dt>
      <dd className="font-medium tabular-nums text-sepia">{value}</dd>
    </div>
  );
}

function DetailBody({
  detail,
  refetch,
}: {
  detail: MarketDetailData;
  refetch: () => void;
}) {
  const { pubkey, market, contributions, oracle, reserves } = detail;
  const isActive = market.status === MarketStatus.Active;
  // The oracle outcome this sub-market binds to. YES = the oracle resolves to it.
  // Binary markets are `outcome 0 of 2`; a categorical oracle exposes N outcomes.
  const optionsCount = oracle?.optionsCount ?? null;
  const yesProbability = impliedYesProbability(reserves);

  // The human-readable question + option labels — on-chain (oracle_meta PDA),
  // read best-effort via the indexer. Absent (no indexer / not yet loaded) → the
  // view degrades to the pubkey + numeric outcome index it always had.
  const oracleKey = market.oracle.toString();
  const metaItems = useMemo(() => [oracleKey], [oracleKey]);
  const meta = useOracleMeta(metaItems).get(oracleKey);
  const subject = meta?.subject?.trim();
  const options = meta?.options ?? [];
  // The full-text label of the specific outcome THIS sub-market pays YES on.
  const boundLabel = options[market.outcomeIndex]?.trim() || null;

  // Tabs are grouped by intent: act on the AMM (Trade, Active only), provide/
  // withdraw + read market state (Liquidity — present in EVERY phase incl. Active,
  // and where funding progress + the implied price now live), run the lifecycle
  // cranks (Manage), and inspect the oracle + bindings (Details).
  const tabs = useMemo<TabItem[]>(() => {
    const items: TabItem[] = [];
    if (isActive) items.push({ id: "trade", label: "Trade", dot: "ember" });
    items.push({ id: "liquidity", label: "Liquidity" });
    items.push({ id: "manage", label: "Manage" });
    items.push({ id: "details", label: "Details" });
    return items;
  }, [isActive]);

  // Default to trading; a non-Active market (no Trade tab) opens on Liquidity.
  const defaultTab = isActive ? "trade" : "liquidity";
  const [tab, setTab] = useState(defaultTab);
  // If the active tab vanishes (e.g. an Active market resolves away the Trade tab
  // while it's open), fall back to the default so no dead panel is shown.
  const activeTab = tabs.some((t) => t.id === tab) ? tab : defaultTab;

  return (
    <div className="mt-8 flex flex-col gap-6">
      {/* Editorial header — the QUESTION leads (full text), then the specific
          outcome this sub-market pays YES on, in words. Mirrors the oracle page. */}
      <header>
        <EyebrowTag pill>Market</EyebrowTag>
        <h1 className="mt-3 text-balance font-serif text-heading font-light text-sepia">
          {subject ?? "Prediction market"}
        </h1>
        <p className="mt-3 font-inter text-body text-bronze">
          Pays <span className="font-medium text-ember-orange">YES</span> if the oracle resolves to{" "}
          {boundLabel ? (
            <span className="font-medium text-sepia">“{boundLabel}”</span>
          ) : (
            <span className="font-medium text-sepia">outcome {market.outcomeIndex}</span>
          )}
          {optionsCount !== null ? (
            <span className="text-driftwood">
              {" "}
              · outcome {market.outcomeIndex} of {optionsCount}
            </span>
          ) : null}
        </p>
        <div className="mt-4 flex flex-wrap items-center gap-x-4 gap-y-2 font-inter text-[13px] text-driftwood">
          <StatusChip status={market.status} />
          <span>{outcomeResolutionText(oracle, market.outcomeIndex)}</span>
          <Truncated value={pubkey} copyable label="market address" head={4} tail={4} />
        </div>
      </header>

      <Tabs items={tabs} value={activeTab} onChange={setTab} ariaLabel="Market sections" />

      {/* Trade — the price chart + buy/sell form (Active only; the cYES/cNO pool exists). */}
      {isActive ? (
        <TabPanel id="trade" active={activeTab === "trade"} className="tab-enter">
          <TradePanel
            pubkey={pubkey}
            market={market}
            reserves={reserves}
            onSuccess={refetch}
            question={subject}
            boundLabel={boundLabel}
          />
        </TabPanel>
      ) : null}

      {/* Liquidity — the market-state read (funding progress + the implied price an
          LP would provide at) sits atop bulk group liquidity, this market's own
          provide/withdraw surface, and the contributions ledger. Every phase. */}
      <TabPanel id="liquidity" active={activeTab === "liquidity"} className="tab-enter flex flex-col gap-6">
        <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
          {/* Funding progress + floor + protocol fee. */}
          <Panel title="Funding">
            <FundingBar market={market} />
            <dl className="flex flex-wrap gap-x-6 gap-y-1 font-inter text-[13px] text-bronze">
              <div className="flex gap-1">
                <dt className="text-driftwood">Raised</dt>
                <dd className="font-medium tabular-nums text-sepia">
                  {formatKass(market.totalContributed)} KASS
                </dd>
              </div>
              <div className="flex gap-1">
                <dt className="text-driftwood">Floor</dt>
                <dd className="font-medium tabular-nums text-sepia">
                  {formatKass(market.minLiquidity)} KASS
                </dd>
              </div>
              <div className="flex gap-1">
                <dt className="text-driftwood">Protocol fee</dt>
                <dd className="font-medium tabular-nums text-sepia">
                  {(market.feeBps / 100).toFixed(2)}%
                </dd>
              </div>
              {market.feeBps > 0 ? (
                <div className="flex gap-1">
                  <dt className="text-driftwood">Fee collected</dt>
                  <dd className="font-medium text-sepia">{market.feeCollected ? "yes" : "no"}</dd>
                </div>
              ) : null}
            </dl>
          </Panel>

          {/* Implied probability — the price an LP provides at (semicircle gauge +
              live cYES/cNO reserves) for an Active market. */}
          <Panel title="Implied probability">
            {isActive ? (
              <>
                <ProbabilityGauge probability={yesProbability} />
                {reserves ? (
                  <dl className="flex flex-col gap-1.5 border-t border-pebble pt-3 font-inter text-[13px]">
                    <ReserveFigure label="cYES reserve" value={formatKass(reserves.base)} />
                    <ReserveFigure label="cNO reserve" value={formatKass(reserves.quote)} />
                  </dl>
                ) : null}
              </>
            ) : (
              <p className="font-inter text-[13px] text-driftwood">
                Live prices appear once the market is Active (the cYES/cNO pool is composed at
                activation).
              </p>
            )}
          </Panel>
        </div>

        <GroupLiquidityPanel oracle={market.oracle.toString()} />
        <Panel title="Your liquidity">
          <MarketLiquidityActions detail={detail} refetch={refetch} />
        </Panel>
        <Panel title={`Contributions (${contributions.length})`}>
          {contributions.length === 0 ? (
            <p className="font-inter text-[13px] text-driftwood">No contributions yet.</p>
          ) : (
            <ul className="flex flex-col divide-y divide-pebble/60">
              {contributions.map(({ pubkey, contribution }) => (
                <li key={pubkey} className="flex items-center justify-between gap-3 py-2">
                  <Truncated
                    value={contribution.contributor.toString()}
                    label="contributor"
                    copyable
                    head={4}
                    tail={4}
                  />
                  <span className="flex items-center gap-3">
                    <span className="font-inter text-[13px] font-medium tabular-nums text-sepia">
                      {formatKass(contribution.amount)} KASS
                    </span>
                    <span
                      className={`font-inter text-[11px] ${
                        contribution.claimed ? "text-stone" : "text-chestnut"
                      }`}
                    >
                      {contribution.claimed ? "claimed" : "open"}
                    </span>
                  </span>
                </li>
              ))}
            </ul>
          )}
        </Panel>
      </TabPanel>

      {/* Manage — the lifecycle cranks (activate / resolve / redeem / collect / close). */}
      <TabPanel id="manage" active={activeTab === "manage"} className="tab-enter">
        <Panel title="Lifecycle actions">
          <MarketLifecycleActions detail={detail} refetch={refetch} />
        </Panel>
      </TabPanel>

      {/* Details — the linked oracle context + the MetaDAO bindings (accounts +
          Explorer links). */}
      <TabPanel id="details" active={activeTab === "details"} className="tab-enter flex flex-col gap-6">
        <Panel title="Linked oracle">
          {subject ? (
            <p className="text-balance font-serif text-subheading font-light text-sepia">
              “{subject}”
            </p>
          ) : null}
          {options.length > 0 ? (
            <div className="flex flex-wrap gap-2" aria-label="Oracle options">
              {options.map((opt, i) => {
                const bound = i === market.outcomeIndex;
                return (
                  <span
                    key={i}
                    className={`rounded-tag border px-2.5 py-1 font-inter text-[13px] ${
                      bound
                        ? "border-ember-orange/50 bg-soft-cream text-sepia"
                        : "border-pebble bg-soft-cream text-bronze"
                    }`}
                  >
                    <span className="tabular-nums text-driftwood">{i}</span>
                    <span className="mx-1 text-driftwood">·</span>
                    {opt}
                    {bound ? <span className="ml-1.5 text-[11px] text-ember-orange">YES</span> : null}
                  </span>
                );
              })}
            </div>
          ) : null}
          {oracle ? (
            <dl className="flex flex-wrap gap-x-8 gap-y-1 font-inter text-[13px] text-bronze">
              <div className="flex gap-1">
                <dt className="text-driftwood">Phase</dt>
                <dd className="font-medium text-sepia">{phaseLabel(oracle.phase)}</dd>
              </div>
              <div className="flex gap-1">
                <dt className="text-driftwood">Options</dt>
                <dd className="font-medium tabular-nums text-sepia">{oracle.optionsCount}</dd>
              </div>
              <div className="flex gap-1">
                <dt className="text-driftwood">Outcome</dt>
                <dd className="font-medium text-sepia">
                  {outcomeResolutionText(oracle, market.outcomeIndex)}
                </dd>
              </div>
            </dl>
          ) : (
            <p className="font-inter text-[13px] text-driftwood">
              The linked oracle account could not be read.
            </p>
          )}
        </Panel>

        <Panel title="Bindings">
          <div className="divide-y divide-pebble/60">
            <AddressRow label="Oracle" address={market.oracle} />
            <AddressRow label="Creator" address={market.creator} />
            <AddressRow label="KASS mint" address={market.kassMint} />
            <AddressRow label="Escrow vault" address={market.escrowVault} />
            <AddressRow label="Question" address={market.question} />
            <AddressRow label="Conditional vault" address={market.vault} />
            <AddressRow label="cYES mint" address={market.yesMint} />
            <AddressRow label="cNO mint" address={market.noMint} />
            <AddressRow label="AMM pool" address={market.amm} />
            <AddressRow label="LP mint" address={market.lpMint} />
            <AddressRow label="LP vault" address={market.lpVault} />
          </div>
        </Panel>
      </TabPanel>
    </div>
  );
}

export default function MarketDetail() {
  const { pubkey } = useParams<{ pubkey: string }>();
  const { data, loading, error, refetch, refetchAfterWrite } = useMarketDetail(pubkey);
  // `ready` whenever we hold data for THIS market, so the Active-market 15s poll
  // refreshes in place instead of blanking to the skeleton and remounting (which
  // would wipe in-progress trade/contribute form fields).
  const view = detailView(pubkey, data, loading, error);

  return (
    <main className="mx-auto max-w-[1000px] px-6 py-20">
      <Link
        to="/markets"
        className={`inline-block font-inter text-[13px] text-driftwood hover:text-sepia ${focusRing}`}
      >
        ← All markets
      </Link>

      {view === "ready" && data ? (
        // Actions use the reconcile-lag-resilient refetch so the UI reliably
        // reflects a just-confirmed write (e.g. Funding → Active after activate).
        <DetailBody detail={data} refetch={refetchAfterWrite} />
      ) : view === "error" && error ? (
        <div className="mx-auto mt-10 max-w-[640px]">
          <div className="mb-6 text-center">
            <SectionHeader
              as="h1"
              eyebrow="Market"
              line1={
                <span className="font-mono text-heading-sm">
                  {pubkey ? truncateMiddle(pubkey, 6, 6) : "Market"}
                </span>
              }
            />
          </div>
          <Card className="flex flex-col items-center gap-4 text-center">
            <p className="font-inter text-body text-bronze">
              {error instanceof MarketNotFoundError
                ? "This market was not found."
                : `Could not load this market: ${error.message}`}
            </p>
            <Button variant="PrimaryChestnut" onClick={refetch}>
              Retry
            </Button>
          </Card>
        </div>
      ) : view === "loading" ? (
        <div className="mt-10 flex flex-col gap-6" aria-hidden="true">
          {Array.from({ length: 3 }, (_, i) => (
            <Card key={i} className="h-28 animate-pulse bg-pure-card">
              <span className="sr-only">Loading</span>
            </Card>
          ))}
        </div>
      ) : null}
    </main>
  );
}

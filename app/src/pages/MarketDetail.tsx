import { Link, useParams } from "react-router-dom";
import { MarketStatus } from "@kassandra-market/sdk";
import type { Address } from "@solana/web3.js";
import { Button, Card, SectionHeader } from "../components/ui";
import { StatusChip } from "../components/markets/StatusChip";
import { FundingBar } from "../components/markets/FundingBar";
import { ProbabilityBar } from "../components/markets/ProbabilityBar";
import { Truncated } from "../components/markets/Truncated";
import { MarketActions } from "../components/markets/actions/MarketActions";
import { useMarketDetail } from "../market/hooks/useMarketDetail";
import { MarketNotFoundError, type MarketDetail as MarketDetailData } from "../market/data/markets";
import { explorerAddressUrl } from "../market/lib/explorer";
import {
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

function DetailBody({
  detail,
  refetch,
}: {
  detail: MarketDetailData;
  refetch: () => void;
}) {
  const { market, contributions, oracle, reserves } = detail;
  const isActive = market.status === MarketStatus.Active;
  // A settled (terminal) market can eventually be closed to reclaim its account
  // rent — but only once every contributor has exited (openContributions === 0).
  const marketTerminal =
    market.status === MarketStatus.Resolved ||
    market.status === MarketStatus.Void ||
    market.status === MarketStatus.Cancelled;
  // The oracle outcome this sub-market binds to. YES = the oracle resolves to it.
  // Binary markets are `outcome 0 of 2`; a categorical oracle exposes N outcomes.
  const optionsCount = oracle?.optionsCount ?? null;

  return (
    <div className="mt-10 flex flex-col gap-6">
      {/* Status + resolution */}
      <Panel title="Status">
        <div className="flex flex-wrap items-center gap-3">
          <StatusChip status={market.status} />
          <span className="font-inter text-body text-sepia">{outcomeResolutionText(oracle, market.outcomeIndex)}</span>
          {market.settled ? (
            <span className="font-inter text-[12px] text-chestnut">· settled on-chain</span>
          ) : null}
        </div>
        <p className="font-inter text-[13px] text-driftwood">
          <span className="font-medium text-bronze">
            Outcome {market.outcomeIndex}
            {optionsCount !== null ? ` of ${optionsCount}` : ""}
          </span>{" "}
          — YES pays if the oracle resolves to outcome {market.outcomeIndex}.
        </p>
        {marketTerminal ? (
          <p className="font-inter text-[13px] text-driftwood">
            {market.openContributions > 0
              ? `${market.openContributions} contributor${
                  market.openContributions === 1 ? "" : "s"
                } yet to claim.`
              : "All contributions claimed — ready to close."}{" "}
            <span className="text-stone">
              Closing the market reclaims its account rent to the creator.
            </span>
          </p>
        ) : null}
      </Panel>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
        {/* Funding */}
        <Panel title="Funding">
          <FundingBar market={market} />
          <dl className="flex flex-wrap gap-x-6 gap-y-1 font-inter text-[13px] text-bronze">
            <div className="flex gap-1">
              <dt className="text-driftwood">Raised</dt>
              <dd className="font-medium text-sepia">{formatKass(market.totalContributed)} KASS</dd>
            </div>
            <div className="flex gap-1">
              <dt className="text-driftwood">Floor</dt>
              <dd className="font-medium text-sepia">{formatKass(market.minLiquidity)} KASS</dd>
            </div>
            <div className="flex gap-1">
              <dt className="text-driftwood">Protocol fee</dt>
              <dd className="font-medium text-sepia">{(market.feeBps / 100).toFixed(2)}%</dd>
            </div>
            {market.feeBps > 0 ? (
              <div className="flex gap-1">
                <dt className="text-driftwood">Fee collected</dt>
                <dd className="font-medium text-sepia">{market.feeCollected ? "yes" : "no"}</dd>
              </div>
            ) : null}
          </dl>
        </Panel>

        {/* Implied probability (Active only) */}
        <Panel title="Implied probability">
          {isActive ? (
            <>
              <ProbabilityBar probability={impliedYesProbability(reserves)} />
              {reserves ? (
                <dl className="flex flex-wrap gap-x-6 gap-y-1 font-inter text-[13px] text-bronze">
                  <div className="flex gap-1">
                    <dt className="text-driftwood">cYES reserve</dt>
                    <dd className="font-mono text-sepia">{reserves.base.toString()}</dd>
                  </div>
                  <div className="flex gap-1">
                    <dt className="text-driftwood">cNO reserve</dt>
                    <dd className="font-mono text-sepia">{reserves.quote.toString()}</dd>
                  </div>
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

      {/* Linked oracle */}
      <Panel title="Linked oracle">
        {oracle ? (
          <dl className="flex flex-wrap gap-x-8 gap-y-1 font-inter text-[13px] text-bronze">
            <div className="flex gap-1">
              <dt className="text-driftwood">Phase</dt>
              <dd className="font-medium text-sepia">{phaseLabel(oracle.phase)}</dd>
            </div>
            <div className="flex gap-1">
              <dt className="text-driftwood">Options</dt>
              <dd className="font-medium text-sepia">{oracle.optionsCount}</dd>
            </div>
            <div className="flex gap-1">
              <dt className="text-driftwood">Outcome</dt>
              <dd className="font-medium text-sepia">{outcomeResolutionText(oracle, market.outcomeIndex)}</dd>
            </div>
          </dl>
        ) : (
          <p className="font-inter text-[13px] text-driftwood">
            The linked oracle account could not be read.
          </p>
        )}
      </Panel>

      {/* MetaDAO bindings */}
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

      {/* Contributions */}
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
                  <span className="font-inter text-[13px] font-medium text-sepia">
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

      {/* Status-gated write actions (contribute / cancel / refund; trade etc. in Task 4). */}
      <MarketActions detail={detail} refetch={refetch} />
    </div>
  );
}

export default function MarketDetail() {
  const { pubkey } = useParams<{ pubkey: string }>();
  const { data, loading, error, refetch, refetchAfterWrite } = useMarketDetail(pubkey);

  return (
    <main className="mx-auto max-w-[900px] px-6 py-20">
      <div className="mx-auto flex max-w-[640px] flex-col items-center text-center">
        <Link
          to="/markets"
          className={`mb-4 font-inter text-[13px] text-driftwood hover:text-sepia ${focusRing}`}
        >
          ← All markets
        </Link>
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

      {loading ? (
        <div className="mt-10 flex flex-col gap-6" aria-hidden="true">
          {Array.from({ length: 3 }, (_, i) => (
            <Card key={i} className="h-28 animate-pulse bg-pure-card">
              <span className="sr-only">Loading</span>
            </Card>
          ))}
        </div>
      ) : error ? (
        <div className="mx-auto mt-10 max-w-[640px]">
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
      ) : data ? (
        // Actions use the reconcile-lag-resilient refetch so the UI reliably
        // reflects a just-confirmed write (e.g. Funding → Active after activate).
        <DetailBody detail={data} refetch={refetchAfterWrite} />
      ) : null}
    </main>
  );
}

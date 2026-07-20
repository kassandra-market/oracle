import { useMemo } from "react";
import { Link, useParams, useSearchParams } from "react-router-dom";
import { MarketStatus, isTerminal } from "@kassandra-market/markets";
import type { Address } from "@solana/web3.js";
import { Button, Card, EyebrowTag, SectionHeader, Tabs, TabPanel, type TabItem } from "../components/ui";
import { StatusChip } from "../components/markets/StatusChip";
import { FundingBar } from "../components/markets/FundingBar";
import { ProbabilityGauge } from "../components/markets/ProbabilityGauge";
import { Truncated } from "../components/markets/Truncated";
import { useOracleMeta } from "../hooks/useOracleMeta";
import { GroupTradePanel } from "../components/markets/actions/GroupTradePanel";
import {
  MarketLiquidityActions,
  MarketLifecycleActions,
} from "../components/markets/actions/MarketActions";
import { GroupLiquidityPanel } from "../components/markets/actions/GroupLiquidityPanel";
import { useMarketDetail } from "../market/hooks/useMarketDetail";
import { useOracleGroup, type OracleGroupState } from "../market/hooks/useOracleGroup";
import { MarketNotFoundError, type MarketDetail as MarketDetailData } from "../market/data/markets";
import { explorerAddressUrl } from "../market/lib/explorer";
import { useWallet } from "@solana/wallet-adapter-react";
import {
  contributorLp,
  detailView,
  formatKass,
  impliedYesProbability,
  phaseLabel,
  outcomeResolutionText,
  truncateMiddle,
} from "../market/lib/marketView";

const ZERO_ADDRESS = "11111111111111111111111111111111";

const focusRing =
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-platinum/40 " +
  "focus-visible:ring-offset-2 focus-visible:ring-offset-liquid-abyss";

/** A labelled address row: a copyable truncation + a Solana Explorer link. */
function AddressRow({ label, address }: { label: string; address: Address }) {
  const value = address.toString();
  const zero = value === ZERO_ADDRESS;
  return (
    <div className="flex items-center justify-between gap-3 py-1.5">
      <span className="font-inter text-[13px] text-silver">{label}</span>
      {zero ? (
        <span className="font-mono text-[12px] text-silver-dim">set at activation</span>
      ) : (
        <span className="flex items-center gap-2">
          <Truncated value={value} label={label} copyable head={4} tail={4} />
          <a
            href={explorerAddressUrl(value)}
            target="_blank"
            rel="noreferrer"
            aria-label={`View ${label} on Solana Explorer`}
            className={`rounded-sm font-inter text-[11px] text-silver hover:text-coral ${focusRing}`}
          >
            explorer ↗
          </a>
        </span>
      )}
    </div>
  );
}

/** A small titled card block. */
function Panel({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <Card className="flex flex-col gap-4">
      <h2 className="font-inter text-[13px] font-medium uppercase tracking-[0.08em] text-silver">
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
      <dt className="text-silver">{label}</dt>
      <dd className="font-medium tabular-nums text-platinum">{value}</dd>
    </div>
  );
}

/** A small hoverable "i" carrying an explanatory tooltip (native `title` +
 *  `aria-label`, matching the app's other title-based tips). */
function InfoTip({ label, children }: { label: string; children: string }) {
  return (
    <span
      role="img"
      aria-label={`${label}: ${children}`}
      title={children}
      className="ml-1 inline-flex h-4 w-4 cursor-help select-none items-center justify-center rounded-full border border-hairline align-middle font-inter text-[10px] leading-none text-silver"
    >
      i
    </span>
  );
}

/** `part / whole` as a one-decimal percent string (`"0%"` when `whole <= 0`), via
 *  an integer-basis-point intermediate so the bigint LP figures never lose data. */
function percentOf(part: bigint, whole: bigint): string {
  if (whole <= 0n) return "0%";
  return `${(Number((part * 10_000n) / whole) / 100).toFixed(1)}%`;
}

/** A headline overview figure: a small caps label over a large serif value. */
function StatTile({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div className="flex flex-col gap-1">
      <span className="font-inter text-[11px] uppercase tracking-[0.06em] text-silver">
        {label}
      </span>
      <span className="font-serif text-heading-sm font-light tabular-nums text-platinum">{value}</span>
      {sub ? <span className="font-inter text-[12px] text-silver">{sub}</span> : null}
    </div>
  );
}

/** The action that produced a contribution row: initial Funding vs added Liquidity. */
function ContribTag({ kind }: { kind: "funding" | "liquidity" }) {
  const funding = kind === "funding";
  return (
    <span
      className={`shrink-0 rounded-tag border px-2 py-0.5 font-inter text-[11px] ${
        funding
          ? "border-cyan-phosphor/30 bg-cyan-phosphor/10 text-cyan-phosphor"
          : "border-aqua/30 bg-aqua/10 text-aqua"
      }`}
    >
      {funding ? "Initial funding" : "Liquidity"}
    </span>
  );
}

/**
 * The top-of-tab LP overview: the pool's cYES/cNO token amounts + LP supply +
 * the connected wallet's share once a pool exists (activated); before
 * activation it degrades to funding progress + the wallet's stake (there is no
 * LP yet). Shows each side's actual reserve amount rather than a single
 * KASS-denominated "pool value" — the latter is a mark-to-market figure that
 * moves with the trade itself, so it reads as an odd, unstable headline number.
 *
 * Pre-activation on a GROUPED market (`group.isGroup`), the bar + "Raised"
 * figure are the GROUP's cumulative funding (summed across every outcome still
 * Funding), not just this one outcome — matching the single cumulative bar in
 * `GroupLiquidityPanel` below, since that panel is the only place funding for a
 * group happens. A lone (non-grouped) market shows its own numbers, unchanged.
 */
function LiquidityOverview({ detail, group }: { detail: MarketDetailData; group: OracleGroupState }) {
  const { market, reserves, contributions } = detail;
  const { publicKey } = useWallet();
  const address = publicKey?.toBase58() ?? null;
  const mine = address
    ? contributions.find((c) => c.contribution.contributor.toString() === address)
    : undefined;

  // A pool exists (activated) → LP-denominated overview.
  if (market.grossLpTotal > 0n) {
    const yourLp = mine ? contributorLp(mine.contribution, market) : 0n;
    return (
      <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
        <StatTile label="cYES" value={reserves ? formatKass(reserves.base) : "—"} />
        <StatTile label="cNO" value={reserves ? formatKass(reserves.quote) : "—"} />
        <StatTile label="LP supply" value={`${formatKass(market.grossLpTotal)} shares`} />
        <StatTile
          label="Your share"
          value={address ? percentOf(yourLp, market.grossLpTotal) : "—"}
          sub={address ? `${formatKass(yourLp)} LP` : "Connect a wallet to see your share"}
        />
      </div>
    );
  }

  // Pre-activation (Funding) → funding progress + the wallet's stake. Grouped →
  // the cumulative bar/figure across every outcome still Funding; lone → this
  // market's own numbers (`group.funding` is empty for a lone market).
  const fundingMarket = group.isGroup
    ? {
        totalContributed: group.funding.reduce((sum, m) => sum + m.market.totalContributed, 0n),
        minLiquidity: group.funding.reduce((sum, m) => sum + m.market.minLiquidity, 0n),
      }
    : market;
  const yourStake = mine?.contribution.amount ?? 0n;
  return (
    <div className="flex flex-col gap-4">
      <FundingBar market={fundingMarket} />
      <div className="grid grid-cols-2 gap-4 sm:grid-cols-3">
        <StatTile label="Raised" value={`${formatKass(fundingMarket.totalContributed)} KASS`} />
        <StatTile
          label="Your stake"
          value={address ? percentOf(yourStake, fundingMarket.totalContributed) : "—"}
          sub={address ? `${formatKass(yourStake)} KASS` : "Connect a wallet to see your stake"}
        />
      </div>
    </div>
  );
}

/**
 * The contributions ledger, latest-first (the data layer sorts by the
 * Contribution PDA's last-write slot). Each contribution expands into up to two
 * tagged rows — the initial Funding stake (KASS) and any post-activation
 * Liquidity added (LP) — with the LP row (always the later action) above the
 * funding row for the same contributor.
 */
function ContributionsLedger({
  contributions,
}: {
  contributions: MarketDetailData["contributions"];
}) {
  if (contributions.length === 0) {
    return <p className="font-inter text-[13px] text-silver">No contributions yet.</p>;
  }
  type Row = {
    key: string;
    contributor: string;
    kind: "funding" | "liquidity";
    amount: string;
    claimed: boolean;
  };
  const rows: Row[] = [];
  for (const { pubkey, contribution: c } of contributions) {
    const contributor = c.contributor.toString();
    // LP first (post-activation → the later action), then the funding stake.
    if (c.lateLp > 0n)
      rows.push({
        key: `${pubkey}:lp`,
        contributor,
        kind: "liquidity",
        amount: `${formatKass(c.lateLp)} LP`,
        claimed: c.claimed,
      });
    if (c.amount > 0n)
      rows.push({
        key: `${pubkey}:fund`,
        contributor,
        kind: "funding",
        amount: `${formatKass(c.amount)} KASS`,
        claimed: c.claimed,
      });
    // A degenerate all-zero contribution still gets one row so it isn't dropped.
    if (c.lateLp === 0n && c.amount === 0n)
      rows.push({
        key: `${pubkey}:fund`,
        contributor,
        kind: "funding",
        amount: "0 KASS",
        claimed: c.claimed,
      });
  }
  return (
    <ul className="flex flex-col divide-y divide-hairline/60">
      {rows.map((r) => (
        <li key={r.key} className="flex items-center justify-between gap-3 py-2">
          <span className="flex min-w-0 items-center gap-2">
            <ContribTag kind={r.kind} />
            <Truncated value={r.contributor} label="contributor" copyable head={4} tail={4} />
          </span>
          <span className="flex items-center gap-3">
            <span className="font-inter text-[13px] font-medium tabular-nums text-platinum">
              {r.amount}
            </span>
            <span
              className={`font-inter text-[11px] ${r.claimed ? "text-silver-dim" : "text-aqua"}`}
            >
              {r.claimed ? "claimed" : "open"}
            </span>
          </span>
        </li>
      ))}
    </ul>
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

  // The categorical group this market's oracle spans (empty/lone for a binary
  // market) — computed once here so the per-outcome contribute form and the
  // cumulative funding bar agree on the exact same set of sibling markets.
  const group = useOracleGroup(market.oracle.toString());
  // Every sub-market in the group shares this ONE oracle, so its phase gates
  // activation identically for all of them — computed once here (rather than
  // per-outcome) and handed to GroupLiquidityPanel's funding→activation handoff.
  const oracleTerminal = oracle ? isTerminal(oracle.phase) : false;

  // LP provenance (only meaningful once the pool is seeded at activation):
  //   - funding LP  = `activationLp` (minted from the funders' escrow at activate),
  //   - independent LP = the rest of `grossLpTotal` (post-activation add_liquidity).
  // `grossLpTotal` is the frozen who-provided-what total (fee-independent), so it's
  // the honest basis for the split even after `collect_fee` trims `lpTotal`.
  const grossLp = market.grossLpTotal;
  const fundingLp = market.activationLp;
  const independentLp = grossLp > fundingLp ? grossLp - fundingLp : 0n;
  const hasPool = grossLp > 0n;

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

  // Trade is available whenever SOME outcome in the group has a live pool — this
  // market's own (isActive) or a sibling's — since GroupTradePanel lets the page
  // trade any of them without navigating away. A still-Funding outcome viewed
  // directly thus still gets a Trade tab as soon as one sibling activates.
  const hasTradableOutcome = isActive || group.active.some((m) => m.pubkey !== pubkey);

  // Tabs are grouped by intent: act on the AMM (Trade, whenever an outcome is
  // tradable), provide/withdraw + read funding & pool composition (Liquidity —
  // present in EVERY phase incl. Active), run the lifecycle cranks (Manage), and
  // inspect the implied probability + oracle + bindings (Details).
  const tabs = useMemo<TabItem[]>(() => {
    const items: TabItem[] = [];
    if (hasTradableOutcome) items.push({ id: "trade", label: "Trade", dot: "coral" });
    items.push({ id: "liquidity", label: "Liquidity" });
    items.push({ id: "manage", label: "Manage" });
    items.push({ id: "details", label: "Details" });
    return items;
  }, [hasTradableOutcome]);

  // Default to trading only when THIS outcome is itself Active — a Funding-phase
  // page defaults to Liquidity (what brought you here) even if a sibling is
  // already tradable; Trade is still one click away via the tab.
  const defaultTab = isActive ? "trade" : "liquidity";
  // The active tab lives in the URL (`?tab=`) so a refresh (or a shared link)
  // restores it. An absent or stale param (e.g. an Active market resolved away the
  // Trade tab) falls back to the default so no dead panel is shown. `replace` keeps
  // tab switches out of the history stack (Back leaves the page, not the tab).
  const [searchParams, setSearchParams] = useSearchParams();
  const paramTab = searchParams.get("tab");
  const activeTab = tabs.some((t) => t.id === paramTab) ? paramTab! : defaultTab;
  const setTab = (id: string) => {
    const next = new URLSearchParams(searchParams);
    next.set("tab", id);
    setSearchParams(next, { replace: true });
  };

  return (
    <div className="mt-8 flex flex-col gap-6">
      {/* Editorial header — the QUESTION leads (full text), then the specific
          outcome this sub-market pays YES on, in words. Mirrors the oracle page. */}
      <header>
        <EyebrowTag pill>Market</EyebrowTag>
        <h1 className="mt-3 text-balance font-serif text-heading font-light text-platinum">
          {subject ?? "Prediction market"}
        </h1>
        <p className="mt-3 font-inter text-body text-silver">
          Pays <span className="font-medium text-coral">YES</span> if the oracle resolves to{" "}
          {boundLabel ? (
            <span className="font-medium text-platinum">“{boundLabel}”</span>
          ) : (
            <span className="font-medium text-platinum">outcome {market.outcomeIndex}</span>
          )}
          {optionsCount !== null ? (
            <span className="text-silver">
              {" "}
              · outcome {market.outcomeIndex} of {optionsCount}
            </span>
          ) : null}
        </p>
        <div className="mt-4 flex flex-wrap items-center gap-x-4 gap-y-2 font-inter text-[13px] text-silver">
          <StatusChip status={market.status} />
          <span>{outcomeResolutionText(oracle, market.outcomeIndex)}</span>
          <Truncated value={pubkey} copyable label="market address" head={4} tail={4} />
          <Link
            to={`/oracles/${market.oracle.toString()}`}
            className={`font-inter text-[13px] font-medium text-aqua hover:text-coral ${focusRing}`}
          >
            View oracle →
          </Link>
        </div>
      </header>

      <Tabs items={tabs} value={activeTab} onChange={setTab} ariaLabel="Market sections" />

      {/* Trade — the price chart + buy/sell form, unified across every tradable
          outcome in the group (an outcome selector above the order ticket when
          more than one is Active; nothing to select for a lone/binary market). */}
      {hasTradableOutcome ? (
        <TabPanel id="trade" active={activeTab === "trade"} className="tab-enter">
          <GroupTradePanel
            detail={detail}
            group={group}
            subject={subject}
            options={options}
            refetch={refetch}
          />
        </TabPanel>
      ) : null}

      {/* Liquidity — TWO panels. Top: the LP overview (cYES/cNO reserves, LP supply,
          your share) + the phase-appropriate provide/claim form, folding in bulk
          group liquidity for a categorical group. Bottom: the detailed pool
          composition + the tagged, latest-first contributions ledger. Present in
          every phase. */}
      <TabPanel id="liquidity" active={activeTab === "liquidity"} className="tab-enter flex flex-col gap-6">
        <Panel title="Liquidity">
          <LiquidityOverview detail={detail} group={group} />
          <div className="border-t border-hairline pt-5">
            <MarketLiquidityActions detail={detail} refetch={refetch} isGrouped={group.isGroup} />
          </div>
          <GroupLiquidityPanel
            group={group}
            embedded
            oracleTerminal={oracleTerminal}
            onSuccess={refetch}
          />
        </Panel>

        <Panel title="Pool composition & contributions">
          {/* LP composition — funders' seed LP vs independent post-activation LP. */}
          {hasPool ? (
            <div className="flex flex-col gap-1.5">
              <p className="font-inter text-[12px] font-medium uppercase tracking-[0.06em] text-silver">
                LP composition
                <InfoTip label="LP composition">
                  Funding LP was minted from the funders' escrow when the market was activated;
                  independent LP was added later by anyone depositing into the live pool. Both share
                  the pool pro-rata by LP.
                </InfoTip>
              </p>
              <dl className="flex flex-col gap-1.5 font-inter text-[13px]">
                <ReserveFigure
                  label={`From funding (${percentOf(fundingLp, grossLp)})`}
                  value={`${formatKass(fundingLp)} LP`}
                />
                <ReserveFigure
                  label={`From independent LPs (${percentOf(independentLp, grossLp)})`}
                  value={`${formatKass(independentLp)} LP`}
                />
                <ReserveFigure label="Total LP" value={`${formatKass(grossLp)} LP`} />
              </dl>
            </div>
          ) : null}

          {/* Pool composition — the underlying cYES/cNO token reserves. */}
          {reserves ? (
            <div className="flex flex-col gap-1.5 border-t border-hairline pt-3">
              <p className="font-inter text-[12px] font-medium uppercase tracking-[0.06em] text-silver">
                Pool composition
                <InfoTip label="Pool composition">
                  The AMM holds a pair of conditional tokens: cYES pays 1 KASS if the outcome
                  resolves YES, cNO pays 1 KASS if it resolves NO. Their reserves set the price.
                </InfoTip>
              </p>
              <dl className="flex flex-col gap-1.5 font-inter text-[13px]">
                <ReserveFigure label="cYES (pays 1 KASS on YES)" value={formatKass(reserves.base)} />
                <ReserveFigure label="cNO (pays 1 KASS on NO)" value={formatKass(reserves.quote)} />
              </dl>
            </div>
          ) : null}

          {/* Pool details — raised / floor / protocol fee. */}
          <dl className="flex flex-wrap gap-x-6 gap-y-1 border-t border-hairline pt-3 font-inter text-[13px] text-silver">
            <div className="flex gap-1">
              <dt className="text-silver">Raised</dt>
              <dd className="font-medium tabular-nums text-platinum">
                {formatKass(market.totalContributed)} KASS
              </dd>
            </div>
            <div className="flex gap-1">
              <dt className="text-silver">Floor</dt>
              <dd className="font-medium tabular-nums text-platinum">
                {formatKass(market.minLiquidity)} KASS
              </dd>
            </div>
            <div className="flex gap-1">
              <dt className="text-silver">Protocol fee</dt>
              <dd className="font-medium tabular-nums text-platinum">
                {(market.feeBps / 100).toFixed(2)}%
              </dd>
            </div>
            {market.feeBps > 0 ? (
              <div className="flex gap-1">
                <dt className="text-silver">Fee collected</dt>
                <dd className="font-medium text-platinum">{market.feeCollected ? "yes" : "no"}</dd>
              </div>
            ) : null}
          </dl>

          {/* Contributions — split into tagged (Initial funding / Liquidity) rows,
              latest-first by the Contribution PDA's last-write slot. */}
          <div className="flex flex-col gap-2 border-t border-hairline pt-3">
            <p className="font-inter text-[12px] font-medium uppercase tracking-[0.06em] text-silver">
              Contributions ({contributions.length})
            </p>
            <ContributionsLedger contributions={contributions} />
          </div>
        </Panel>
      </TabPanel>

      {/* Manage — the lifecycle cranks (activate / resolve / redeem / collect / close). */}
      <TabPanel id="manage" active={activeTab === "manage"} className="tab-enter">
        <Panel title="Lifecycle actions">
          <MarketLifecycleActions detail={detail} refetch={refetch} />
        </Panel>
      </TabPanel>

      {/* Details — the implied probability read, the linked oracle context, and the
          MetaDAO bindings (accounts + Explorer links). */}
      <TabPanel id="details" active={activeTab === "details"} className="tab-enter flex flex-col gap-6">
        <Panel title="Implied probability">
          {isActive ? (
            <>
              <p className="font-inter text-[13px] text-silver">
                The market's live estimate that this outcome resolves{" "}
                <span className="font-medium text-coral">YES</span>.
                <InfoTip label="What is implied probability">
                  Implied probability is the market's estimate of the chance this outcome resolves
                  YES, read from the pool price: P(YES) = cNO ÷ (cYES + cNO). A larger cNO reserve
                  (cheaper NO) prices YES as more likely.
                </InfoTip>
              </p>
              <ProbabilityGauge probability={yesProbability} />
            </>
          ) : (
            <p className="font-inter text-[13px] text-silver">
              Live prices appear once the market is Active (the cYES/cNO pool is composed at
              activation).
            </p>
          )}
        </Panel>

        <Panel title="Linked oracle">
          {subject ? (
            <p className="text-balance font-serif text-subheading font-light text-platinum">
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
                        ? "border-coral/50 bg-liquid-deep text-platinum"
                        : "border-hairline bg-liquid-deep text-silver"
                    }`}
                  >
                    <span className="tabular-nums text-silver">{i}</span>
                    <span className="mx-1 text-silver">·</span>
                    {opt}
                    {bound ? <span className="ml-1.5 text-[11px] text-coral">YES</span> : null}
                  </span>
                );
              })}
            </div>
          ) : null}
          {oracle ? (
            <dl className="flex flex-wrap gap-x-8 gap-y-1 font-inter text-[13px] text-silver">
              <div className="flex gap-1">
                <dt className="text-silver">Phase</dt>
                <dd className="font-medium text-platinum">{phaseLabel(oracle.phase)}</dd>
              </div>
              <div className="flex gap-1">
                <dt className="text-silver">Options</dt>
                <dd className="font-medium tabular-nums text-platinum">{oracle.optionsCount}</dd>
              </div>
              <div className="flex gap-1">
                <dt className="text-silver">Outcome</dt>
                <dd className="font-medium text-platinum">
                  {outcomeResolutionText(oracle, market.outcomeIndex)}
                </dd>
              </div>
            </dl>
          ) : (
            <p className="font-inter text-[13px] text-silver">
              The linked oracle account could not be read.
            </p>
          )}
          <Link
            to={`/oracles/${market.oracle.toString()}`}
            className={`font-inter text-[13px] font-medium text-aqua hover:text-coral ${focusRing}`}
          >
            Open oracle page →
          </Link>
        </Panel>

        <Panel title="Bindings">
          <div className="divide-y divide-hairline/60">
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
        className={`inline-block font-inter text-[13px] text-silver hover:text-platinum ${focusRing}`}
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
            <p className="font-inter text-body text-silver">
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
            <Card key={i} className="h-28 animate-pulse bg-liquid-kelp">
              <span className="sr-only">Loading</span>
            </Card>
          ))}
        </div>
      ) : null}
    </main>
  );
}

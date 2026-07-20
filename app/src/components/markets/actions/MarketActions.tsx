import { useState } from "react";
import { MarketStatus, isTerminal } from "@kassandra-market/markets";
import type { MarketDetail } from "../../../market/data/markets";
import { fundingActions, fundingProgress } from "../../../market/lib/marketView";
import { useConfig } from "../../../market/hooks/useMarketDetail";
import { ContributeForm } from "./ContributeForm";
import { AddLiquidityControl } from "./AddLiquidityControl";
import { CancelControl } from "./CancelControl";
import { RefundControl } from "./RefundControl";
import { ActivateControl } from "./ActivateControl";
import { ClaimLpControl } from "./ClaimLpControl";
import { ResolveControl } from "./ResolveControl";
import { RedeemControl } from "./RedeemControl";
import { CollectFeeControl } from "./CollectFeeControl";
import { CloseMarketControl } from "./CloseMarketControl";

/**
 * A small terminal-market note: how many contributors have yet to exit (claim/
 * refund) before the market can be closed, or that it's ready to close. It's the
 * human-readable form of the `close_market` gate (`openContributions === 0`).
 */
function ContributorsRemaining({ open }: { open: number }) {
  return (
    <p className="font-inter text-[12px] text-silver">
      {open > 0
        ? `${open} contributor${open === 1 ? "" : "s"} yet to claim — the market can be closed once all have exited.`
        : "All contributions claimed — the market is ready to close."}
    </p>
  );
}

/** A calm empty-state note for a tab whose actions aren't available in this phase. */
function NoActions({ children }: { children: React.ReactNode }) {
  return <p className="font-inter text-[13px] text-silver">{children}</p>;
}

type LiqTab = "deposit" | "claim";

/** The Deposit/Claim underline sub-tabs (mirrors the Trade ticket's Buy/Sell). */
function LiquiditySubTabs({ value, onChange }: { value: LiqTab; onChange: (v: LiqTab) => void }) {
  const tabs: { value: LiqTab; label: string }[] = [
    { value: "deposit", label: "Deposit" },
    { value: "claim", label: "Claim" },
  ];
  return (
    <div role="tablist" aria-label="Liquidity action" className="flex gap-5 border-b border-hairline">
      {tabs.map((t) => {
        const active = t.value === value;
        return (
          <button
            key={t.value}
            role="tab"
            type="button"
            aria-selected={active}
            onClick={() => onChange(t.value)}
            className={`relative -mb-px pb-2 font-inter text-[14px] transition-colors ${
              active ? "font-medium text-platinum" : "text-silver hover:text-platinum"
            }`}
          >
            {t.label}
            <span
              aria-hidden
              className={`pointer-events-none absolute inset-x-0 -bottom-px h-0.5 rounded-full bg-aqua transition-opacity ${
                active ? "opacity-100" : "opacity-0"
              }`}
            />
          </button>
        );
      })}
    </div>
  );
}

/**
 * The Active-market liquidity surface as two tabs of one panel: Deposit (add into
 * the live AMM) and Claim (withdraw the pro-rata LP; self-gates until the market
 * settles and its fee is collected). Both controls render bare (no nested Card).
 */
function LiquidityActionTabs({ detail, refetch }: { detail: MarketDetail; refetch: () => void }) {
  const { pubkey, market, contributions, reserves } = detail;
  const [tab, setTab] = useState<LiqTab>("deposit");
  return (
    <div className="flex flex-col gap-4">
      <LiquiditySubTabs value={tab} onChange={setTab} />
      {tab === "deposit" ? (
        <AddLiquidityControl
          embedded
          pubkey={pubkey}
          market={market}
          reserves={reserves}
          onSuccess={refetch}
        />
      ) : (
        <ClaimLpControl
          embedded
          pubkey={pubkey}
          market={market}
          contributions={contributions}
          onSuccess={refetch}
        />
      )}
    </div>
  );
}

/**
 * The status-gated LIQUIDITY surface — deposit into / withdraw from THIS market's
 * pool, phase-routed so it's available across the market's whole life (this is
 * why the Liquidity tab is present for Active markets too, not just Funding):
 *
 *   - Funding   → ContributeForm (seed the funding floor) for a LONE market; a
 *                 GROUPED market (`isGrouped`) has no per-outcome contribute form
 *                 at all — funding only happens through the cumulative
 *                 GroupLiquidityPanel below, so there is exactly one funding
 *                 action + one progress bar for the whole group, not one per
 *                 outcome.
 *   - Active    → LiquidityActionTabs: Deposit (add into the live AMM) + Claim
 *                 (LP withdrawal; self-gates until settle) as two tabs of one panel.
 *   - Resolved / Void → ClaimLpControl (waits for fee collection before it opens).
 *   - Cancelled → RefundControl (reclaim staked KASS) until every contributor has
 *                 exited, after which there's nothing left to withdraw.
 *
 * The bulk cross-outcome GroupLiquidityPanel sits ABOVE this in the Liquidity tab.
 */
export function MarketLiquidityActions({
  detail,
  refetch,
  isGrouped,
}: {
  detail: MarketDetail;
  refetch: () => void;
  /** True when this market shares its oracle with sibling outcome markets — see
   *  {@link useOracleGroup}. Suppresses the per-outcome Funding contribute form. */
  isGrouped: boolean;
}) {
  const { pubkey, market, contributions } = detail;

  switch (market.status) {
    case MarketStatus.Funding:
      return isGrouped ? (
        <NoActions>Fund this option as part of the group below.</NoActions>
      ) : (
        <ContributeForm pubkey={pubkey} market={market} onSuccess={refetch} />
      );

    case MarketStatus.Active:
      // Active markets can BOTH take new liquidity (into the live AMM) and, once
      // settled, withdraw it — Deposit + Claim as two tabs of one panel.
      return <LiquidityActionTabs detail={detail} refetch={refetch} />;

    case MarketStatus.Resolved:
    case MarketStatus.Void:
      return (
        <ClaimLpControl
          pubkey={pubkey}
          market={market}
          contributions={contributions}
          onSuccess={refetch}
        />
      );

    case MarketStatus.Cancelled:
      return market.openContributions > 0 ? (
        <RefundControl pubkey={pubkey} market={market} onSuccess={refetch} />
      ) : (
        <NoActions>All contributors have been refunded — nothing left to withdraw.</NoActions>
      );

    default:
      return null;
  }
}

/**
 * The status-gated LIFECYCLE surface — the cranks that move the market between
 * phases (and the winner's redeem), phase-routed:
 *
 *   - Funding   → ActivateControl once the funding floor is met AND the oracle is
 *                 still live; CancelControl when the oracle is terminal AND the
 *                 market is still under floor (an under-funded market whose oracle
 *                 already resolved can only be cancelled → refunded).
 *   - Active    → ResolveControl once the oracle is terminal.
 *   - Resolved / Void → RedeemControl (redeem the winning conditional tokens) +
 *                 CollectFeeControl (the permissionless protocol-fee crank while a
 *                 non-zero fee is uncollected) + CloseMarketControl once the fee is
 *                 collected AND every contributor has exited.
 *   - Cancelled → CloseMarketControl once every contributor has been refunded
 *                 (reclaim the market's rent to the creator).
 */
export function MarketLifecycleActions({
  detail,
  refetch,
}: {
  detail: MarketDetail;
  refetch: () => void;
}) {
  const { pubkey, market, oracle } = detail;
  const oracleTerminal = oracle ? isTerminal(oracle.phase) : false;
  const config = useConfig();
  // The permissionless fee crank is available on a settled market that carries a
  // non-zero, uncollected protocol fee (needs the Config for the fee destination).
  const showCollectFee = config.data != null && market.feeBps > 0 && !market.feeCollected;

  switch (market.status) {
    case MarketStatus.Funding: {
      const { funded } = fundingProgress(market);
      const { canActivate, canCancel } = fundingActions(funded, oracleTerminal);
      if (!canActivate && !canCancel)
        return (
          <NoActions>
            Waiting on the funding floor — activation opens once the market is fully funded.
          </NoActions>
        );
      return (
        <div className="flex flex-col gap-6">
          {canActivate ? <ActivateControl pubkey={pubkey} market={market} onSuccess={refetch} /> : null}
          {canCancel ? <CancelControl pubkey={pubkey} market={market} onSuccess={refetch} /> : null}
        </div>
      );
    }

    case MarketStatus.Active:
      return oracleTerminal ? (
        <ResolveControl pubkey={pubkey} market={market} onSuccess={refetch} />
      ) : (
        <NoActions>
          The market resolves once its linked oracle reaches a terminal phase.
        </NoActions>
      );

    case MarketStatus.Resolved:
    case MarketStatus.Void: {
      // Activated markets close only once the fee is collected AND every
      // contributor has exited (openContributions === 0); the crank routes the
      // market's account rent back to the creator.
      const canClose = market.feeCollected && market.openContributions === 0;
      return (
        <div className="flex flex-col gap-6">
          <RedeemControl pubkey={pubkey} market={market} onSuccess={refetch} />
          {showCollectFee ? (
            <CollectFeeControl
              pubkey={pubkey}
              market={market}
              config={config.data!}
              onSuccess={refetch}
            />
          ) : null}
          {canClose ? (
            <CloseMarketControl pubkey={pubkey} market={market} onSuccess={refetch} />
          ) : market.openContributions > 0 ? (
            <ContributorsRemaining open={market.openContributions} />
          ) : null}
        </div>
      );
    }

    case MarketStatus.Cancelled: {
      // A never-activated market closes once every contributor has been refunded.
      const canClose = market.openContributions === 0;
      return canClose ? (
        <CloseMarketControl pubkey={pubkey} market={market} onSuccess={refetch} />
      ) : (
        <ContributorsRemaining open={market.openContributions} />
      );
    }

    default:
      return null;
  }
}

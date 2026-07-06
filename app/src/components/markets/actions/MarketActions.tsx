import { MarketStatus, isTerminal } from "@kassandra-market/sdk";
import type { MarketDetail } from "../../../market/data/markets";
import { fundingActions, fundingProgress } from "../../../market/lib/marketView";
import { useConfig } from "../../../market/hooks/useMarketDetail";
import { ContributeForm } from "./ContributeForm";
import { CancelControl } from "./CancelControl";
import { RefundControl } from "./RefundControl";
import { ActivateControl } from "./ActivateControl";
import { TradePanel } from "./TradePanel";
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
    <p className="font-inter text-[12px] text-driftwood">
      {open > 0
        ? `${open} contributor${open === 1 ? "" : "s"} yet to claim — the market can be closed once all have exited.`
        : "All contributions claimed — the market is ready to close."}
    </p>
  );
}

/**
 * The status-gated write surface under the read-only market panels. It routes on
 * `market.status`, rendering only the actions valid for the market's current
 * lifecycle phase:
 *
 *   - Funding   → ContributeForm; plus ActivateControl once the funding floor is
 *                 met AND the oracle is still live (activation needs a non-terminal
 *                 oracle); plus CancelControl when the oracle is terminal AND the
 *                 market is still under its floor (an under-funded market whose
 *                 oracle already resolved can only be cancelled → refunded).
 *   - Active    → TradePanel + ClaimLpControl; plus ResolveControl once the oracle
 *                 is terminal.
 *   - Resolved / Void → RedeemControl + CollectFeeControl (the permissionless
 *                 protocol-fee crank, shown while a non-zero fee is uncollected) +
 *                 ClaimLpControl (which waits for fee collection before it opens);
 *                 plus CloseMarketControl once the fee is collected AND every
 *                 contributor has exited (`openContributions === 0`).
 *   - Cancelled → RefundControl (reclaim staked KASS); once every contributor has
 *                 been refunded (`openContributions === 0`) it flips to
 *                 CloseMarketControl (reclaim the market's rent to the creator).
 *
 * Every action's `onSuccess` calls `refetch()` so the freshly-changed on-chain
 * state re-renders; the KASS-balance refetch is owned by the individual forms.
 */
export function MarketActions({
  detail,
  refetch,
}: {
  detail: MarketDetail;
  refetch: () => void;
}) {
  const { pubkey, market, oracle, reserves, contributions } = detail;
  const oracleTerminal = oracle ? isTerminal(oracle.phase) : false;
  const config = useConfig();
  // The permissionless fee crank is available on a settled market that carries a
  // non-zero, uncollected protocol fee (needs the Config for the fee destination).
  const showCollectFee =
    config.data != null && market.feeBps > 0 && !market.feeCollected;

  switch (market.status) {
    case MarketStatus.Funding: {
      const { funded } = fundingProgress(market);
      const { canActivate, canCancel } = fundingActions(funded, oracleTerminal);
      return (
        <div className="mt-6 flex flex-col gap-6">
          <ContributeForm pubkey={pubkey} market={market} onSuccess={refetch} />
          {canActivate ? <ActivateControl pubkey={pubkey} market={market} onSuccess={refetch} /> : null}
          {canCancel ? <CancelControl pubkey={pubkey} market={market} onSuccess={refetch} /> : null}
        </div>
      );
    }

    case MarketStatus.Active:
      return (
        <div className="mt-6 flex flex-col gap-6">
          <TradePanel pubkey={pubkey} market={market} reserves={reserves} onSuccess={refetch} />
          {oracleTerminal ? <ResolveControl pubkey={pubkey} market={market} onSuccess={refetch} /> : null}
          <ClaimLpControl
            pubkey={pubkey}
            market={market}
            contributions={contributions}
            onSuccess={refetch}
          />
        </div>
      );

    case MarketStatus.Resolved:
    case MarketStatus.Void: {
      // Activated markets close only once the fee is collected AND every
      // contributor has exited (openContributions === 0); the crank routes the
      // market's account rent back to the creator.
      const canClose = market.feeCollected && market.openContributions === 0;
      return (
        <div className="mt-6 flex flex-col gap-6">
          <RedeemControl pubkey={pubkey} market={market} onSuccess={refetch} />
          {showCollectFee ? (
            <CollectFeeControl
              pubkey={pubkey}
              market={market}
              config={config.data!}
              onSuccess={refetch}
            />
          ) : null}
          <ClaimLpControl
            pubkey={pubkey}
            market={market}
            contributions={contributions}
            onSuccess={refetch}
          />
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
      return (
        <div className="mt-6 flex flex-col gap-6">
          {canClose ? (
            <CloseMarketControl pubkey={pubkey} market={market} onSuccess={refetch} />
          ) : (
            <>
              <RefundControl pubkey={pubkey} market={market} onSuccess={refetch} />
              <ContributorsRemaining open={market.openContributions} />
            </>
          )}
        </div>
      );
    }

    default:
      return null;
  }
}

export default MarketActions;

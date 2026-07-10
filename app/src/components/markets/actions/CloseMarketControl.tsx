import type { Market } from "@kassandra-market/markets";
import { Card } from "../../ui";
import { buildCloseMarketIxs } from "../../../market/data/actions";
import { useWriteAction } from "../../../market/hooks/useWriteAction";
import { ConnectGate } from "./ConnectGate";
import { SubmitButton } from "./formPrimitives";
import { WriteStatusRegion } from "./WriteStatusRegion";

/**
 * Permissionless "Close market & reclaim rent" crank, shown by {@link MarketActions}
 * on a fully-settled market — status terminal (Resolved/Void/Cancelled), the
 * protocol fee collected (activated markets only), AND `openContributions === 0`
 * (every contributor has claimed/refunded, so nothing is stranded). The gate on
 * `openContributions` lives in {@link MarketActions}; by the time this renders the
 * market is reap-ready.
 *
 * It SPL-`CloseAccount`s the Market-PDA-owned token accounts and closes the
 * `Market` PDA, routing ALL reclaimed account rent back to the creator (the
 * original payer). Anyone may crank it — the caller only pays the fee.
 *
 * After a successful close the market account is GONE; `onSuccess` → the detail
 * refetch then 404s and the page falls back to its "market not found" state.
 */
export function CloseMarketControl({
  pubkey,
  market,
  onSuccess,
}: {
  pubkey: string;
  market: Market;
  onSuccess: () => void;
}) {
  const action = useWriteAction(onSuccess);

  const onSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    void action.run(() => buildCloseMarketIxs({ market: pubkey, creator: market.creator }));
  };

  return (
    <Card className="flex flex-col gap-4">
      <div>
        <h3 className="font-serif text-subheading font-light text-sepia">Close market &amp; reclaim rent</h3>
        <p className="mt-1 font-inter text-[13px] text-driftwood">
          Every contributor has exited this settled market, so its program accounts can be reaped.
          Closing returns all of the market's account rent to the creator and removes the market from
          the chain. Permissionless — anyone may crank it.
        </p>
      </div>
      <ConnectGate connected={action.connected}>
        <form className="flex flex-col gap-3" onSubmit={onSubmit} noValidate>
          <div className="flex items-center gap-3">
            <SubmitButton verb="Close market" status={action.status} />
          </div>
          <WriteStatusRegion status={action.status} successVerb="Market closed" />
        </form>
      </ConnectGate>
    </Card>
  );
}

export default CloseMarketControl;

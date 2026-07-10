import type { Market } from "@kassandra-market/markets";
import { Card } from "../../ui";
import { buildRefundIxs } from "../../../market/data/actions";
import { useWriteAction } from "../../../market/hooks/useWriteAction";
import { ConnectGate } from "./ConnectGate";
import { SubmitButton } from "./formPrimitives";
import { WriteStatusRegion } from "./WriteStatusRegion";

/**
 * Permissionless per-contributor refund from a Cancelled market — returns the
 * connected wallet's stake to its KASS ATA (created idempotently if absent). The
 * program is the guard for "did this wallet actually contribute"; the form just
 * builds the ix for the connected authority.
 */
export function RefundControl({
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
    void action.run(() =>
      buildRefundIxs({
        indexer: action.indexer,
        market: pubkey,
        kassMint: market.kassMint,
        contributor: action.address!,
      }),
    );
  };

  return (
    <Card className="flex flex-col gap-4">
      <div>
        <h3 className="font-serif text-subheading font-light text-sepia">Refund contribution</h3>
        <p className="mt-1 font-inter text-[13px] text-driftwood">
          This market was cancelled. Reclaim your staked KASS.
        </p>
      </div>
      <ConnectGate connected={action.connected}>
        <form className="flex flex-col gap-3" onSubmit={onSubmit} noValidate>
          <div className="flex items-center gap-3">
            <SubmitButton verb="Refund my stake" status={action.status} />
          </div>
          <WriteStatusRegion status={action.status} successVerb="Refunded" />
        </form>
      </ConnectGate>
    </Card>
  );
}

export default RefundControl;

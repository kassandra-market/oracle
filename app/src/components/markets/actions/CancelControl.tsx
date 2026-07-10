import type { Market } from "@kassandra-market/markets";
import { Card } from "../../ui";
import { buildCancelIxs } from "../../../market/data/actions";
import { useWriteAction } from "../../../market/hooks/useWriteAction";
import { ConnectGate } from "./ConnectGate";
import { SubmitButton } from "./formPrimitives";
import { WriteStatusRegion } from "./WriteStatusRegion";

/**
 * Permissionless crank: mark an under-funded Funding market Cancelled once its
 * oracle is terminal. Anyone may call it; the caller only pays the fee. Shown by
 * {@link MarketActions} only when the gating conditions hold.
 */
export function CancelControl({
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
    void action.run(() => buildCancelIxs({ market: pubkey, oracle: market.oracle }));
  };

  return (
    <Card className="flex flex-col gap-4">
      <div>
        <h3 className="font-serif text-subheading font-light text-sepia">Cancel market</h3>
        <p className="mt-1 font-inter text-[13px] text-driftwood">
          The oracle resolved before this market reached its funding floor, so contributions can be
          refunded. Anyone may cancel it.
        </p>
      </div>
      <ConnectGate connected={action.connected}>
        <form className="flex flex-col gap-3" onSubmit={onSubmit} noValidate>
          <div className="flex items-center gap-3">
            <SubmitButton verb="Cancel market" status={action.status} />
          </div>
          <WriteStatusRegion status={action.status} successVerb="Cancelled" />
        </form>
      </ConnectGate>
    </Card>
  );
}

export default CancelControl;

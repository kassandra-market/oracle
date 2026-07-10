import type { Config, Market } from "@kassandra-market/markets";
import { Card } from "../../ui";
import { buildCollectFeeIxs } from "../../../market/data/actions";
import { useWriteAction } from "../../../market/hooks/useWriteAction";
import { ConnectGate } from "./ConnectGate";
import { SubmitButton } from "./formPrimitives";
import { WriteStatusRegion } from "./WriteStatusRegion";

/**
 * Permissionless "collect protocol fee" crank, shown by {@link MarketActions} on a
 * Resolved/Void market that carries a non-zero `feeBps` and has NOT yet been
 * collected. It cuts the protocol's share of the market's accrued LP earnings and
 * routes it — denominated in KASS — to the futarchy-governed fee destination, then
 * opens `claim_lp` (which is gated on collection). Anyone may crank it.
 */
export function CollectFeeControl({
  pubkey,
  market,
  config,
  onSuccess,
}: {
  pubkey: string;
  market: Market;
  config: Config;
  onSuccess: () => void;
}) {
  const action = useWriteAction(onSuccess);

  const onSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    void action.run(() =>
      buildCollectFeeIxs({ market: pubkey, marketAccount: market, config }),
    );
  };

  return (
    <Card className="flex flex-col gap-4">
      <div>
        <h3 className="font-serif text-subheading font-light text-sepia">Collect protocol fee</h3>
        <p className="mt-1 font-inter text-[13px] text-driftwood">
          This market accrued a {(market.feeBps / 100).toFixed(2)}% protocol fee on its LP earnings.
          Collecting routes that accrued cut to the KASS futarchy and unlocks LP claims (claims are
          held until the fee is collected). Permissionless — anyone may crank it.
        </p>
      </div>
      <ConnectGate connected={action.connected}>
        <form className="flex flex-col gap-3" onSubmit={onSubmit} noValidate>
          <div className="flex items-center gap-3">
            <SubmitButton verb="Collect protocol fee" status={action.status} />
          </div>
          <WriteStatusRegion status={action.status} successVerb="Fee collected" />
        </form>
      </ConnectGate>
    </Card>
  );
}

export default CollectFeeControl;

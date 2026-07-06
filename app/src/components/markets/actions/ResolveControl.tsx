import type { Market } from "@kassandra-market/sdk";
import { Card } from "../../ui";
import { buildResolveIxs } from "../../../market/data/actions";
import { useWriteAction } from "../../../market/hooks/useWriteAction";
import { ConnectGate } from "./ConnectGate";
import { SubmitButton } from "./formPrimitives";
import { WriteStatusRegion } from "./WriteStatusRegion";

/**
 * Permissionless "resolve" crank, shown by {@link MarketActions} on an Active
 * market whose linked oracle has reached a terminal phase. It bridges the oracle
 * result into the market's MetaDAO question (stamping the payout numerators
 * redeem reads) and flips the market to Resolved/Void. Anyone may crank it.
 */
export function ResolveControl({
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
      buildResolveIxs({ market: pubkey, oracle: market.oracle, question: market.question }),
    );
  };

  return (
    <Card className="flex flex-col gap-4">
      <div>
        <h3 className="font-serif text-subheading font-light text-sepia">Resolve market</h3>
        <p className="mt-1 font-inter text-[13px] text-driftwood">
          The oracle has reached a terminal outcome. Resolving settles the market's question so
          winning positions become redeemable. Permissionless — anyone may crank it.
        </p>
      </div>
      <ConnectGate connected={action.connected}>
        <form className="flex flex-col gap-3" onSubmit={onSubmit} noValidate>
          <div className="flex items-center gap-3">
            <SubmitButton verb="Resolve market" status={action.status} />
          </div>
          <WriteStatusRegion status={action.status} successVerb="Resolved" />
        </form>
      </ConnectGate>
    </Card>
  );
}

export default ResolveControl;

import type { Market } from "@kassandra-market/markets";
import { Card } from "../../ui";
import { buildRedeemIxs, marketRefs } from "../../../market/data/actions";
import { useWriteAction } from "../../../market/hooks/useWriteAction";
import { useKassBalance } from "../../../market/hooks/useKassBalance";
import { formatKass } from "../../../market/lib/marketView";
import { ConnectGate } from "./ConnectGate";
import { SubmitButton } from "./formPrimitives";
import { WriteStatusRegion } from "./WriteStatusRegion";

/**
 * Redeem a holder's resolved payout, shown by {@link MarketActions} on a
 * Resolved/Void market. It burns the wallet's full cYES + cNO balances and pays
 * the resolved KASS (winning legs pay 1:1, worthless legs pay 0). Shows the
 * holder's current cYES/cNO balances and gates the submit on holding some.
 */
export function RedeemControl({
  pubkey,
  market,
  onSuccess,
}: {
  pubkey: string;
  market: Market;
  onSuccess: () => void;
}) {
  const yes = useKassBalance(market.yesMint.toString());
  const no = useKassBalance(market.noMint.toString());
  const action = useWriteAction(() => {
    yes.refetch();
    no.refetch();
    onSuccess();
  });

  const nothing = yes.balance === 0n && no.balance === 0n;

  const onSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    void action.run(async () => {
      const refs = await marketRefs(pubkey, market);
      return buildRedeemIxs({ indexer: action.indexer, refs, user: action.address! });
    });
  };

  return (
    <Card className="flex flex-col gap-4">
      <div>
        <h3 className="font-serif text-subheading font-light text-sepia">Redeem payout</h3>
        <p className="mt-1 font-inter text-[13px] text-driftwood">
          The market is settled. Redeem burns your conditional tokens for the resolved KASS payout
          (winning shares pay out, worthless shares pay nothing).
        </p>
      </div>
      <ConnectGate connected={action.connected}>
        <form className="flex flex-col gap-3" onSubmit={onSubmit} noValidate>
          <dl className="flex flex-wrap gap-x-6 gap-y-1 font-inter text-[12px] text-driftwood">
            <div className="flex gap-1">
              <dt>Your YES</dt>
              <dd className="text-bronze">{yes.balance === null ? "—" : formatKass(yes.balance)}</dd>
            </div>
            <div className="flex gap-1">
              <dt>Your NO</dt>
              <dd className="text-bronze">{no.balance === null ? "—" : formatKass(no.balance)}</dd>
            </div>
          </dl>
          <div className="flex items-center gap-3">
            <SubmitButton verb="Redeem" status={action.status} disabled={nothing} />
          </div>
          {nothing ? (
            <p className="font-inter text-[12px] text-stone">No redeemable positions in this wallet.</p>
          ) : null}
          <WriteStatusRegion status={action.status} successVerb="Redeemed" />
        </form>
      </ConnectGate>
    </Card>
  );
}

export default RedeemControl;

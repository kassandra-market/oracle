import type { Market, Contribution } from "@kassandra-market/sdk";
import { Card } from "../../ui";
import { buildClaimLpIxs } from "../../../market/data/actions";
import { useWriteAction } from "../../../market/hooks/useWriteAction";
import { formatKass } from "../../../market/lib/marketView";
import { SubmitButton } from "./formPrimitives";
import { WriteStatusRegion } from "./WriteStatusRegion";

/**
 * A contributor's pro-rata LP claim, shown by {@link MarketActions} on an
 * Active/Resolved/Void market. It self-hides unless the CONNECTED wallet has a
 * still-open (unclaimed) contribution to this market — permissionless, but only
 * the contributor's own stake is claimable. On success the detail refetch flips
 * the contribution to "claimed" and this control disappears.
 *
 * `claim_lp` is gated on protocol-fee collection: it opens only once
 * `market.feeCollected` is set (resolve → collect_fee → claim_lp). Until then this
 * shows a disabled "waiting for fee collection" state rather than an enabled claim,
 * so a contributor isn't handed a button the program would reject.
 */
export function ClaimLpControl({
  pubkey,
  market,
  contributions,
  onSuccess,
}: {
  pubkey: string;
  market: Market;
  contributions: { pubkey: string; contribution: Contribution }[];
  onSuccess: () => void;
}) {
  const action = useWriteAction(onSuccess);

  const mine =
    action.address == null
      ? undefined
      : contributions.find(
          ({ contribution }) => !contribution.claimed && contribution.contributor.toString() === action.address,
        );

  // Nothing to claim (disconnected, not a contributor, or already claimed) → hide.
  if (!mine) return null;

  // Fee gate: claim_lp opens only after the protocol fee is collected. Until then
  // (Active, or Resolved/Void awaiting the crank) show a disabled waiting state.
  if (!market.feeCollected) {
    return (
      <Card className="flex flex-col gap-4">
        <div>
          <h3 className="font-serif text-subheading font-light text-sepia">Claim LP tokens</h3>
          <p className="mt-1 font-inter text-[13px] text-driftwood">
            Your {formatKass(mine.contribution.amount)} KASS contribution earned a pro-rata share of
            the pool's LP tokens. Claims open once the market resolves and its protocol fee is
            collected.
          </p>
        </div>
        <SubmitButton verb="Waiting for fee collection" status={{ kind: "idle" }} disabled />
      </Card>
    );
  }

  const onSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    void action.run(() =>
      buildClaimLpIxs({
        indexer: action.indexer,
        market: pubkey,
        contributor: action.address!,
        lpMint: market.lpMint,
      }),
    );
  };

  return (
    <Card className="flex flex-col gap-4">
      <div>
        <h3 className="font-serif text-subheading font-light text-sepia">Claim LP tokens</h3>
        <p className="mt-1 font-inter text-[13px] text-driftwood">
          Your {formatKass(mine.contribution.amount)} KASS contribution earned a pro-rata share of
          the pool's LP tokens at activation. Claim them to your wallet.
        </p>
      </div>
      <form className="flex flex-col gap-3" onSubmit={onSubmit} noValidate>
        <div className="flex items-center gap-3">
          <SubmitButton verb="Claim LP" status={action.status} />
        </div>
        <WriteStatusRegion status={action.status} successVerb="LP claimed" />
      </form>
    </Card>
  );
}

export default ClaimLpControl;

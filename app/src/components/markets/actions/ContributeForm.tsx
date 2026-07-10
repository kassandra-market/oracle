import { useState, type FormEvent } from "react";
import type { Market } from "@kassandra-market/markets";
import { Card } from "../../ui";
import { buildContributeIxs } from "../../../market/data/actions";
import { useWriteAction } from "../../../market/hooks/useWriteAction";
import { useKassBalance } from "../../../market/hooks/useKassBalance";
import { formatKass } from "../../../market/lib/marketView";
import { parseKassAmount, kassBalanceGateError } from "../../../market/data/amount";
import { ConnectGate } from "./ConnectGate";
import { Field, KassBalanceLine, SubmitButton, TextInput } from "./formPrimitives";
import { WriteStatusRegion } from "./WriteStatusRegion";

/**
 * Add KASS to a Funding market's escrow (create-or-increment the caller's
 * Contribution). Shows the connected wallet's KASS balance and gates the submit
 * on it (additively — a `null` balance never blocks; the tx is the guard).
 */
export function ContributeForm({
  pubkey,
  market,
  onSuccess,
}: {
  pubkey: string;
  market: Market;
  onSuccess: () => void;
}) {
  const kassMint = market.kassMint.toString();
  const { balance, loading: balanceLoading, refetch: refetchBalance } = useKassBalance(kassMint);
  const action = useWriteAction(() => {
    refetchBalance();
    onSuccess();
  });

  const [amount, setAmount] = useState("");
  const [amountError, setAmountError] = useState<string | undefined>();
  const balanceError = kassBalanceGateError(parseKassAmount(amount).value, balance);

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    const parsed = parseKassAmount(amount);
    if (parsed.error) {
      setAmountError(parsed.error);
      return;
    }
    setAmountError(undefined);
    void action.run(() =>
      buildContributeIxs({
        indexer: action.indexer,
        market: pubkey,
        kassMint,
        contributor: action.address!,
        amount: parsed.value!,
      }),
    );
  };

  return (
    <Card className="flex flex-col gap-4">
      <div>
        <h3 className="font-serif text-subheading font-light text-sepia">Contribute funding</h3>
        <p className="mt-1 font-inter text-[13px] text-driftwood">
          Stake KASS toward this market's funding floor. Refundable if it's cancelled.
        </p>
      </div>
      <ConnectGate connected={action.connected}>
        <form className="flex flex-col gap-4" onSubmit={onSubmit} noValidate>
          <Field label="Amount (KASS)" error={amountError ?? balanceError}>
            {(ids) => (
              <TextInput
                ids={ids}
                inputMode="decimal"
                placeholder="e.g. 250"
                value={amount}
                onChange={(e) => setAmount(e.target.value)}
              />
            )}
          </Field>
          <KassBalanceLine balance={balance} loading={balanceLoading} format={formatKass} />
          <div className="flex items-center gap-3">
            <SubmitButton verb="Contribute" status={action.status} disabled={Boolean(balanceError)} />
          </div>
          <WriteStatusRegion status={action.status} successVerb="Contributed" />
        </form>
      </ConnectGate>
    </Card>
  );
}

export default ContributeForm;

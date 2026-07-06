import { useState, type FormEvent } from "react";
import { pda, type Market } from "@kassandra-market/sdk";
import { Card } from "../../ui";
import {
  buildBuyIxs,
  buildSellIxs,
  marketRefs,
  previewBuy,
  DEFAULT_SLIPPAGE_BPS,
  type Outcome,
} from "../../../market/data/actions";
import type { AmmReserves } from "../../../market/data/markets";
import { useWriteAction } from "../../../market/hooks/useWriteAction";
import { useKassBalance } from "../../../market/hooks/useKassBalance";
import { formatKass, formatProbability, impliedYesProbability } from "../../../market/lib/marketView";
import { parseKassAmount, kassBalanceGateError } from "../../../market/data/amount";
import { ConnectGate } from "./ConnectGate";
import { Field, KassBalanceLine, SubmitButton, TextInput } from "./formPrimitives";
import { WriteStatusRegion } from "./WriteStatusRegion";

type Mode = "buy" | "sell";

/** A pill toggle: one selected option out of a small labelled set. */
function Toggle<T extends string>({
  value,
  options,
  onChange,
  ariaLabel,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
  ariaLabel: string;
}) {
  return (
    <div role="group" aria-label={ariaLabel} className="inline-flex rounded-button border border-pebble p-0.5">
      {options.map((o) => {
        const active = o.value === value;
        return (
          <button
            key={o.value}
            type="button"
            aria-pressed={active}
            onClick={() => onChange(o.value)}
            className={`rounded-[10px] px-3 py-1.5 font-inter text-[13px] transition-colors ${
              active ? "bg-chestnut text-liquid-abyss" : "text-sepia hover:bg-pebble/50"
            }`}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}

/**
 * The Active-market trade surface: a YES/NO outcome toggle, a Buy/Sell mode, a
 * KASS (buy) / position (sell) amount, a live implied-probability readout from
 * the polled cYES/cNO reserves, and a buy/sell submit. Buy splits KASS into a
 * cYES+cNO pair and swaps the unwanted leg; sell unwinds a held leg back to KASS
 * (amounts computed from the live reserves). A disabled "pay with any token"
 * toggle marks the deferred Jupiter any-token entry.
 */
export function TradePanel({
  pubkey,
  market,
  reserves,
  onSuccess,
}: {
  pubkey: string;
  market: Market;
  reserves: AmmReserves | null;
  onSuccess: () => void;
}) {
  const kassMint = market.kassMint.toString();
  const yesMint = market.yesMint.toString();
  const noMint = market.noMint.toString();

  const kass = useKassBalance(kassMint);
  const yes = useKassBalance(yesMint);
  const no = useKassBalance(noMint);

  const action = useWriteAction(() => {
    kass.refetch();
    yes.refetch();
    no.refetch();
    onSuccess();
  });

  const [mode, setMode] = useState<Mode>("buy");
  const [outcome, setOutcome] = useState<Outcome>("yes");
  const [amount, setAmount] = useState("");
  const [amountError, setAmountError] = useState<string | undefined>();

  const parsed = parseKassAmount(amount);
  const yesProb = impliedYesProbability(reserves);
  const outcomeProb = outcome === "yes" ? yesProb : yesProb === null ? null : 1 - yesProb;

  const positionBalance = outcome === "yes" ? yes.balance : no.balance;
  // Buy gates on KASS; sell gates on the held leg (both raw base units, 9 dp).
  const gateBalance = mode === "buy" ? kass.balance : positionBalance;
  const balanceError = kassBalanceGateError(parsed.value, gateBalance);

  const buyReceived =
    mode === "buy" && parsed.value ? previewBuy(reserves, outcome, parsed.value).received : null;

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (parsed.error) {
      setAmountError(parsed.error);
      return;
    }
    setAmountError(undefined);
    const value = parsed.value!;
    void action.run(async () => {
      const refs = await marketRefs(pubkey, market);
      const userKassAta = (await pda.associatedTokenAccount(action.address!, market.kassMint)).address;
      if (mode === "buy") {
        return buildBuyIxs({
          indexer: action.indexer,
          refs,
          user: action.address!,
          outcome,
          kassAmount: value,
          userKassAta,
          reserves,
          slippageBps: DEFAULT_SLIPPAGE_BPS,
        });
      }
      return buildSellIxs({
        indexer: action.indexer,
        refs,
        user: action.address!,
        outcome,
        positionAmount: value,
        userKassAta,
        reserves,
        slippageBps: DEFAULT_SLIPPAGE_BPS,
      });
    });
  };

  const amountLabel = mode === "buy" ? "Amount (KASS to spend)" : `Amount (${outcome.toUpperCase()} shares to sell)`;

  return (
    <Card className="flex flex-col gap-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h3 className="font-serif text-subheading font-light text-sepia">Trade</h3>
          <p className="mt-1 font-inter text-[13px] text-driftwood">
            Take or close a net YES/NO position against the cYES/cNO pool.
          </p>
        </div>
        <div className="text-right">
          <p className="font-inter text-[12px] text-driftwood">Implied {outcome.toUpperCase()}</p>
          <p className="font-serif text-heading-sm font-light text-sepia">{formatProbability(outcomeProb)}</p>
        </div>
      </div>

      <ConnectGate connected={action.connected}>
        <form className="flex flex-col gap-4" onSubmit={onSubmit} noValidate>
          <div className="flex flex-wrap items-center gap-3">
            <Toggle<Mode>
              ariaLabel="Buy or sell"
              value={mode}
              onChange={setMode}
              options={[
                { value: "buy", label: "Buy" },
                { value: "sell", label: "Sell" },
              ]}
            />
            <Toggle<Outcome>
              ariaLabel="Outcome"
              value={outcome}
              onChange={setOutcome}
              options={[
                { value: "yes", label: "YES" },
                { value: "no", label: "NO" },
              ]}
            />
          </div>

          <Field label={amountLabel} error={amountError ?? balanceError}>
            {(ids) => (
              <TextInput
                ids={ids}
                inputMode="decimal"
                placeholder="e.g. 100"
                value={amount}
                onChange={(e) => setAmount(e.target.value)}
              />
            )}
          </Field>

          {mode === "buy" ? (
            <>
              <KassBalanceLine balance={kass.balance} loading={kass.loading} format={formatKass} />
              {buyReceived !== null ? (
                <p className="-mt-1 font-inter text-[12px] text-driftwood">
                  You receive ≈{" "}
                  <span className="text-bronze">
                    {formatKass(buyReceived)} {outcome.toUpperCase()}
                  </span>{" "}
                  shares
                </p>
              ) : null}
            </>
          ) : (
            <p className="-mt-1 font-inter text-[12px] text-driftwood">
              Your {outcome.toUpperCase()} shares:{" "}
              <span className="text-bronze">
                {positionBalance === null ? "—" : formatKass(positionBalance)}
              </span>
            </p>
          )}

          {/* Jupiter any-token entry: DISABLED (deferred). */}
          {/* TODO wire buildJupiterEntryRequest + app fetch (GET /quote → POST /swap) + composeWithEntry. */}
          <label
            className="flex cursor-not-allowed items-center gap-2 font-inter text-[12px] text-stone"
            title="Coming soon — pay with USDC/SOL via Jupiter"
          >
            <input type="checkbox" disabled className="cursor-not-allowed" />
            Pay with any token (Jupiter) — coming soon
          </label>

          <div className="flex items-center gap-3">
            <SubmitButton
              verb={mode === "buy" ? `Buy ${outcome.toUpperCase()}` : `Sell ${outcome.toUpperCase()}`}
              status={action.status}
              disabled={Boolean(balanceError)}
            />
          </div>
          <WriteStatusRegion status={action.status} successVerb={mode === "buy" ? "Bought" : "Sold"} />
        </form>
      </ConnectGate>
    </Card>
  );
}

export default TradePanel;

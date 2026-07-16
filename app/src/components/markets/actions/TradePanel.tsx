import { useId, useState, type FormEvent } from "react";
import { pda, type Market } from "@kassandra-market/markets";
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
import {
  KASS_DECIMALS,
  formatKass,
  impliedYesProbability,
} from "../../../market/lib/marketView";
import { parseKassAmount, balanceGateError } from "../../../market/data/amount";
import { useKassUsdcPrice } from "../../../hooks/useKassUsdcPrice";
import { PriceChart } from "../PriceChart";
import { ConnectGate } from "./ConnectGate";
import { SubmitButton } from "./formPrimitives";
import { WriteStatusRegion } from "./WriteStatusRegion";

type Mode = "buy" | "sell";

/** The unit a share PRICE is displayed in. A share pays 1 KASS at resolution, so
 *  its price in KASS equals the implied probability; USD applies the KASS→USDC
 *  TWAP (unavailable → the USD unit is disabled). */
type Unit = "%" | "KASS" | "USD";
const UNITS: Unit[] = ["%", "KASS", "USD"];

/**
 * Format an implied probability `0..1` as a share price in the chosen unit:
 *   - `%`    → the probability (58%);
 *   - `KASS` → the KASS price of one share (≈ the probability, since it pays 1 KASS);
 *   - `USD`  → that KASS price × the KASS→USDC price (`null` price → em dash).
 */
function formatSharePrice(prob: number | null, unit: Unit, kassUsd: number | null): string {
  if (prob === null) return "—";
  if (unit === "%") return `${Math.round(prob * 100)}%`;
  if (unit === "KASS") return `${prob.toFixed(2)} KASS`;
  if (kassUsd === null) return "—";
  return `$${(prob * kassUsd).toFixed(2)}`;
}

/** Whole-KASS quick-add chips (mirrors the reference's +$1/+$5/… stepper). */
const PRESETS = [10, 50, 100] as const;

/** A base-unit KASS balance → a plain, comma-free decimal string the amount input
 *  (and {@link parseKassAmount}) accepts. Trailing-zero trimmed. */
function toPlainAmount(base: bigint): string {
  const s = base.toString().padStart(KASS_DECIMALS + 1, "0");
  const whole = s.slice(0, s.length - KASS_DECIMALS);
  const frac = s.slice(s.length - KASS_DECIMALS).replace(/0+$/, "");
  return frac ? `${whole}.${frac}` : whole;
}

/** The Buy/Sell underline tabs (Acheter/Vendre in the reference). */
function ModeTabs({ value, onChange }: { value: Mode; onChange: (m: Mode) => void }) {
  const tabs: { value: Mode; label: string }[] = [
    { value: "buy", label: "Buy" },
    { value: "sell", label: "Sell" },
  ];
  return (
    <div role="tablist" aria-label="Buy or sell" className="flex gap-5">
      {tabs.map((t) => {
        const active = t.value === value;
        return (
          <button
            key={t.value}
            role="tab"
            type="button"
            aria-selected={active}
            onClick={() => onChange(t.value)}
            className={`relative -mb-px pb-2 font-inter text-[15px] transition-colors ${
              active ? "font-medium text-sepia" : "text-driftwood hover:text-sepia"
            }`}
          >
            {t.label}
            <span
              aria-hidden
              className={`pointer-events-none absolute inset-x-0 -bottom-px h-0.5 rounded-full bg-chestnut transition-opacity ${
                active ? "opacity-100" : "opacity-0"
              }`}
            />
          </button>
        );
      })}
    </div>
  );
}

/** The price-unit switcher (% / KASS / USD). USD is disabled — greyed, with an
 *  explanatory tooltip — when no KASS→USDC price is available. */
function UnitTabs({
  value,
  onChange,
  usdAvailable,
}: {
  value: Unit;
  onChange: (u: Unit) => void;
  usdAvailable: boolean;
}) {
  return (
    <div
      role="group"
      aria-label="Price unit"
      className="inline-flex rounded-button border border-pebble p-0.5"
    >
      {UNITS.map((u) => {
        const disabled = u === "USD" && !usdAvailable;
        const selected = u === value;
        return (
          <button
            key={u}
            type="button"
            aria-pressed={selected}
            disabled={disabled}
            title={disabled ? "No KASS/USDC price feed on this cluster yet" : undefined}
            onClick={() => onChange(u)}
            className={`rounded-[10px] px-2.5 py-1 font-inter text-[12px] transition-colors ${
              selected
                ? "bg-chestnut text-liquid-abyss"
                : disabled
                  ? "cursor-not-allowed text-stone/50"
                  : "text-sepia hover:bg-pebble/50"
            }`}
          >
            {u}
          </button>
        );
      })}
    </div>
  );
}

/**
 * A big YES/NO outcome-price button (the reference's "Oui 58,0¢" / "Non 42,1¢"):
 * doubles as the outcome selector and the live implied-price readout. Selected
 * fills its tone (YES aqua-chestnut, NO ember); unselected is a quiet outline.
 */
function OutcomeButton({
  outcome,
  selected,
  probability,
  unit,
  kassUsd,
  onSelect,
}: {
  outcome: Outcome;
  selected: boolean;
  probability: number | null;
  unit: Unit;
  kassUsd: number | null;
  onSelect: () => void;
}) {
  const isYes = outcome === "yes";
  const label = isYes ? "YES" : "NO";
  const selectedClass = isYes
    ? "border-chestnut bg-chestnut text-liquid-abyss"
    : "border-ember-orange bg-ember-orange text-liquid-abyss";
  const idleClass = isYes
    ? "border-pebble bg-soft-cream text-chestnut hover:border-chestnut/50"
    : "border-pebble bg-soft-cream text-ember-orange hover:border-ember-orange/50";
  return (
    <button
      type="button"
      aria-pressed={selected}
      onClick={onSelect}
      className={`flex items-baseline justify-between gap-2 rounded-tag border px-4 py-3 transition-colors active:scale-[0.98] ${
        selected ? selectedClass : idleClass
      }`}
    >
      <span className="font-inter text-[14px] font-medium">{label}</span>
      <span className="whitespace-nowrap font-serif text-[17px] font-light tabular-nums">
        {formatSharePrice(probability, unit, kassUsd)}
      </span>
    </button>
  );
}

/**
 * The Active-market trade surface, laid out like a prediction-market order ticket:
 * a wide price-history panel (the YES + NO implied-probability curves, with their
 * live readouts) beside a floating order card — Buy/Sell tabs, big YES/NO price
 * toggles, a large amount field with quick-add chips, a live "you receive"
 * estimate and the trade CTA. Buy splits KASS into a cYES+cNO pair and swaps the
 * unwanted leg; sell unwinds a held leg back to KASS (amounts from live reserves).
 */
export function TradePanel({
  pubkey,
  market,
  reserves,
  onSuccess,
  question,
  boundLabel,
}: {
  pubkey: string;
  market: Market;
  reserves: AmmReserves | null;
  onSuccess: () => void;
  /** The oracle question (header context; falls back to a generic label). */
  question?: string;
  /** The outcome this market pays YES on, in words (header context). */
  boundLabel?: string | null;
}) {
  const kassMint = market.kassMint.toString();
  const yesMint = market.yesMint.toString();
  const noMint = market.noMint.toString();

  const kass = useKassBalance(kassMint);
  const yes = useKassBalance(yesMint);
  const no = useKassBalance(noMint);

  // Bumped on every confirmed trade so the price chart reloads its series at
  // once (rather than waiting for its poll), keeping the curve in sync with the
  // freshly-refetched YES/NO readouts.
  const [tradeNonce, setTradeNonce] = useState(0);

  const action = useWriteAction(() => {
    kass.refetch();
    yes.refetch();
    no.refetch();
    setTradeNonce((n) => n + 1);
    onSuccess();
  });

  const [mode, setMode] = useState<Mode>("buy");
  const [outcome, setOutcome] = useState<Outcome>("yes");
  const [amount, setAmount] = useState("");
  const [amountError, setAmountError] = useState<string | undefined>();
  const [unit, setUnit] = useState<Unit>("%");

  // KASS→USD price (governance-anchored futarchy spot TWAP); null → USD disabled.
  const kassUsd = useKassUsdcPrice();
  const usdAvailable = kassUsd !== null;
  // Never render USD when it isn't available (e.g. the feed drops after selection).
  const displayUnit: Unit = unit === "USD" && !usdAvailable ? "%" : unit;

  const amountId = useId();
  const descId = `${amountId}-desc`;

  const parsed = parseKassAmount(amount);
  const yesProb = impliedYesProbability(reserves);
  const noProb = yesProb === null ? null : 1 - yesProb;

  const positionBalance = outcome === "yes" ? yes.balance : no.balance;
  // Buy gates on KASS; sell gates on the held outcome shares (both 9 dp). The
  // gate message names the asset it checks, so selling asks for shares, not KASS.
  const gateBalance = mode === "buy" ? kass.balance : positionBalance;
  const gateAsset = mode === "buy" ? "KASS" : `${outcome.toUpperCase()} shares`;
  const balanceError = balanceGateError(parsed.value, gateBalance, gateAsset);

  const buyReceived =
    mode === "buy" && parsed.value ? previewBuy(reserves, outcome, parsed.value).received : null;

  function bump(n: number) {
    const cur = Number(amount);
    setAmount(String((Number.isFinite(cur) ? cur : 0) + n));
    setAmountError(undefined);
  }
  function setMax() {
    if (gateBalance != null && gateBalance > 0n) setAmount(toPlainAmount(gateBalance));
    setAmountError(undefined);
  }

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

  const inputError = amountError ?? balanceError;

  return (
    <div className="grid grid-cols-1 gap-6 lg:grid-cols-5">
      {/* Price panel — the wide chart with the live YES/NO readouts. */}
      <Card className="flex flex-col gap-4 lg:col-span-3">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h3 className="font-serif text-subheading font-light text-sepia">Price history</h3>
            <p className="mt-1 font-inter text-[13px] text-driftwood">
              {displayUnit === "%"
                ? "Implied probability · YES vs NO"
                : `Share price (${displayUnit}) · YES vs NO`}
            </p>
          </div>
          <div className="flex gap-5 text-right">
            <div>
              <p className="font-inter text-[11px] uppercase tracking-[0.06em] text-driftwood">YES</p>
              <p className="font-serif text-heading-sm font-light tabular-nums text-chestnut">
                {formatSharePrice(yesProb, displayUnit, kassUsd)}
              </p>
            </div>
            <div>
              <p className="font-inter text-[11px] uppercase tracking-[0.06em] text-driftwood">NO</p>
              <p className="font-serif text-heading-sm font-light tabular-nums text-ember-orange">
                {formatSharePrice(noProb, displayUnit, kassUsd)}
              </p>
            </div>
          </div>
        </div>
        <div className="flex justify-end">
          <UnitTabs value={unit} onChange={setUnit} usdAvailable={usdAvailable} />
        </div>
        <PriceChart pubkey={pubkey} refreshKey={tradeNonce} />
      </Card>

      {/* Order ticket — the floating buy/sell card. */}
      <Card className="flex flex-col gap-4 lg:col-span-2">
        <div className="border-b border-pebble pb-3">
          <p className="font-inter text-[11px] uppercase tracking-[0.06em] text-driftwood">Order</p>
          <p className="mt-1 text-balance font-inter text-[14px] text-sepia" title={question}>
            {question ?? "Trade this market"}
          </p>
          <p className="mt-0.5 font-inter text-[12px] text-driftwood">
            {boundLabel ? <span className="text-bronze">{boundLabel}</span> : "This outcome"}
            {" · "}
            <span className={outcome === "yes" ? "text-chestnut" : "text-ember-orange"}>
              {outcome.toUpperCase()}
            </span>
          </p>
        </div>

        <div className="flex items-center justify-between">
          <ModeTabs value={mode} onChange={setMode} />
          <span
            className="rounded-tag border border-pebble px-2.5 py-1 font-inter text-[12px] text-driftwood"
            title="Trades execute at the current AMM price"
          >
            Market order
          </span>
        </div>

        <ConnectGate connected={action.connected}>
          <form className="flex flex-col gap-4" onSubmit={onSubmit} noValidate>
            {/* Big YES/NO price toggles. */}
            <div
              role="group"
              aria-label="Outcome"
              className="grid grid-cols-2 gap-2"
            >
              <OutcomeButton
                outcome="yes"
                selected={outcome === "yes"}
                probability={yesProb}
                unit={displayUnit}
                kassUsd={kassUsd}
                onSelect={() => setOutcome("yes")}
              />
              <OutcomeButton
                outcome="no"
                selected={outcome === "no"}
                probability={noProb}
                unit={displayUnit}
                kassUsd={kassUsd}
                onSelect={() => setOutcome("no")}
              />
            </div>

            {/* Amount — large field + balance line + quick-add chips. */}
            <div className="flex flex-col gap-2">
              <div className="flex items-baseline justify-between gap-2">
                <label htmlFor={amountId} className="font-inter text-[13px] font-medium text-sepia">
                  Amount
                </label>
                <span className="font-inter text-[12px] text-driftwood">
                  {mode === "buy" ? (
                    <>
                      Balance{" "}
                      <span className="text-bronze">
                        {kass.balance === null ? "—" : `${formatKass(kass.balance)} KASS`}
                      </span>
                    </>
                  ) : (
                    <>
                      You hold{" "}
                      <span className="text-bronze">
                        {positionBalance === null ? "—" : `${formatKass(positionBalance)} ${outcome.toUpperCase()}`}
                      </span>
                    </>
                  )}
                </span>
              </div>
              <div
                className={`flex items-baseline gap-2 rounded-tag border bg-pure-card px-3 py-2.5 transition-colors focus-within:ring-2 focus-within:ring-sepia/40 focus-within:ring-offset-2 focus-within:ring-offset-parchment ${
                  inputError ? "border-ember-orange/60" : "border-pebble"
                }`}
              >
                <input
                  id={amountId}
                  aria-describedby={descId}
                  aria-invalid={Boolean(inputError)}
                  inputMode="decimal"
                  placeholder="0"
                  value={amount}
                  onChange={(e) => setAmount(e.target.value)}
                  className="w-full bg-transparent font-serif text-heading-sm font-light tabular-nums text-sepia placeholder:text-driftwood focus:outline-none"
                />
                <span className="font-inter text-[13px] text-driftwood">
                  {mode === "buy" ? "KASS" : "shares"}
                </span>
              </div>
              <p id={descId} className="min-h-[1rem] font-inter text-[12px]">
                {inputError ? <span className="text-ember-orange">{inputError}</span> : null}
              </p>
              <div className="grid grid-cols-4 gap-2">
                {PRESETS.map((n) => (
                  <button
                    key={n}
                    type="button"
                    onClick={() => bump(n)}
                    className="rounded-tag border border-pebble bg-soft-cream px-2 py-1.5 font-inter text-[13px] tabular-nums text-sepia transition-colors hover:border-driftwood active:scale-[0.96]"
                  >
                    +{n}
                  </button>
                ))}
                <button
                  type="button"
                  onClick={setMax}
                  className="rounded-tag border border-pebble bg-soft-cream px-2 py-1.5 font-inter text-[13px] text-sepia transition-colors hover:border-driftwood active:scale-[0.96]"
                >
                  Max
                </button>
              </div>
            </div>

            {/* Live "you receive" estimate (buy only). */}
            {mode === "buy" && buyReceived !== null ? (
              <div className="flex items-baseline justify-between rounded-tag bg-soft-cream px-3 py-2 font-inter text-[13px]">
                <span className="text-driftwood">You receive ≈</span>
                <span className="tabular-nums text-sepia">
                  {formatKass(buyReceived)} {outcome.toUpperCase()} shares
                </span>
              </div>
            ) : null}

            <SubmitButton
              className="w-full py-3 text-[15px]"
              verb={mode === "buy" ? `Buy ${outcome.toUpperCase()}` : `Sell ${outcome.toUpperCase()}`}
              status={action.status}
              disabled={Boolean(balanceError)}
            />
            <WriteStatusRegion status={action.status} successVerb={mode === "buy" ? "Bought" : "Sold"} />

            {/* Jupiter any-token entry: DISABLED (deferred). */}
            {/* TODO wire buildJupiterEntryRequest + app fetch (GET /quote → POST /swap) + composeWithEntry. */}
            <label
              className="flex cursor-not-allowed items-center gap-2 font-inter text-[12px] text-stone"
              title="Coming soon — pay with USDC/SOL via Jupiter"
            >
              <input type="checkbox" disabled className="cursor-not-allowed" />
              Pay with any token (Jupiter) — coming soon
            </label>
          </form>
        </ConnectGate>
      </Card>
    </div>
  );
}

export default TradePanel;

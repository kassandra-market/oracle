import { useEffect, useState, type FormEvent } from "react";
import { useNavigate } from "react-router-dom";
import { Address } from "@solana/web3.js";
import { decodeMarketOracle, pda } from "@kassandra-market/markets";
import { Card } from "../../ui";
import { buildCreateMarketIxs, buildCreateAllSteps, type ActivateStep } from "../../../market/data/actions";
import { useWriteAction } from "../../../market/hooks/useWriteAction";
import { useActionSequence, type StepStatus } from "../../../market/hooks/useActionSequence";
import { useConfig } from "../../../market/hooks/useMarketDetail";
import { useKassBalance } from "../../../market/hooks/useKassBalance";
import { formatKass, outcomeLabel } from "../../../market/lib/marketView";
import { parseKassAmount, kassBalanceGateError } from "../../../market/data/amount";
import { ConnectGate } from "./ConnectGate";
import { Field, KassBalanceLine, SubmitButton, TextInput } from "./formPrimitives";
import { WriteStatusRegion } from "./WriteStatusRegion";

const focusRing =
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40 " +
  "focus-visible:ring-offset-2 focus-visible:ring-offset-parchment";

/**
 * Create a new prediction market bound to an existing Kassandra oracle: an oracle
 * address and a KASS seed (pools always seed 50/50). Once
 * the oracle is entered the form reads its `options_count`: a binary (2-option)
 * oracle stays a simple form (`outcome_index = 0`), while a categorical (N>2)
 * oracle exposes an OUTCOME SELECTOR — the created sub-market is "YES if the
 * oracle resolves to outcome ⟨i⟩" — and a BATCH mode that creates ALL N outcome
 * sub-markets in one resumable multi-tx sequence (skip-if-exists resume). On a
 * single create it navigates to the new sub-market's detail; on a batch create it
 * navigates to the grouped `/markets` view.
 */
export function CreateMarketForm() {
  const navigate = useNavigate();
  const config = useConfig();
  const kassMint = config.data ? config.data.kassMint.toString() : undefined;
  const notInitialized = !config.loading && config.data === null;
  const { balance, loading: balanceLoading, refetch: refetchBalance } = useKassBalance(kassMint);

  const [oracle, setOracle] = useState("");
  const [seed, setSeed] = useState("");
  const [oracleError, setOracleError] = useState<string | undefined>();
  const [seedError, setSeedError] = useState<string | undefined>();

  // The linked oracle's options_count once the address is read:
  //   undefined = not yet read / no address, null = unreadable, number = known.
  const [optionsCount, setOptionsCount] = useState<number | null | undefined>(undefined);
  const [outcomeIndex, setOutcomeIndex] = useState(0);
  // Client-side outcome labels (there are NO on-chain labels — purely cosmetic).
  const [labels, setLabels] = useState<Record<number, string>>({});

  // Batch "create all N outcomes" mode (categorical oracles only).
  const [batchMode, setBatchMode] = useState(false);
  const [batchSteps, setBatchSteps] = useState<ActivateStep[] | null>(null);
  const [batchError, setBatchError] = useState<string | undefined>();

  const action = useWriteAction(() => {
    refetchBalance();
    // Navigate to the new sub-market's detail once confirmed (PDA keyed by
    // (oracle, outcomeIndex)).
    void pda
      .market(oracle.trim(), outcomeIndex)
      .then(({ address }) => navigate(`/markets/${address.toString()}`));
  });

  // The batch sequence: on completion refetch balance + navigate to the grouped
  // categorical view (the created sub-markets appear as one card there).
  const seq = useActionSequence(() => {
    refetchBalance();
    navigate("/markets");
  });

  // Read the entered oracle's options_count so a categorical oracle can offer an
  // outcome selector. Debounced by React's effect batching; races are guarded by
  // a cancellation flag so a stale read never clobbers a newer address.
  const trimmedOracle = oracle.trim();
  const { indexer } = action;
  useEffect(() => {
    if (trimmedOracle === "") {
      setOptionsCount(undefined);
      return;
    }
    let addr: Address;
    try {
      addr = new Address(trimmedOracle);
    } catch {
      setOptionsCount(null);
      return;
    }
    let cancelled = false;
    setOptionsCount(undefined);
    void indexer
      .getAccount(addr.toString())
      .then((acct) => {
        if (cancelled) return;
        if (!acct || acct.data.length === 0) {
          setOptionsCount(null);
          return;
        }
        try {
          setOptionsCount(decodeMarketOracle(acct.data).optionsCount);
        } catch {
          setOptionsCount(null);
        }
      })
      .catch(() => {
        if (!cancelled) setOptionsCount(null);
      });
    return () => {
      cancelled = true;
    };
  }, [trimmedOracle, indexer]);

  // Keep the chosen outcome in range as the oracle (options_count) changes.
  useEffect(() => {
    if (typeof optionsCount === "number" && outcomeIndex >= optionsCount) setOutcomeIndex(0);
  }, [optionsCount, outcomeIndex]);

  const isCategorical = typeof optionsCount === "number" && optionsCount > 2;

  // Batch mode only makes sense for a categorical oracle; drop it otherwise.
  useEffect(() => {
    if (!isCategorical) setBatchMode(false);
  }, [isCategorical]);

  const parsedSeed = parseKassAmount(seed);
  // In batch mode the creator funds `optionsCount × seed`; gate on that total.
  const totalCost =
    batchMode && parsedSeed.value !== undefined && typeof optionsCount === "number"
      ? parsedSeed.value * BigInt(optionsCount)
      : undefined;
  const balanceError = kassBalanceGateError(
    batchMode ? totalCost : parsedSeed.value,
    balance,
  );

  const enableBatch = () => setBatchMode(true);

  /** Shared field validation for both modes; returns the parsed values or null. */
  const validate = (): { seedValue: bigint } | null => {
    if (trimmedOracle === "") {
      setOracleError("Enter the Kassandra oracle address.");
      return null;
    }
    if (parsedSeed.error) {
      setSeedError(parsedSeed.error);
      return null;
    }
    if (!kassMint) {
      setOracleError("Waiting for the on-chain config (KASS mint) to load.");
      return null;
    }
    setOracleError(undefined);
    setSeedError(undefined);
    return { seedValue: parsedSeed.value! };
  };

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (typeof optionsCount === "number" && outcomeIndex >= optionsCount) {
      setOracleError(`Outcome must be between 0 and ${optionsCount - 1}.`);
      return;
    }
    const v = validate();
    if (!v) return;
    void action.run(() =>
      buildCreateMarketIxs({
        indexer: action.indexer,
        oracle: trimmedOracle,
        kassMint: kassMint!,
        creator: action.address!,
        seedAmount: v.seedValue,
        outcomeIndex,
      }),
    );
  };

  const onBatchSubmit = async (e: FormEvent) => {
    e.preventDefault();
    if (seq.busy) return;
    if (typeof optionsCount !== "number") return;
    const v = validate();
    if (!v) return;
    try {
      setBatchError(undefined);
      const built = await buildCreateAllSteps({
        indexer: action.indexer,
        oracle: trimmedOracle,
        optionsCount,
        creator: seq.address!,
        kassMint: kassMint!,
        seedAmount: v.seedValue,
      });
      setBatchSteps(built);
      await seq.run(built);
    } catch (err) {
      setBatchError(err instanceof Error ? err.message : String(err));
    }
  };

  const anyBatchError = seq.statuses.some((s) => s.kind === "error");
  const batchVerb = seq.allDone
    ? "Created — view markets"
    : anyBatchError
      ? "Retry creating"
      : seq.busy
        ? "Creating…"
        : `Create all ${typeof optionsCount === "number" ? optionsCount : ""} outcomes`;
  const runningIdx = seq.statuses.findIndex((s) => s.kind === "running");

  return (
    <Card className="flex flex-col gap-4">
      <div>
        <h3 className="font-serif text-subheading font-light text-sepia">Create market</h3>
        <p className="mt-1 font-inter text-[13px] text-driftwood">
          Bind a market to an existing Kassandra oracle and seed its funding.
        </p>
      </div>
      {notInitialized ? (
        <div className="rounded-tag border border-pebble bg-soft-cream p-4">
          <p className="font-inter text-[13px] text-bronze">
            The program is not initialized (no on-chain Config), so its KASS mint is unknown. Deploy
            + initialize the program before creating a market.
          </p>
        </div>
      ) : null}
      <ConnectGate connected={action.connected}>
        <form className="flex flex-col gap-4" onSubmit={batchMode ? onBatchSubmit : onSubmit} noValidate>
          <Field
            label="Oracle address"
            hint="The Kassandra oracle this market resolves against."
            error={oracleError}
          >
            {(ids) => (
              <TextInput
                ids={ids}
                inputMode="text"
                placeholder="Base58 oracle pubkey"
                value={oracle}
                onChange={(e) => setOracle(e.target.value)}
              />
            )}
          </Field>

          {/* Mode toggle — only for a categorical (N>2) oracle: create one outcome
              at a time, or create all N in one resumable sequence. */}
          {isCategorical ? (
            <div
              role="radiogroup"
              aria-label="Create mode"
              className="inline-flex rounded-tag border border-pebble bg-soft-cream p-0.5"
            >
              <ModeButton active={!batchMode} onClick={() => setBatchMode(false)}>
                Single outcome
              </ModeButton>
              <ModeButton active={batchMode} onClick={enableBatch}>
                All {optionsCount} outcomes
              </ModeButton>
            </div>
          ) : null}

          {/* Outcome selector — categorical single-outcome mode only. A binary
              oracle stays a simple form (outcome_index 0, no selector). */}
          {isCategorical && !batchMode ? (
            <Field
              label="Outcome"
              hint="This sub-market: YES if the oracle resolves to the chosen outcome."
            >
              {(ids) => (
                <div className="flex flex-col gap-2">
                  <select
                    id={ids.id}
                    aria-describedby={ids.describedById}
                    value={outcomeIndex}
                    onChange={(e) => setOutcomeIndex(Number(e.target.value))}
                    className={`w-full rounded-tag border border-pebble bg-pure-card px-3 py-2 font-inter text-[14px] text-sepia ${focusRing}`}
                  >
                    {Array.from({ length: optionsCount as number }, (_, i) => (
                      <option key={i} value={i}>
                        {outcomeLabel(i, labels[i])}
                      </option>
                    ))}
                  </select>
                  <TextInput
                    ids={{ id: `${ids.id}-label`, describedById: ids.describedById, invalid: false }}
                    inputMode="text"
                    placeholder={`Optional label for outcome ${outcomeIndex} (not stored on-chain)`}
                    value={labels[outcomeIndex] ?? ""}
                    onChange={(e) =>
                      setLabels((prev) => ({ ...prev, [outcomeIndex]: e.target.value }))
                    }
                  />
                  <p className="font-inter text-[12px] text-driftwood">
                    This market: YES if the oracle resolves to{" "}
                    <span className="font-medium text-bronze">
                      {outcomeLabel(outcomeIndex, labels[outcomeIndex])}
                    </span>{" "}
                    (of {optionsCount} outcomes).
                  </p>
                </div>
              )}
            </Field>
          ) : isCategorical && batchMode ? (
            <p className="font-inter text-[12px] text-driftwood">
              Creates all {optionsCount} outcome sub-markets — one "YES if the oracle resolves to
              outcome ⟨i⟩" market per outcome — as {optionsCount} sequential transactions
              (resumable; already-created outcomes are skipped).
            </p>
          ) : optionsCount === 2 ? (
            <p className="font-inter text-[12px] text-driftwood">
              Binary oracle — YES if the oracle resolves to outcome 0.
            </p>
          ) : null}

          <Field
            label={batchMode ? "Seed per outcome (KASS)" : "Seed (KASS)"}
            hint="Your initial contribution to the funding pool."
            error={seedError ?? balanceError}
          >
            {(ids) => (
              <TextInput
                ids={ids}
                inputMode="decimal"
                placeholder="e.g. 1000"
                value={seed}
                onChange={(e) => setSeed(e.target.value)}
              />
            )}
          </Field>
          {batchMode && parsedSeed.value !== undefined && typeof optionsCount === "number" ? (
            <p className="-mt-1 font-inter text-[12px] text-driftwood">
              Total: {optionsCount} × seed ={" "}
              <span className="font-medium text-bronze">
                {formatKass(parsedSeed.value * BigInt(optionsCount))}
              </span>
            </p>
          ) : null}
          <KassBalanceLine balance={balance} loading={balanceLoading} format={formatKass} />

          {batchMode ? (
            <>
              {batchSteps ? <BatchStepList steps={batchSteps} statuses={seq.statuses} /> : null}
              {seq.busy && runningIdx >= 0 ? (
                <p aria-live="polite" className="font-inter text-[12px] text-bronze">
                  Creating outcome {runningIdx} of {optionsCount}…
                </p>
              ) : null}
              <div className="flex items-center gap-3">
                <button
                  type="submit"
                  disabled={seq.busy || seq.allDone || Boolean(balanceError)}
                  aria-busy={seq.busy}
                  className="inline-flex items-center justify-center gap-2 rounded-button bg-chestnut px-4 py-2.5 font-inter text-body font-medium text-liquid-abyss shadow-bloom transition-all duration-150 hover:-translate-y-px hover:brightness-110 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-phosphor focus-visible:ring-offset-2 focus-visible:ring-offset-parchment disabled:cursor-not-allowed disabled:opacity-50"
                >
                  {batchVerb}
                </button>
              </div>
              {batchError ? (
                <div className="rounded-tag border border-ember-orange/40 bg-ember-orange/10 px-3 py-2">
                  <p className="font-inter text-[13px] text-ember-orange">{batchError}</p>
                </div>
              ) : null}
            </>
          ) : (
            <>
              <div className="flex items-center gap-3">
                <SubmitButton
                  verb="Create market"
                  status={action.status}
                  disabled={Boolean(balanceError)}
                />
              </div>
              <WriteStatusRegion status={action.status} successVerb="Market created" />
            </>
          )}
        </form>
      </ConnectGate>
    </Card>
  );
}

/** One segment of the single/batch mode toggle. */
function ModeButton({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={active}
      onClick={onClick}
      className={`rounded-[10px] px-3 py-1.5 font-inter text-[13px] font-medium transition-colors ${
        active ? "bg-chestnut text-liquid-abyss shadow-bloom" : "text-driftwood hover:text-sepia"
      }`}
    >
      {children}
    </button>
  );
}

/** Compact per-outcome progress list for the batch sequence. */
function BatchStepList({ steps, statuses }: { steps: ActivateStep[]; statuses: StepStatus[] }) {
  return (
    <ol aria-live="polite" className="flex flex-col gap-1.5">
      {steps.map((step, i) => {
        const st: StepStatus = statuses[i] ?? { kind: "pending" };
        const glyph =
          st.kind === "done" ? "✓" : st.kind === "error" ? "✕" : st.kind === "running" ? "…" : "○";
        const tone =
          st.kind === "done"
            ? "text-chestnut"
            : st.kind === "error"
              ? "text-ember-orange"
              : st.kind === "running"
                ? "text-bronze"
                : "text-stone";
        return (
          <li key={step.label} className="flex flex-col gap-0.5">
            <div className="flex items-center gap-2 font-inter text-[13px]">
              <span className={`w-3 text-center font-mono text-[12px] ${tone}`}>{glyph}</span>
              <span className={st.kind === "done" ? "text-chestnut" : "text-sepia"}>
                {i + 1}. {step.label}
              </span>
              {st.kind === "done" && st.signature === "already-landed" ? (
                <span className="font-mono text-[11px] text-stone">already on-chain</span>
              ) : null}
            </div>
            {st.kind === "error" ? (
              <p className="pl-6 font-inter text-[12px] text-ember-orange">{st.message}</p>
            ) : null}
          </li>
        );
      })}
    </ol>
  );
}

export default CreateMarketForm;

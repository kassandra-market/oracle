import { useState } from "react";
import type { Market } from "@kassandra-market/markets";
import { Card } from "../../ui";
import { buildActivateSequence, type ActivateStep } from "../../../market/data/actions";
import { useActionSequence, type StepStatus } from "../../../market/hooks/useActionSequence";
import { explorerTxUrl, shortSig } from "../../../market/lib/explorer";
import { ConnectGate } from "./ConnectGate";

/**
 * Permissionless "activate" crank, shown by {@link MarketActions} once a Funding
 * market has reached its floor and its oracle is still live. It stands up the
 * market's MetaDAO scaffolding (question → conditional vault → cYES/cNO AMM) and
 * then `activate`s — splitting the escrowed KASS into the pool and seeding LP —
 * as a SEQUENCE of four wallet-signed transactions (too much account-creation +
 * CPI for one tx). A mid-sequence failure can be retried from where it stopped.
 */
export function ActivateControl({
  pubkey,
  market,
  onSuccess,
}: {
  pubkey: string;
  market: Market;
  onSuccess: () => void;
}) {
  const seq = useActionSequence(onSuccess);
  const [steps, setSteps] = useState<ActivateStep[] | null>(null);
  const [buildError, setBuildError] = useState<string | undefined>();

  const onSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (seq.busy) return;
    try {
      setBuildError(undefined);
      const built = steps ?? (await buildActivateSequence({
        market: pubkey,
        oracle: market.oracle,
        kassMint: market.kassMint,
        payer: seq.address!,
      }));
      setSteps(built);
      await seq.run(built);
    } catch (err) {
      setBuildError(err instanceof Error ? err.message : String(err));
    }
  };

  const anyError = seq.statuses.some((s) => s.kind === "error");
  const verb = seq.allDone
    ? "Activated"
    : anyError
      ? "Retry activation"
      : seq.busy
        ? "Activating…"
        : "Activate market";

  return (
    <Card className="flex flex-col gap-4">
      <div>
        <h3 className="font-serif text-subheading font-light text-sepia">Activate market</h3>
        <p className="mt-1 font-inter text-[13px] text-driftwood">
          The funding floor is met. Activation composes the cYES/cNO pool and seeds it with the
          escrowed KASS, opening the market for trading. Permissionless — anyone may crank it (four
          sequential transactions).
        </p>
      </div>
      <ConnectGate connected={seq.connected}>
        <form className="flex flex-col gap-3" onSubmit={onSubmit} noValidate>
          {steps ? <StepList steps={steps} statuses={seq.statuses} /> : null}
          <div className="flex items-center gap-3">
            <button
              type="submit"
              disabled={seq.busy || seq.allDone}
              aria-busy={seq.busy}
              className="inline-flex items-center justify-center gap-2 rounded-button bg-chestnut px-4 py-2.5 font-inter text-body font-medium text-liquid-abyss shadow-bloom transition-all duration-150 hover:-translate-y-px hover:brightness-110 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-phosphor focus-visible:ring-offset-2 focus-visible:ring-offset-parchment disabled:cursor-not-allowed disabled:opacity-50"
            >
              {verb}
            </button>
          </div>
          {buildError ? (
            <div className="rounded-tag border border-ember-orange/40 bg-ember-orange/10 px-3 py-2">
              <p className="font-inter text-[13px] text-ember-orange">{buildError}</p>
            </div>
          ) : null}
        </form>
      </ConnectGate>
    </Card>
  );
}

/** The per-step progress list (pending / running / done+sig / error+logs). */
function StepList({ steps, statuses }: { steps: ActivateStep[]; statuses: StepStatus[] }) {
  return (
    <ol aria-live="polite" className="flex flex-col gap-1.5">
      {steps.map((step, i) => {
        const st = statuses[i] ?? { kind: "pending" as const };
        return (
          <li key={step.label} className="flex flex-col gap-0.5">
            <div className="flex items-center gap-2 font-inter text-[13px]">
              <StepGlyph status={st} />
              <span className={st.kind === "done" ? "text-chestnut" : "text-sepia"}>
                {i + 1}. {step.label}
              </span>
              {st.kind === "done" && st.signature === "already-landed" ? (
                <span className="font-mono text-[11px] text-stone">already on-chain</span>
              ) : st.kind === "done" ? (
                <a
                  href={explorerTxUrl(st.signature)}
                  target="_blank"
                  rel="noreferrer noopener"
                  className="font-mono text-[11px] text-driftwood underline decoration-pebble underline-offset-4 hover:text-sepia"
                >
                  {shortSig(st.signature)}
                </a>
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

function StepGlyph({ status }: { status: StepStatus }) {
  const glyph =
    status.kind === "done" ? "✓" : status.kind === "error" ? "✕" : status.kind === "running" ? "…" : "○";
  const tone =
    status.kind === "done"
      ? "text-chestnut"
      : status.kind === "error"
        ? "text-ember-orange"
        : status.kind === "running"
          ? "text-bronze"
          : "text-stone";
  return <span className={`w-3 text-center font-mono text-[12px] ${tone}`}>{glyph}</span>;
}

export default ActivateControl;

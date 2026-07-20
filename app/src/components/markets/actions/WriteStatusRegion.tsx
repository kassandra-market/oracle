import { explorerTxUrl, shortSig } from "../../../market/lib/explorer";
import type { WriteStatus } from "../../../market/data/writeAction";

const BUSY_LABEL: Record<"building" | "signing" | "confirming", string> = {
  building: "Preparing transaction…",
  signing: "Awaiting wallet signature…",
  confirming: "Confirming on-chain…",
};

/**
 * The per-form status region under a submit button. `aria-live="polite"` so the
 * transition (building → signing → confirming → success/error) is announced.
 * Quiet silver for in-flight + aqua for success, ember reserved for
 * the error accent only. On-chain fields (a program log, the signature) are
 * rendered as inert text — never linked/executed except the explorer link.
 */
export function WriteStatusRegion({
  status,
  successVerb = "Done",
}: {
  status: WriteStatus;
  /** Past-tense verb for the confirmation line, e.g. "Market created" / "Contributed". */
  successVerb?: string;
}) {
  return (
    <div aria-live="polite" className="min-h-[1.25rem]">
      {status.kind === "building" || status.kind === "signing" || status.kind === "confirming" ? (
        <p className="status-enter font-inter text-[13px] text-silver">{BUSY_LABEL[status.kind]}</p>
      ) : null}

      {status.kind === "success" ? (
        <p className="status-enter status-enter-success font-inter text-[13px] text-aqua">
          {successVerb} · <span className="font-mono">{shortSig(status.signature)}</span>
          {" · "}
          <a
            href={explorerTxUrl(status.signature)}
            target="_blank"
            rel="noreferrer noopener"
            className="underline decoration-hairline underline-offset-4 hover:text-platinum focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-platinum/40 focus-visible:ring-offset-2 focus-visible:ring-offset-liquid-abyss"
          >
            View on Explorer
          </a>
        </p>
      ) : null}

      {status.kind === "error" ? (
        <div className="status-enter rounded-tag border border-coral/40 bg-coral/10 px-3 py-2">
          <p className="font-inter text-[13px] text-coral">{status.message}</p>
          {status.logs && status.logs.length > 0 ? (
            <pre className="mt-1.5 max-h-32 overflow-auto whitespace-pre-wrap break-all font-mono text-[11px] text-silver">
              {status.logs.join("\n")}
            </pre>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

export default WriteStatusRegion;

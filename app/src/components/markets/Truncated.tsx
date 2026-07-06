import { useState } from "react";
import { truncateMiddle } from "../../market/lib/marketView";

export interface TruncatedProps {
  /** The full value (pubkey / signature). Displayed truncated; copied in full. */
  value: string;
  /** Chars kept at the head / tail of the truncation. */
  head?: number;
  tail?: number;
  /** Render as a copy-on-click button (via `navigator.clipboard`). */
  copyable?: boolean;
  /** Accessible noun for the value, e.g. "market address" — used in the copy label. */
  label?: string;
  className?: string;
}

/**
 * A reusable truncated identifier. Pubkeys/signatures show as `Abc1…Xy9z` in
 * monospace; when `copyable`, the whole thing is a labelled button that writes
 * the full value to the clipboard and flashes a transient confirmation. No ember
 * here — the copy affordance stays sepia to protect the ember budget.
 */
export function Truncated({
  value,
  head = 4,
  tail = 4,
  copyable = false,
  label = "value",
  className = "",
}: TruncatedProps) {
  const [copied, setCopied] = useState(false);
  const shown = truncateMiddle(value, head, tail);

  if (!copyable) {
    return (
      <span className={`font-mono text-[13px] text-bronze ${className}`} title={value}>
        {shown}
      </span>
    );
  }

  const onCopy = () => {
    const clip = typeof navigator !== "undefined" ? navigator.clipboard : undefined;
    if (!clip) return;
    void clip
      .writeText(value)
      .then(() => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1200);
      })
      .catch(() => {
        /* clipboard denied (insecure context) — no-op, the value is still visible */
      });
  };

  return (
    <button
      type="button"
      onClick={onCopy}
      title={`Copy ${label}: ${value}`}
      aria-label={copied ? `${label} copied to clipboard` : `Copy ${label} ${value}`}
      className={`group inline-flex items-center gap-1 rounded-sm font-mono text-[13px] text-bronze transition-colors hover:text-sepia focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40 focus-visible:ring-offset-2 focus-visible:ring-offset-parchment ${className}`}
    >
      <span>{shown}</span>
      <span aria-hidden="true" className="text-[11px] text-driftwood group-hover:text-sepia">
        {copied ? "✓ copied" : "⧉"}
      </span>
    </button>
  );
}

export default Truncated;

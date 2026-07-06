import type { MarketStatus } from "@kassandra-market/sdk";
import { statusChipClasses, statusLabel } from "../../market/lib/marketView";

/**
 * The market lifecycle-status chip. Maps a {@link MarketStatus} → a readable
 * label + an on-brand Delphi tone (Active is the one ember spark). The label is
 * real text (never color-only) and an `aria-label` names it as the status.
 */
export function StatusChip({ status }: { status: MarketStatus }) {
  return (
    <span
      aria-label={`Status: ${statusLabel(status)}`}
      className={`inline-flex items-center rounded-tag border px-2.5 py-1 font-inter text-[12px] font-medium ${statusChipClasses(status)}`}
    >
      {statusLabel(status)}
    </span>
  );
}

export default StatusChip;

/**
 * A YES/NO implied-probability bar for an Active market. The ember segment is the
 * YES share (`probability`), the remainder reads as NO. `null` (empty/absent
 * pool) renders a quiet "price unavailable" placeholder. Labels are real text so
 * the split is never color-only.
 */
export function ProbabilityBar({ probability }: { probability: number | null }) {
  if (probability === null) {
    return (
      <p className="font-inter text-[12px] text-driftwood">Live price unavailable</p>
    );
  }
  // Round YES once and derive NO as the complement, so the pair always sums to
  // 100% (independently rounding both can show e.g. 64% / 37%).
  const yesPct = Math.round(probability * 100);
  const noPct = 100 - yesPct;
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-baseline justify-between font-inter text-[12px]">
        <span className="font-medium text-ember-orange">YES {yesPct}%</span>
        <span className="text-driftwood">NO {noPct}%</span>
      </div>
      <div
        className="h-1.5 w-full overflow-hidden rounded-sm bg-soft-cream"
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={yesPct}
        aria-label="Implied YES probability"
      >
        <div
          className="h-full rounded-sm bg-ember-orange transition-all"
          style={{ width: `${yesPct}%` }}
        />
      </div>
    </div>
  );
}

export default ProbabilityBar;

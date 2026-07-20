/**
 * Pure presentation helpers for the market browse views — status → label/chip
 * mapping, funding progress, implied-probability + KASS formatting, resolution
 * outcome text, and pubkey truncation. NO React here; the pages + chip
 * components consume these, and `app/test/marketView.test.ts` unit-tests them.
 */
import {
  MarketStatus,
  Phase,
  type Contribution,
  type Market,
  type MarketOracle,
} from "@kassandra-market/markets";
import type { AmmReserves, MarketSummary } from "../data/markets";

/** The winning categorical option indices for a binary market. */
export const YES_OPTION = 0;
export const NO_OPTION = 1;
/** `resolvedOption` sentinel for a dead-end (no valid option). Mirrors `state.rs` 0xFF. */
export const RESOLVED_OPTION_NONE = 0xff;
/** KASS mint decimals (raw base units → human amount). */
export const KASS_DECIMALS = 9;

/** On-brand chip tones (mirrors the sibling oracle `Chip` vocabulary). */
export type ChipTone = "neutral" | "info" | "ember" | "confirmed" | "muted";

/** Tailwind class strings per {@link ChipTone}. */
export const CHIP_TONE_CLASSES: Record<ChipTone, string> = {
  // Quiet default — achromatic warm.
  neutral: "border-hairline bg-liquid-deep text-silver",
  // Subtle cyan hint — crowdfunding in progress (Funding).
  info: "border-cyan-phosphor/30 bg-cyan-phosphor/10 text-cyan-phosphor",
  // The single ember punctuation moment — the live/active market.
  ember: "border-coral/40 bg-coral/10 text-coral",
  // A calm, grounded aqua "confirmed" for resolution.
  confirmed: "border-aqua/30 bg-aqua/10 text-aqua",
  // Lowest-emphasis silver-dim for voided / cancelled.
  muted: "border-hairline bg-transparent text-silver-dim",
};

/** Human label for a {@link MarketStatus}. */
export function statusLabel(status: MarketStatus): string {
  switch (status) {
    case MarketStatus.Funding:
      return "Funding";
    case MarketStatus.Active:
      return "Active";
    case MarketStatus.Resolved:
      return "Resolved";
    case MarketStatus.Void:
      return "Void";
    case MarketStatus.Cancelled:
      return "Cancelled";
    default:
      return "Unknown";
  }
}

/** On-brand chip tone for a {@link MarketStatus} (ember reserved for the live market). */
export function statusTone(status: MarketStatus): ChipTone {
  switch (status) {
    case MarketStatus.Active:
      return "ember";
    case MarketStatus.Resolved:
      return "confirmed";
    case MarketStatus.Void:
    case MarketStatus.Cancelled:
      return "muted";
    case MarketStatus.Funding:
      return "info";
    default:
      return "neutral";
  }
}

/** The chip class string for a {@link MarketStatus}. */
export function statusChipClasses(status: MarketStatus): string {
  return CHIP_TONE_CLASSES[statusTone(status)];
}

/** Which exits a `Funding` market's UI should offer. */
export interface FundingActions {
  /** Crank the funded market live (needs the floor reached AND a live oracle). */
  canActivate: boolean;
  /** Cancel → refund. The ONLY exit once the oracle is terminal. */
  canCancel: boolean;
}

/**
 * UI gating for a `Funding` market's exits.
 *
 * A terminal oracle makes `activate` impossible, so **Cancel (→ refund) is the
 * only exit at ANY funding level** — including a fully-funded market whose oracle
 * resolved before activation (mirrors `cancel.rs`, which admits exactly that so
 * contributions aren't stranded). `activate` needs the floor reached AND a live
 * oracle. The two are mutually exclusive (cancel requires terminal, activate
 * requires non-terminal).
 */
export function fundingActions(funded: boolean, oracleTerminal: boolean): FundingActions {
  return { canActivate: funded && !oracleTerminal, canCancel: oracleTerminal };
}

export interface FundingProgress {
  /** Progress `0..1` (clamped) of `totalContributed / minLiquidity`. */
  pct: number;
  /** True once contributions reach the funding floor. */
  funded: boolean;
}

/**
 * Funding progress toward the market's `minLiquidity` floor. `pct` is clamped to
 * `0..1`; `funded` is the true bigint compare (`totalContributed >= minLiquidity`)
 * so a rounded `pct` never lies about the threshold. A zero/absent floor reports
 * fully funded (`pct: 1`).
 */
export function fundingProgress(market: Pick<Market, "totalContributed" | "minLiquidity">): FundingProgress {
  const { totalContributed, minLiquidity } = market;
  if (minLiquidity <= 0n) return { pct: 1, funded: true };
  const funded = totalContributed >= minLiquidity;
  if (funded) return { pct: 1, funded: true };
  // Both < 2^53-ish in practice; ratio only, full precision not needed here.
  const pct = Number(totalContributed) / Number(minLiquidity);
  return { pct: Math.max(0, Math.min(1, pct)), funded: false };
}

/**
 * Implied YES probability `0..1` from the cYES/cNO pool reserves. base = cYES,
 * quote = cNO, so a large YES reserve (cheap YES) → low probability:
 * `P(YES) = quote / (base + quote)`. Returns `null` when reserves are absent or
 * the pool is empty (probability undefined).
 */
export function impliedYesProbability(reserves: AmmReserves | null | undefined): number | null {
  if (!reserves) return null;
  const total = reserves.base + reserves.quote;
  if (total <= 0n) return null;
  return Number(reserves.quote) / Number(total);
}

/**
 * Mark-to-market KASS value of the cYES/cNO pool. Each conditional token is worth
 * its win probability (cYES → P(YES), cNO → P(NO)), so the pool marks to
 * `base·P(YES) + quote·P(NO) = 2·base·quote / (base + quote)` — which reduces to
 * the complete-set value at a 50/50 pool and adds the excess side's probability
 * weight otherwise. Base units (KASS decimals); `null` when reserves are absent or
 * the pool is empty.
 */
export function poolValueKass(reserves: AmmReserves | null | undefined): bigint | null {
  if (!reserves) return null;
  const sum = reserves.base + reserves.quote;
  if (sum <= 0n) return null;
  return (2n * reserves.base * reserves.quote) / sum;
}

/**
 * A contributor's gross LP position — the honest basis `claim_lp` pays out on:
 * their funding stake's pro-rata share of the LP minted at activation
 * (`amount / activationContributed × activationLp`) plus any LP they added
 * post-activation (`lateLp`). Zero funding-derived LP before activation
 * (`activationContributed == 0`); a pure late LP contributes only `lateLp`.
 */
export function contributorLp(
  contribution: Pick<Contribution, "amount" | "lateLp">,
  market: Pick<Market, "activationLp" | "activationContributed">,
): bigint {
  const fundingLp =
    market.activationContributed > 0n
      ? (contribution.amount * market.activationLp) / market.activationContributed
      : 0n;
  return fundingLp + contribution.lateLp;
}

/** Format a `0..1` probability as a whole-percent string (`0.634` → `63%`); `null` → `—`. */
export function formatProbability(p: number | null): string {
  if (p === null || Number.isNaN(p)) return "—";
  return `${Math.round(p * 100)}%`;
}

/**
 * Format a raw base-unit KASS amount ({@link KASS_DECIMALS} decimals) as a human
 * string with thousands separators, trimming trailing fractional zeros:
 * `1234500000000n` → `1,234.5`, `1000000000n` → `1`.
 */
export function formatKass(amount: bigint): string {
  const neg = amount < 0n;
  const abs = neg ? -amount : amount;
  const scale = 10n ** BigInt(KASS_DECIMALS);
  const whole = abs / scale;
  const frac = abs % scale;
  const wholeStr = whole.toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  let out = wholeStr;
  if (frac > 0n) {
    const fracStr = frac.toString().padStart(KASS_DECIMALS, "0").replace(/0+$/, "");
    out = `${wholeStr}.${fracStr}`;
  }
  return neg ? `-${out}` : out;
}

/** Human label for a Kassandra oracle {@link Phase}. */
export function phaseLabel(phase: Phase): string {
  switch (phase) {
    case Phase.Created:
      return "Created";
    case Phase.Proposal:
      return "Proposal";
    case Phase.FactProposal:
      return "Fact proposal";
    case Phase.FactVoting:
      return "Fact voting";
    case Phase.AiClaim:
      return "AI claim";
    case Phase.Challenge:
      return "Challenged";
    case Phase.FinalRecompute:
      return "Final recompute";
    case Phase.Resolved:
      return "Resolved";
    case Phase.InvalidDeadend:
      return "Dead end";
    default:
      return "Unknown";
  }
}

/**
 * Human resolution text for a market's linked oracle. `Resolved` → "YES won" /
 * "NO won" (from `resolvedOption` with YES=0 / NO=1, else "Resolved");
 * `InvalidDeadend` → "Voided"; any other phase falls back to the phase label. A
 * `null` oracle (unreadable) → "Oracle unavailable".
 */
export function resolutionText(oracle: MarketOracle | null | undefined): string {
  if (!oracle) return "Oracle unavailable";
  if (oracle.phase === Phase.Resolved) {
    if (oracle.resolvedOption === YES_OPTION) return "YES won";
    if (oracle.resolvedOption === NO_OPTION) return "NO won";
    return "Resolved";
  }
  if (oracle.phase === Phase.InvalidDeadend) return "Voided";
  return phaseLabel(oracle.phase);
}

/**
 * Display label for a categorical outcome: the caller's client-side label when
 * present (labels are NOT on-chain — the oracle binds only a prompt hash), else
 * a generic `Outcome {index}`.
 */
export function outcomeLabel(index: number, label?: string | null): string {
  const trimmed = label?.trim();
  return trimmed && trimmed !== "" ? trimmed : `Outcome ${index}`;
}

/** One outcome sub-market condensed for a categorical group row. */
export interface OutcomeRow {
  /** The oracle outcome this sub-market binds to (`market.outcomeIndex`). */
  index: number;
  /** Display label ({@link outcomeLabel}). */
  label: string;
  /** Base58 sub-market PDA (its detail route target). */
  pubkey: string;
  /** Implied YES probability `0..1` (the outcome's chance), or `null`. */
  probability: number | null;
  /** The sub-market's lifecycle status. */
  status: MarketStatus;
}

/**
 * Condense a sub-market {@link MarketSummary} into an {@link OutcomeRow} for the
 * grouped categorical view — its outcome index, YES probability (the implied
 * chance of that outcome, from its pool reserves), status, and a display label.
 */
export function outcomeRow(summary: MarketSummary, label?: string | null): OutcomeRow {
  const index = summary.market.outcomeIndex;
  return {
    index,
    label: outcomeLabel(index, label),
    pubkey: summary.pubkey,
    probability: impliedYesProbability(summary.reserves),
    status: summary.market.status,
  };
}

/**
 * Resolution text for a categorical sub-market (YES = the oracle resolves to
 * `outcomeIndex`). `Resolved` → "YES won" when the oracle's winning option is
 * this outcome, else "NO won"; `InvalidDeadend` → "Voided"; otherwise the phase
 * label. A `null` oracle → "Oracle unavailable".
 */
export function outcomeResolutionText(
  oracle: MarketOracle | null | undefined,
  outcomeIndex: number,
): string {
  if (!oracle) return "Oracle unavailable";
  if (oracle.phase === Phase.Resolved) {
    return oracle.resolvedOption === outcomeIndex ? "YES won" : "NO won";
  }
  if (oracle.phase === Phase.InvalidDeadend) return "Voided";
  return phaseLabel(oracle.phase);
}

/**
 * The base58 pubkey of the prediction sub-market to link to for an `oracle` — the
 * lowest-outcome sub-market that resolves against it (its detail page shows the
 * whole categorical group), or `undefined` when none is bound yet. Powers the
 * oracle page's reverse link to its market.
 */
export function firstBoundMarketPubkey(
  markets: MarketSummary[],
  oracle: string,
): string | undefined {
  return markets
    .filter((m) => m.market.oracle.toString() === oracle)
    .sort((a, b) => a.market.outcomeIndex - b.market.outcomeIndex)[0]?.pubkey;
}

/** Truncate a long identifier keeping `head`+`tail` chars: `Abc1…Xy9z`. */
export function truncateMiddle(value: string, head = 4, tail = 4): string {
  if (value.length <= head + tail + 1) return value;
  return `${value.slice(0, head)}…${value.slice(-tail)}`;
}

/** The market-detail page's four top-level views. */
export type DetailView = "loading" | "error" | "ready" | "empty";

/**
 * Pick the market-detail page's top-level view from its async read state.
 *
 * `ready` (render the detail) WINS whenever we already hold data for the CURRENT
 * market — even mid-refetch (`loading` true). The Active-market poll refetches
 * every 15s and the read layer flips `loading` true on each run while keeping the
 * data; without this precedence the page would blank back to the skeleton on every
 * tick, remounting the trade/contribute forms and wiping any in-progress input.
 * The skeleton therefore shows only before the current market's FIRST load, and
 * when navigating to a DIFFERENT market (data still holds the previous one — a
 * `data.pubkey` mismatch — so we don't flash stale detail).
 */
export function detailView(
  pubkey: string | undefined,
  data: { pubkey: string } | undefined,
  loading: boolean,
  error: unknown,
): DetailView {
  if (data && data.pubkey === pubkey) return "ready";
  if (loading) return "loading";
  if (error) return "error";
  return "empty";
}

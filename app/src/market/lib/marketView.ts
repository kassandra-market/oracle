/**
 * Pure presentation helpers for the market browse views — status → label/chip
 * mapping, funding progress, implied-probability + KASS formatting, resolution
 * outcome text, and pubkey truncation. NO React here; the pages + chip
 * components consume these, and `app/test/marketView.test.ts` unit-tests them.
 */
import { MarketStatus, Phase, type Market, type MarketOracle } from "@kassandra-market/markets";
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

/** Tailwind class strings (Delphi tokens) per {@link ChipTone}. */
export const CHIP_TONE_CLASSES: Record<ChipTone, string> = {
  // Quiet default — achromatic warm.
  neutral: "border-pebble bg-soft-cream text-bronze",
  // Subtle cyan hint — crowdfunding in progress (Funding).
  info: "border-cobalt/30 bg-cobalt/10 text-cyan-phosphor",
  // The single ember punctuation moment — the live/active market.
  ember: "border-ember-orange/40 bg-ember-orange/10 text-ember-orange",
  // A calm, grounded aqua "confirmed" for resolution.
  confirmed: "border-chestnut/30 bg-chestnut/10 text-chestnut",
  // Lowest-emphasis stone for voided / cancelled.
  muted: "border-pebble bg-transparent text-stone",
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

/** The Delphi chip class string for a {@link MarketStatus}. */
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

/** Truncate a long identifier keeping `head`+`tail` chars: `Abc1…Xy9z`. */
export function truncateMiddle(value: string, head = 4, tail = 4): string {
  if (value.length <= head + tail + 1) return value;
  return `${value.slice(0, head)}…${value.slice(-tail)}`;
}

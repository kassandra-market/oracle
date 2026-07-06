/**
 * KASS amount parsing for the funding forms (pure, NO React).
 *
 * The forms take a HUMAN-decimal KASS amount (e.g. `1.5`), not raw base units —
 * {@link parseKassAmount} scales it by 10^{@link KASS_DECIMALS} into the `bigint`
 * base-unit value the SDK builders consume, rejecting malformed / over-precise /
 * non-positive input with an inline message. Formatting back to a human string
 * for display lives in `lib/marketView` (`formatKass`).
 */
import { KASS_DECIMALS } from "../lib/marketView";

/** 10^KASS_DECIMALS — the base-unit scale for one whole KASS. */
const KASS_SCALE = 10n ** BigInt(KASS_DECIMALS);

/**
 * Parse a human KASS amount (`"1.5"`, `"1000"`, `".25"`) into raw base units.
 * Returns `{ value }` on success or `{ error }` with a form message:
 *  - empty / non-numeric → prompts a valid amount,
 *  - more than {@link KASS_DECIMALS} fractional digits → too-precise,
 *  - zero / negative → must be greater than zero.
 */
export function parseKassAmount(raw: string): { value?: bigint; error?: string } {
  const t = raw.trim();
  if (t === "") return { error: "Enter a KASS amount." };
  const m = /^(\d*)(?:\.(\d*))?$/.exec(t);
  if (!m || (m[1] === "" && (m[2] ?? "") === "")) {
    return { error: "Amount must be a number, e.g. 1.5." };
  }
  const whole = m[1] === "" ? "0" : m[1];
  const frac = m[2] ?? "";
  if (frac.length > KASS_DECIMALS) {
    return { error: `KASS supports at most ${KASS_DECIMALS} decimal places.` };
  }
  const value = BigInt(whole) * KASS_SCALE + BigInt(frac.padEnd(KASS_DECIMALS, "0") || "0");
  if (value <= 0n) return { error: "Amount must be greater than zero." };
  return { value };
}

/**
 * Additive KASS-balance gate: a message when the entered `amount` can't be
 * covered by `balance`, else `undefined`. A `null` balance (disconnected /
 * loading / transient error) never blocks — the on-chain tx is the ultimate
 * guard. Mirrors the sibling app's `balanceGateError`, formatted at 9 decimals.
 */
export function kassBalanceGateError(
  amount: bigint | undefined,
  balance: bigint | null,
): string | undefined {
  if (balance === null) return undefined;
  if (balance === 0n) return "You have no KASS — you need KASS to participate.";
  if (amount !== undefined && amount > balance) {
    return "Amount exceeds your KASS balance.";
  }
  return undefined;
}

/**
 * Bulk liquidity write ACTIONS (pure ix-builders, NO React) for a categorical
 * oracle's group of sub-markets: deposit into (contribute) or withdraw from
 * (claim-LP) several/all sub-markets at once, as an {@link ActivateStep} sequence
 * driven by `useActionSequence`.
 *
 * The DEFAULT deposit distribution is a UNIFORM share of the entered total across
 * the fundable sub-markets ({@link uniformSplit}). Each step reuses the existing
 * single-market builder, and sets `skipIfLanded: false` — contribute/claim-LP
 * legitimately act on an existing account, so the sequence must never skip them.
 *
 * {@link outcomesReadyToActivate} + {@link buildBulkActivateSteps} extend this to
 * the funding→activation handoff: whenever a deposit is ABOUT to push one or more
 * Funding outcomes over their floor (or sweeps in one already over it that nobody
 * has cranked yet), their `activate` sequence is appended to the SAME batch —
 * "if a transaction is about to fund the market, it should also advance to the
 * next phase if possible" — instead of leaving a separate manual crank for later.
 */
import { buildContributeIxs } from "./contribute";
import { buildClaimLpIxs } from "./claimLp";
import { buildAddLiquidityIxs } from "./addLiquidity";
import { buildActivateSequence, type ActivateStep } from "./activate";
import { toAddress, type AddressInput } from "./ata";
import type { IndexerReads } from "../../lib/indexer";
import type { Market } from "@kassandra-market/markets";
import type { AmmReserves } from "../markets";
import { fundingProgress } from "../../lib/marketView";
import { ValidationError } from "../writeAction";

/**
 * Split `total` into `n` as-even-as-possible non-negative base-unit shares. Any
 * indivisible remainder (0..n-1 base units) is spread one-per across the leading
 * shares, so the shares always sum EXACTLY to `total` (no dust lost/created).
 * `total=10, n=3 → [4,3,3]`; `total=10, n=4 → [3,3,2,2]`; `n<=0 → []`.
 */
export function uniformSplit(total: bigint, n: number): bigint[] {
  if (n <= 0) return [];
  if (total < 0n) throw new ValidationError("Amount must be zero or greater.");
  const base = total / BigInt(n);
  let remainder = total - base * BigInt(n);
  const shares: bigint[] = [];
  for (let i = 0; i < n; i++) {
    const extra = remainder > 0n ? 1n : 0n;
    remainder -= extra;
    shares.push(base + extra);
  }
  return shares;
}

/** One sub-market's deposit: its PDA, a display label, and the KASS to contribute. */
export interface BulkContributeEntry {
  market: AddressInput;
  label: string;
  amount: bigint;
}

export interface BuildBulkContributeArgs {
  indexer: IndexerReads;
  /** Canonical KASS mint (shared by every sub-market in the group). */
  kassMint: AddressInput;
  /** Contributor authority (the signer). */
  contributor: AddressInput;
  /** Per-sub-market deposits; entries with `amount <= 0` are dropped. */
  entries: BulkContributeEntry[];
}

/**
 * One contribute step per funded sub-market (dropping zero-amount entries), each
 * flagged `skipIfLanded: false` so a repeat deposit is never skipped.
 */
export async function buildBulkContributeSteps(
  args: BuildBulkContributeArgs,
): Promise<ActivateStep[]> {
  const funded = args.entries.filter((e) => e.amount > 0n);
  if (funded.length === 0) {
    throw new ValidationError("Enter an amount to deposit.");
  }
  return Promise.all(
    funded.map(async (e) => ({
      label: e.label,
      ixs: await buildContributeIxs({
        indexer: args.indexer,
        market: e.market,
        kassMint: args.kassMint,
        contributor: args.contributor,
        amount: e.amount,
      }),
      checkAccount: toAddress("Market", e.market),
      skipIfLanded: false,
    })),
  );
}

/** One Funding sub-market's floor state + this deposit's share, for the
 *  funding→activation handoff ({@link outcomesReadyToActivate}). */
export interface BulkFundingEntry {
  market: AddressInput;
  /** The sub-market's own oracle (shared by every outcome in the group). */
  oracle: AddressInput;
  label: string;
  /** This outcome's `totalContributed` BEFORE the current deposit lands. */
  totalContributed: bigint;
  minLiquidity: bigint;
  /** This outcome's share of the current deposit (0 if it isn't receiving one). */
  amount: bigint;
}

/**
 * Which Funding entries will be AT OR OVER their floor once `amount` lands (or
 * already are, even with a zero share this round — sweeping in one nobody has
 * cranked yet), and therefore ready to activate in the SAME transaction batch.
 * Empty whenever the shared oracle is terminal (an `activate` there would
 * revert — those outcomes fall back to `CancelControl` instead).
 */
export function outcomesReadyToActivate(
  entries: BulkFundingEntry[],
  oracleTerminal: boolean,
): BulkFundingEntry[] {
  if (oracleTerminal) return [];
  return entries.filter(
    (e) =>
      fundingProgress({
        totalContributed: e.totalContributed + e.amount,
        minLiquidity: e.minLiquidity,
      }).funded,
  );
}

export interface BuildBulkActivateArgs {
  /** Canonical KASS mint (shared by every sub-market in the group). */
  kassMint: AddressInput;
  /** Rent payer + signer for every step (the connected keeper wallet). */
  payer: AddressInput;
  /** Pre-filtered via {@link outcomesReadyToActivate}. */
  entries: BulkFundingEntry[];
}

/**
 * One full {@link buildActivateSequence} (compose + activate, 4 steps) PER newly-
 * eligible outcome, concatenated in order — each step's label prefixed with its
 * outcome so a multi-outcome batch's step list ("Outcome 1 · Initialize
 * question", "Outcome 1 · Activate market", "Outcome 2 · …") stays legible in
 * the UI instead of four identically-labelled steps repeated per outcome.
 */
export async function buildBulkActivateSteps(args: BuildBulkActivateArgs): Promise<ActivateStep[]> {
  const groups = await Promise.all(
    args.entries.map(async (e) => {
      const steps = await buildActivateSequence({
        market: e.market,
        oracle: e.oracle,
        kassMint: args.kassMint,
        payer: args.payer,
      });
      return steps.map((s) => ({ ...s, label: `${e.label} · ${s.label}` }));
    }),
  );
  return groups.flat();
}

/**
 * One Active sub-market's add-liquidity deposit: its PDA, a display label, the
 * KASS to add, its decoded account (MetaDAO bindings + `lpTotal`), and its live
 * cYES/cNO pool reserves (to size the balanced deposit).
 */
export interface BulkAddLiquidityEntry {
  market: AddressInput;
  label: string;
  amount: bigint;
  marketAccount: Market;
  reserves: AmmReserves;
}

export interface BuildBulkAddLiquidityArgs {
  /** Depositor authority (the signer). */
  contributor: AddressInput;
  /** Per-Active-sub-market deposits; entries with `amount <= 0` are dropped. */
  entries: BulkAddLiquidityEntry[];
  /** Slippage tolerance on minted LP, in bps (default 100 = 1%). */
  slippageBps?: number;
}

/**
 * One add-liquidity step per funded Active sub-market (dropping zero-amount
 * entries), each flagged `skipIfLanded: false` so a repeat deposit is never
 * skipped. Each step's ix list already carries its own compute budget + idempotent
 * ATA creates (from {@link buildAddLiquidityIxs}).
 */
export async function buildBulkAddLiquiditySteps(
  args: BuildBulkAddLiquidityArgs,
): Promise<ActivateStep[]> {
  const funded = args.entries.filter((e) => e.amount > 0n);
  if (funded.length === 0) {
    throw new ValidationError("Enter an amount to deposit.");
  }
  return Promise.all(
    funded.map(async (e) => ({
      label: e.label,
      ixs: (
        await buildAddLiquidityIxs({
          market: e.market,
          marketAccount: e.marketAccount,
          reserves: e.reserves,
          contributor: args.contributor,
          amount: e.amount,
          slippageBps: args.slippageBps,
        })
      ).ixs,
      checkAccount: toAddress("Market", e.market),
      skipIfLanded: false,
    })),
  );
}

/** One sub-market's withdrawal: its PDA, a display label, and its LP mint. */
export interface BulkClaimLpEntry {
  market: AddressInput;
  label: string;
  lpMint: AddressInput;
}

export interface BuildBulkClaimLpArgs {
  indexer: IndexerReads;
  /** The contributor claiming across the group (the signer). */
  contributor: AddressInput;
  entries: BulkClaimLpEntry[];
}

/** One claim-LP step per eligible sub-market, flagged `skipIfLanded: false`. */
export async function buildBulkClaimLpSteps(
  args: BuildBulkClaimLpArgs,
): Promise<ActivateStep[]> {
  if (args.entries.length === 0) {
    throw new ValidationError("Nothing to withdraw.");
  }
  return Promise.all(
    args.entries.map(async (e) => ({
      label: e.label,
      ixs: await buildClaimLpIxs({
        indexer: args.indexer,
        market: e.market,
        contributor: args.contributor,
        lpMint: e.lpMint,
      }),
      checkAccount: toAddress("Market", e.market),
      skipIfLanded: false,
    })),
  );
}

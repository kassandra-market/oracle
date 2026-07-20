/**
 * The keeper `compose → activate` bring-up as a STAGED, multi-tx SEQUENCE (pure
 * ix-builders, NO React).
 *
 * Turning a fully-funded market live means standing up its MetaDAO scaffolding
 * (`initializeQuestion → initializeConditionalVault → createAmm`) and then
 * `activate` (which splits the escrow into the pool + seeds LP). That is far too
 * much account-creation + CPI to fit one transaction — the Rust harness
 * (`compose_metadao_market` + `activate`) sends each composition instruction in
 * its OWN transaction, so we mirror that: {@link buildActivateSequence} returns
 * an ORDERED list of {@link ActivateStep}s. The sequence RUNNER
 * (`useActionSequence`) then packs as many of these steps as will fit into as
 * few transactions as possible (see `data/actions/packTx`) and, when more than
 * one transaction is still needed, signs them all in a SINGLE wallet approval
 * via `signAllTransactions` — the four instructions together create a Question,
 * a vault + 2 conditional mints, an AMM + LP mint + 2 vault ATAs, and 3
 * market-owned token accounts (well past the 1232-byte tx size limit before
 * even accounting for compute cost), so they still land as multiple
 * transactions, just not multiple wallet popups.
 *
 * RESUME SAFETY: the composition instructions are NOT idempotent — each creates a
 * fresh deterministic-PDA account (Question / vault / AMM) and REVERTS with
 * "already in use" on re-submit. So a naive resume is dangerous: if a step's tx
 * actually landed while `confirmSignature` merely timed out (30s, plausible under
 * congestion), re-sending it would permanently revert. Each step therefore
 * carries the address it CREATES ({@link ActivateStep.checkAccount}); the sender
 * probes it with {@link stepAlreadyLanded} before (re)sending and SKIPS the step
 * when the account already exists — a landed-but-unconfirmed step becomes a safe
 * skip instead of a fatal revert.
 */
import { Address, type TransactionInstruction } from "@solana/web3.js";
import { flows } from "@kassandra-market/markets";
import type { IndexerReads } from "../../lib/indexer";
import { toAddress, type AddressInput } from "./ata";

/** One transaction in the activate sequence: a human label + its instructions. */
export interface ActivateStep {
  /** Short label the control renders in its step list. */
  label: string;
  /** Compute-unit limit to prepend as a `SetComputeUnitLimit` (omit for the default). */
  computeUnits?: number;
  /** The instructions for this step's transaction. */
  ixs: TransactionInstruction[];
  /**
   * The account this step CREATES. If it already exists on-chain the step has
   * already landed (even if a prior confirm timed out), so the sender SKIPS it on
   * (re)send rather than re-running the non-idempotent instruction.
   */
  checkAccount: Address;
  /**
   * Whether the sender may skip this step when {@link checkAccount} already exists
   * (default `true`). Steps whose instruction is idempotent-by-creation (create,
   * activate) leave it unset. Set `false` for steps that LEGITIMATELY repeat
   * against an existing account — e.g. `contribute` adds to an existing
   * Contribution — so the sender never wrongly skips a real deposit/withdraw.
   */
  skipIfLanded?: boolean;
}

export interface BuildActivateArgs {
  /** The `Market` PDA (`detail.pubkey`). */
  market: AddressInput;
  /** The Kassandra oracle the market resolves against (== `market.oracle`). */
  oracle: AddressInput;
  /** Canonical KASS mint (== `market.kassMint`). */
  kassMint: AddressInput;
  /** Rent payer + signer for every step (the connected keeper wallet). */
  payer: AddressInput;
}

/** Compute budget for a single composition instruction (createAmm creates the pool + LP mint + 2 ATAs). */
export const COMPOSE_COMPUTE_UNITS = 400_000;
/** Compute budget for `activate` (split_tokens + add_liquidity CPIs + 3 new token accounts). */
export const ACTIVATE_COMPUTE_UNITS = 1_400_000;

/**
 * Build the ordered `[question, vault, amm, activate]` transaction sequence.
 * Composes the MetaDAO scaffolding via `flows.composeMarketInstructions` (which
 * also yields the derived `refs`), then wires those refs into
 * `flows.activateInstruction`. Each composition instruction is its own step (the
 * on-chain reads chain question → vault → amm, so order is load-bearing) and
 * `activate` is the last.
 */
export async function buildActivateSequence(args: BuildActivateArgs): Promise<ActivateStep[]> {
  const market = toAddress("Market", args.market);
  const oracle = toAddress("Oracle", args.oracle);
  const kassMint = toAddress("KASS mint", args.kassMint);
  const payer = toAddress("Payer", args.payer);

  const { instructions, refs } = await flows.composeMarketInstructions({
    market,
    oracle,
    kassMint,
    payer,
  });
  const activateIx = await flows.activateInstruction({ refs, payer });

  const [ixQuestion, ixVault, ixAmm] = instructions;
  return [
    // Each step's `checkAccount` is the account it creates — a landed step's
    // account exists, so the sender skips it on resume (see `stepAlreadyLanded`).
    {
      label: "Initialize question",
      computeUnits: COMPOSE_COMPUTE_UNITS,
      ixs: [ixQuestion],
      checkAccount: refs.question,
    },
    {
      label: "Initialize conditional vault",
      computeUnits: COMPOSE_COMPUTE_UNITS,
      ixs: [ixVault],
      checkAccount: refs.vault,
    },
    {
      label: "Create AMM pool",
      computeUnits: COMPOSE_COMPUTE_UNITS,
      ixs: [ixAmm],
      checkAccount: refs.amm,
    },
    {
      // `activate` creates the market-owned cYES holder (among others); its
      // existence means activate already ran.
      label: "Activate market",
      computeUnits: ACTIVATE_COMPUTE_UNITS,
      ixs: [activateIx],
      checkAccount: refs.marketCyes,
    },
  ];
}

/**
 * Whether a step's `checkAccount` already exists on-chain — i.e. the step has
 * ALREADY landed (its non-idempotent instruction ran), even if a prior
 * `confirmSignature` timed out. The sequence sender calls this before (re)sending
 * each step and SKIPS the step when it returns `true`, so a resume never
 * re-submits a landed init (which would revert "already in use"). A transient RPC
 * failure resolves to `false` (don't skip — the send is the ultimate guard).
 */
export async function stepAlreadyLanded(indexer: IndexerReads, step: ActivateStep): Promise<boolean> {
  try {
    const info = await indexer.getAccount(step.checkAccount.toString());
    return info != null;
  } catch {
    return false;
  }
}

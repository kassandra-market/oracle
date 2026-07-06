/**
 * The batch "create all N outcomes" write action as a STAGED, multi-tx SEQUENCE
 * (pure ix-builders, NO React).
 *
 * A categorical Kassandra oracle (`options_count > 2`) has one binary sub-market
 * per outcome, each its own `create_market` instruction. That is one account-heavy
 * transaction per outcome — too much to pack reliably — so {@link buildCreateAllSteps}
 * returns one {@link ActivateStep} PER OUTCOME that the shared
 * {@link useActionSequence} hook sends as a resumable sequence.
 *
 * RESUME SAFETY: `create_market` reverts if its market PDA already exists (the
 * duplicate guard), so a re-run of a partially-completed batch would fatally
 * revert already-created outcomes. Each step therefore carries its outcome's
 * market PDA as `checkAccount`; the sequence sender probes it and SKIPS the step
 * when the market already exists (skip-if-exists resume), exactly like activate.
 *
 * The creator's KASS ATA is the seed-transfer source for every outcome, so an
 * idempotent create-ATA ix is prepended ONCE — on step 0 only — when that account
 * is absent (all later steps reuse the account step 0 created).
 */
import { flows } from "@kassandra-market/sdk";
import type { ActivateStep } from "./activate";
import { ensureKassAta, toAddress, type AddressInput } from "./ata";
import { ValidationError } from "../writeAction";
import type { IndexerClient } from "../../lib/indexer";

export interface BuildCreateAllArgs {
  indexer: IndexerClient;
  /** The categorical Kassandra oracle every sub-market resolves against. */
  oracle: AddressInput;
  /** The oracle's `options_count` — one create step is emitted per outcome. */
  optionsCount: number;
  /** Creator authority (the signer): pays rent + seeds each contribution. */
  creator: AddressInput;
  /** Canonical KASS mint (== `config.kass_mint`). */
  kassMint: AddressInput;
  /** KASS seeded into each outcome's escrow (raw base units, > 0); charged per outcome. */
  seedAmount: bigint;
}

/**
 * Build the ordered per-outcome create sequence for a categorical oracle. Derives
 * the creator's KASS ATA (the shared seed source) via {@link ensureKassAta} and
 * hands it to the SDK `createAllOutcomeMarkets` to get one `createMarket` ix per
 * outcome; wraps each in an {@link ActivateStep} whose `checkAccount` is that
 * outcome's market PDA (skip-if-exists). When the ATA is absent, its idempotent
 * create ix is prepended to STEP 0 only.
 */
export async function buildCreateAllSteps(args: BuildCreateAllArgs): Promise<ActivateStep[]> {
  const oracle = toAddress("Oracle", args.oracle);
  const kassMint = toAddress("KASS mint", args.kassMint);
  const creator = toAddress("Creator", args.creator);

  if (!Number.isInteger(args.optionsCount) || args.optionsCount < 1) {
    throw new ValidationError("Oracle must have at least one outcome.");
  }
  if (args.seedAmount <= 0n) {
    throw new ValidationError("Seed amount must be greater than zero.");
  }

  const { ata, createIx } = await ensureKassAta(args.indexer, creator, kassMint);

  const { steps } = await flows.createAllOutcomeMarkets({
    oracle,
    optionsCount: args.optionsCount,
    creator,
    kassMint,
    creatorKassAta: ata,
    seedAmount: args.seedAmount,
  });

  return steps.map((step, i) => ({
    label: `Outcome ${step.outcomeIndex}`,
    // Prepend the idempotent create-ATA ix once, on step 0 only (later outcomes
    // reuse the account step 0 created).
    ixs: i === 0 && createIx ? [createIx, step.instruction] : [step.instruction],
    checkAccount: step.market,
  }));
}

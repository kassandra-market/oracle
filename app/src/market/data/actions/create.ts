/**
 * The create-market write ACTION (pure ix-builder, NO React).
 *
 * {@link buildCreateMarketIxs} binds a NEW prediction sub-market (one oracle
 * outcome, `outcomeIndex`) to an EXISTING Kassandra oracle: YES = the oracle
 * resolves to that outcome (binary markets use `outcomeIndex = 0`). It derives
 * the creator's KASS ATA (the seed-transfer
 * source), PREPENDS an idempotent create-ATA ix when that account is absent, and
 * appends the SDK `createMarket` builder. Validation (oracle a valid address,
 * `seedAmount > 0`) throws a typed `ValidationError` the form surfaces inline.
 */
import { TransactionInstruction } from "@solana/web3.js";
import { createMarket } from "@kassandra-market/sdk";
import type { IndexerClient } from "../../lib/indexer";
import { ValidationError } from "../writeAction";
import { ensureKassAta, toAddress, type AddressInput } from "./ata";

export interface BuildCreateMarketArgs {
  indexer: IndexerClient;
  /** The Kassandra oracle the market resolves against (seeds the market PDA). */
  oracle: AddressInput;
  /** Canonical KASS mint (== `config.kass_mint`). */
  kassMint: AddressInput;
  /** Creator authority (the signer): pays rent + seeds the first contribution. */
  creator: AddressInput;
  /** KASS seeded into escrow as the creator's contribution (raw base units, > 0). */
  seedAmount: bigint;
  /**
   * The oracle outcome this sub-market binds to (`0 <= outcomeIndex <
   * oracle.options_count`). YES = the oracle resolves to this outcome. Binary
   * (2-option) oracles use `0`; a categorical oracle has one sub-market per
   * outcome. Defaults to `0`.
   */
  outcomeIndex?: number;
}

/**
 * Assemble the create-market instruction list: an optional idempotent create-ATA
 * (when the creator's KASS ATA is absent) followed by the `createMarket` ix.
 */
export async function buildCreateMarketIxs(
  args: BuildCreateMarketArgs,
): Promise<TransactionInstruction[]> {
  const oracle = toAddress("Oracle", args.oracle);
  const kassMint = toAddress("KASS mint", args.kassMint);
  const creator = toAddress("Creator", args.creator);

  if (args.seedAmount <= 0n) {
    throw new ValidationError("Seed amount must be greater than zero.");
  }
  const outcomeIndex = args.outcomeIndex ?? 0;
  if (!Number.isInteger(outcomeIndex) || outcomeIndex < 0) {
    throw new ValidationError("Outcome index must be a non-negative whole number.");
  }

  const { ata, createIx } = await ensureKassAta(args.indexer, creator, kassMint);

  const ix = await createMarket({
    creator,
    oracle,
    kassMint,
    creatorKassAta: ata,
    seedAmount: args.seedAmount,
    outcomeIndex,
  });

  return createIx ? [createIx, ix] : [ix];
}

/**
 * The contribute write ACTION (pure ix-builder, NO React).
 *
 * {@link buildContributeIxs} adds `amount` KASS to a Funding market's escrow and
 * creates-or-increments the caller's Contribution. It derives the contributor's
 * KASS ATA (the transfer source), PREPENDS an idempotent create-ATA ix when
 * absent, then appends the SDK `contribute` builder. `amount > 0` is validated
 * with a typed `ValidationError`.
 */
import { TransactionInstruction } from "@solana/web3.js";
import { contribute } from "@kassandra-market/markets";
import type { IndexerClient } from "../../lib/indexer";
import { ValidationError } from "../writeAction";
import { ensureKassAta, toAddress, type AddressInput } from "./ata";

export interface BuildContributeArgs {
  indexer: IndexerClient;
  /** The market being contributed to. */
  market: AddressInput;
  /** Canonical KASS mint (== `market.kass_mint`). */
  kassMint: AddressInput;
  /** Contributor authority (the signer). */
  contributor: AddressInput;
  /** KASS to stake (raw base units, > 0). */
  amount: bigint;
}

/**
 * Assemble the contribute instruction list: an optional idempotent create-ATA
 * (when the contributor's KASS ATA is absent) followed by the `contribute` ix.
 */
export async function buildContributeIxs(
  args: BuildContributeArgs,
): Promise<TransactionInstruction[]> {
  const market = toAddress("Market", args.market);
  const kassMint = toAddress("KASS mint", args.kassMint);
  const contributor = toAddress("Contributor", args.contributor);

  if (args.amount <= 0n) {
    throw new ValidationError("Amount must be greater than zero.");
  }

  const { ata, createIx } = await ensureKassAta(args.indexer, contributor, kassMint);

  const ix = await contribute({
    contributor,
    market,
    contributorKassAta: ata,
    amount: args.amount,
  });

  return createIx ? [createIx, ix] : [ix];
}

/**
 * The refund write ACTION (pure ix-builder, NO React).
 *
 * {@link buildRefundIxs} returns a contributor's stake from a Cancelled market.
 * It is PERMISSIONLESS; the refund destination is the contributor's KASS ATA, so
 * we derive it and PREPEND an idempotent create-ATA ix when absent (a contributor
 * whose ATA was since closed still gets paid), then append the SDK `refund` ix.
 */
import { TransactionInstruction } from "@solana/web3.js";
import { refund } from "@kassandra-market/sdk";
import type { IndexerClient } from "../../lib/indexer";
import { ensureKassAta, toAddress, type AddressInput } from "./ata";

export interface BuildRefundArgs {
  indexer: IndexerClient;
  /** The Cancelled market. */
  market: AddressInput;
  /** Canonical KASS mint (== `market.kass_mint`). */
  kassMint: AddressInput;
  /** The contributor being refunded (seeds the Contribution PDA + refund dest). */
  contributor: AddressInput;
}

/**
 * Assemble the refund instruction list: an optional idempotent create-ATA (when
 * the contributor's KASS ATA is absent) followed by the `refund` ix.
 */
export async function buildRefundIxs(
  args: BuildRefundArgs,
): Promise<TransactionInstruction[]> {
  const market = toAddress("Market", args.market);
  const kassMint = toAddress("KASS mint", args.kassMint);
  const contributor = toAddress("Contributor", args.contributor);

  const { ata, createIx } = await ensureKassAta(args.indexer, contributor, kassMint);

  const ix = await refund({
    market,
    contributor,
    contributorKassAta: ata,
  });

  return createIx ? [createIx, ix] : [ix];
}

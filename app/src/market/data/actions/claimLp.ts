/**
 * The claim-LP write ACTION (pure ix-builder, NO React).
 *
 * After a market activates, each contributor may permissionlessly claim their
 * pro-rata share of the AMM LP tokens the escrow seeded. {@link buildClaimLpIxs}
 * derives the contributor's LP ATA (mint == `market.lpMint`), PREPENDS an
 * idempotent create-ATA ix when it's absent (the `claim_lp` account list has no
 * ATA/System program, so the destination must pre-exist), then appends the SDK
 * `claimLp` builder.
 */
import { TransactionInstruction } from "@solana/web3.js";
import { claimLp } from "@kassandra-market/sdk";
import type { IndexerClient } from "../../lib/indexer";
import { ensureAta, toAddress, type AddressInput } from "./ata";

export interface BuildClaimLpArgs {
  indexer: IndexerClient;
  /** The Active/Resolved/Void market being claimed against. */
  market: AddressInput;
  /** The contributor claiming (seeds the Contribution PDA + LP destination). */
  contributor: AddressInput;
  /** The AMM pool's LP mint (== `market.lpMint`). */
  lpMint: AddressInput;
}

/**
 * Assemble the claim-LP instruction list: an optional idempotent create-ATA
 * (contributor's LP ATA) followed by the `claimLp` ix.
 */
export async function buildClaimLpIxs(args: BuildClaimLpArgs): Promise<TransactionInstruction[]> {
  const market = toAddress("Market", args.market);
  const contributor = toAddress("Contributor", args.contributor);
  const lpMint = toAddress("LP mint", args.lpMint);

  const { ata, createIx } = await ensureAta(args.indexer, contributor, lpMint);
  const ix = await claimLp({ market, contributor, contributorLpAta: ata });
  return createIx ? [createIx, ix] : [ix];
}

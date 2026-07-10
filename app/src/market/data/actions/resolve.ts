/**
 * The resolve-market write ACTION (pure ix-builder, NO React).
 *
 * A permissionless, idempotent crank that bridges the terminal Kassandra oracle
 * result into the market's MetaDAO `resolve_question` (stamping the payout
 * numerators redeem reads). {@link buildResolveIxs} moves no tokens, so there is
 * no ATA — just the single SDK `resolveMarket` ix, wired with the conditional-
 * vault `#[event_cpi]` event authority (derived under the vault program).
 */
import { TransactionInstruction } from "@solana/web3.js";
import { metadao, resolveMarket } from "@kassandra-market/markets";
import { toAddress, type AddressInput } from "./ata";

export interface BuildResolveArgs {
  /** The market to resolve (also the CPI signer via seeds). */
  market: AddressInput;
  /** The market's Kassandra oracle (must be `Resolved`). */
  oracle: AddressInput;
  /** The market's MetaDAO Question (== `market.question`). */
  question: AddressInput;
}

/** Assemble the (single-ix) resolve-market instruction list. */
export async function buildResolveIxs(args: BuildResolveArgs): Promise<TransactionInstruction[]> {
  const market = toAddress("Market", args.market);
  const oracle = toAddress("Oracle", args.oracle);
  const question = toAddress("Question", args.question);

  // conditional_vault `#[event_cpi]` authority — `eventAuthority(CONDITIONAL_VAULT_ID)`.
  const cvEventAuthority = (await metadao.pda.vaultEventAuthority()).address;
  const ix = await resolveMarket({ market, oracle, question, cvEventAuthority });
  return [ix];
}

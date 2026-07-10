/**
 * High-level redeem flow — after a market resolves, a holder burns their full
 * cYES + cNO balances and receives the resolved KASS payout.
 *
 * The MetaDAO `redeem_tokens` pays each conditional token by the question's
 * resolved numerators (`[1,0]` ⇒ cYES pays 1:1 and cNO pays 0, etc.), so a YES
 * winner who holds only cYES gets their stake back, and worthless legs pay 0. The
 * caller must first crank `resolveMarket` so the question carries a denominator.
 *
 * User conditional-token accounts default to the canonical ATAs (from the vault's
 * `yesMint`/`noMint`) but may be overridden (e.g. fabricated test accounts).
 */
import { Address, type TransactionInstruction } from "@solana/web3.js";

import * as metadao from "../metadao/index.js";
import type { AddressInput } from "../pda.js";
import type { MarketRefs } from "./compose.js";
import { toAddr } from "./util.js";

export interface RedeemParams {
  /** Composed market refs (from `composeMarketInstructions`). */
  refs: MarketRefs;
  /** Holder + signer (owns the conditional accounts). */
  user: AddressInput;
  /** Holder's KASS token account the payout lands in. */
  userKassAta: AddressInput;
  /** Holder's cYES account (defaults to the ATA on `refs.yesMint`). */
  userYesAta?: AddressInput;
  /** Holder's cNO account (defaults to the ATA on `refs.noMint`). */
  userNoAta?: AddressInput;
}

/**
 * Build the single `redeem_tokens` instruction. Returns it plus the resolved
 * user conditional ATAs. The question must already be resolved (`resolveMarket`).
 *
 * PRECONDITION: the holder's cYES/cNO accounts AND the destination KASS account
 * must already exist — `redeem_tokens` cannot create them. For a fresh wallet
 * prepend {@link ensureConditionalAtasInstructions} with `includeKass: true`.
 */
export async function redeemInstructions(
  params: RedeemParams,
): Promise<{ instructions: TransactionInstruction[]; userYesAta: Address; userNoAta: Address }> {
  const { refs, user } = params;
  const yes = params.userYesAta ? toAddr(params.userYesAta) : await metadao.pda.ata(user, refs.yesMint);
  const no = params.userNoAta ? toAddr(params.userNoAta) : await metadao.pda.ata(user, refs.noMint);

  const redeem = await metadao.redeemTokens({
    question: refs.question,
    vault: refs.vault,
    vaultUnderlyingAta: refs.vaultUnderlyingAta,
    authority: user,
    userUnderlyingAta: params.userKassAta,
    conditionalMints: [refs.yesMint, refs.noMint],
    userConditionalAtas: [yes, no],
  });

  return { instructions: [redeem], userYesAta: yes, userNoAta: no };
}

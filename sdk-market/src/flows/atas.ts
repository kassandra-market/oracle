/**
 * ATA-creation helper for the trade / redeem flows.
 *
 * The MetaDAO `InteractWithVault` account list (split / merge / redeem) and the
 * AMM `swap` list carry NO Associated-Token-Account or System program, so they
 * CANNOT create the user's cYES/cNO (or the redeem KASS) token accounts — those
 * must already exist. For a fresh wallet an app must prepend account creation.
 *
 * {@link ensureConditionalAtasInstructions} returns idempotent
 * `createAssociatedTokenAccountIdempotent` instructions (ATA program discriminant
 * `1`) for the user's cYES + cNO (and optionally the KASS underlying) ATAs. They
 * are safe to prepend unconditionally — an already-existing ATA is a no-op.
 */
import { Address, TransactionInstruction } from "@solana/web3.js";
import type { AccountMeta } from "@solana/web3.js";

import { ATA_PROGRAM_ID, SYSTEM_PROGRAM_ID, TOKEN_PROGRAM_ID } from "../constants.js";
import * as pda from "../pda.js";
import type { AddressInput } from "../pda.js";
import type { MarketRefs } from "./compose.js";
import { toAddr } from "./util.js";

function ro(pubkey: Address, isSigner = false): AccountMeta {
  return { pubkey, isSigner, isWritable: false };
}
function w(pubkey: Address, isSigner = false): AccountMeta {
  return { pubkey, isSigner, isWritable: true };
}

/**
 * A single `createAssociatedTokenAccountIdempotent` instruction (ATA program disc
 * `1`). Accounts: `payer(signer,w) ata(w) owner(ro) mint(ro) system tokenProgram`.
 * The byte-identical leaf shared by the SDK flows and the app action builders.
 */
export function createAtaIdempotentInstruction(
  payer: Address,
  ata: Address,
  owner: Address,
  mint: Address,
): TransactionInstruction {
  return new TransactionInstruction({
    programId: ATA_PROGRAM_ID,
    keys: [w(payer, true), w(ata), ro(owner), ro(mint), ro(SYSTEM_PROGRAM_ID), ro(TOKEN_PROGRAM_ID)],
    data: Uint8Array.from([1]),
  });
}

/**
 * {@link createAtaIdempotentInstruction} for the derived `ATA(owner, mint)`, plus
 * that address. Returns the ix and the ATA it creates.
 */
async function createIdempotentAta(
  payer: Address,
  owner: Address,
  mint: Address,
): Promise<{ ix: TransactionInstruction; ata: Address }> {
  const ata = (await pda.associatedTokenAccount(owner, mint)).address;
  return { ix: createAtaIdempotentInstruction(payer, ata, owner, mint), ata };
}

export interface EnsureAtasParams {
  /** Composed market refs (carry the cYES/cNO + KASS mints). */
  refs: MarketRefs;
  /** The wallet that will own (and here, pay for) the ATAs. */
  user: AddressInput;
  /** Rent payer (defaults to `user`). */
  payer?: AddressInput;
  /** Also create the user's KASS underlying ATA (needed by redeem). Default false. */
  includeKass?: boolean;
}

/**
 * Idempotent `createAssociatedTokenAccountIdempotent` instructions for the user's
 * cYES + cNO ATAs (and the KASS ATA when `includeKass`). Prepend these to a
 * `buyInstructions` / `redeemInstructions` list for a wallet whose ATAs may not
 * exist yet. Returns the instructions plus the derived ATA addresses.
 */
export async function ensureConditionalAtasInstructions(params: EnsureAtasParams): Promise<{
  instructions: TransactionInstruction[];
  userYesAta: Address;
  userNoAta: Address;
  userKassAta?: Address;
}> {
  const owner = toAddr(params.user);
  const payer = params.payer ? toAddr(params.payer) : owner;

  const yes = await createIdempotentAta(payer, owner, toAddr(params.refs.yesMint));
  const no = await createIdempotentAta(payer, owner, toAddr(params.refs.noMint));
  const instructions = [yes.ix, no.ix];
  let userKassAta: Address | undefined;
  if (params.includeKass) {
    const kass = await createIdempotentAta(payer, owner, toAddr(params.refs.kassMint));
    instructions.push(kass.ix);
    userKassAta = kass.ata;
  }

  return { instructions, userYesAta: yes.ata, userNoAta: no.ata, userKassAta };
}

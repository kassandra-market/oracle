/**
 * Instruction builders for the MetaDAO `amm` v0.4.2 (delayed-twap standalone AMM).
 *
 * Each builder returns a web3.js (classic) `TransactionInstruction` whose
 * `data == [disc, ...borsh_args]` and whose `keys` are the EXACT account-meta
 * order pinned in `sdk-rs/src/metadao.rs` (the wire-format source of truth).
 * Every instruction is Anchor `#[event_cpi]`: the two trailing accounts
 * (event-authority PDA, program id) are appended by the builders.
 */
import { TransactionInstruction } from "@solana/web3.js";

import { SYSTEM_PROGRAM_ID, TOKEN_PROGRAM_ID } from "../constants.js";
import type { AddressInput } from "../pda.js";
import { AMM_V04_ID, ATA_PROGRAM_ID, DISC, SwapType } from "./constants.js";
import * as pda from "./pda.js";
import { addr, concat, ro, u8b, u64le, u128le, w } from "./util.js";

// ── create_amm ────────────────────────────────────────────────────────────────

export interface CreateAmmArgs {
  /** Rent payer + signer. */
  payer: AddressInput;
  /** Base-token mint (kassandra-market: the cYES conditional mint). */
  baseMint: AddressInput;
  /** Quote-token mint (kassandra-market: the cNO conditional mint). */
  quoteMint: AddressInput;
  /** `twap_initial_observation` (u128). */
  twapInitialObservation: bigint | number;
  /** `twap_max_observation_change_per_update` (u128). */
  twapMaxObservationChangePerUpdate: bigint | number;
  /** `twap_start_delay_slots` (u64) — the v0.4.1+ delayed-twap field (default 0). */
  twapStartDelaySlots?: bigint | number;
}

/**
 * `amm::create_amm` — 12 accounts. Data (40 bytes):
 * `disc[8] ++ twap_initial_observation:u128 ++
 * twap_max_observation_change_per_update:u128 ++ twap_start_delay_slots:u64`.
 */
export async function createAmm(a: CreateAmmArgs): Promise<TransactionInstruction> {
  const ammAddr = (await pda.amm(a.baseMint, a.quoteMint)).address;
  const lpMint = (await pda.ammLpMint(ammAddr)).address;
  const vaultBase = await pda.ata(ammAddr, a.baseMint);
  const vaultQuote = await pda.ata(ammAddr, a.quoteMint);
  const eventAuthority = (await pda.ammEventAuthority()).address;
  return new TransactionInstruction({
    programId: AMM_V04_ID,
    keys: [
      w(a.payer, true),
      w(ammAddr),
      w(lpMint),
      ro(a.baseMint),
      ro(a.quoteMint),
      w(vaultBase),
      w(vaultQuote),
      ro(ATA_PROGRAM_ID),
      ro(TOKEN_PROGRAM_ID),
      ro(SYSTEM_PROGRAM_ID),
      ro(eventAuthority),
      ro(AMM_V04_ID),
    ],
    data: concat([
      DISC.createAmm,
      u128le(a.twapInitialObservation),
      u128le(a.twapMaxObservationChangePerUpdate),
      u64le(a.twapStartDelaySlots ?? 0),
    ]),
  });
}

// ── add_liquidity ─────────────────────────────────────────────────────────────

export interface AddLiquidityArgs {
  /** Liquidity provider + signer (owns the user token accounts). */
  payer: AddressInput;
  baseMint: AddressInput;
  quoteMint: AddressInput;
  /** `quote_amount` (u64). */
  quoteAmount: bigint | number;
  /** `max_base_amount` (u64). */
  maxBaseAmount: bigint | number;
  /** `min_lp_tokens` (u64, default 0). */
  minLpTokens?: bigint | number;
}

/**
 * `amm::add_liquidity` — 11 accounts. Data:
 * `disc[8] ++ quote_amount:u64 ++ max_base_amount:u64 ++ min_lp_tokens:u64`.
 * The user base/quote/lp accounts are the payer's ATAs.
 */
export async function addLiquidity(a: AddLiquidityArgs): Promise<TransactionInstruction> {
  const ammAddr = (await pda.amm(a.baseMint, a.quoteMint)).address;
  const lpMint = (await pda.ammLpMint(ammAddr)).address;
  const userLp = await pda.ata(a.payer, lpMint);
  const userBase = await pda.ata(a.payer, a.baseMint);
  const userQuote = await pda.ata(a.payer, a.quoteMint);
  const vaultBase = await pda.ata(ammAddr, a.baseMint);
  const vaultQuote = await pda.ata(ammAddr, a.quoteMint);
  const eventAuthority = (await pda.ammEventAuthority()).address;
  return new TransactionInstruction({
    programId: AMM_V04_ID,
    keys: [
      w(a.payer, true),
      w(ammAddr),
      w(lpMint),
      w(userLp),
      w(userBase),
      w(userQuote),
      w(vaultBase),
      w(vaultQuote),
      ro(TOKEN_PROGRAM_ID),
      ro(eventAuthority),
      ro(AMM_V04_ID),
    ],
    data: concat([
      DISC.addLiquidity,
      u64le(a.quoteAmount),
      u64le(a.maxBaseAmount),
      u64le(a.minLpTokens ?? 0),
    ]),
  });
}

// ── swap ──────────────────────────────────────────────────────────────────────

export interface SwapArgs {
  /** Trader + signer (owns the user token accounts). */
  payer: AddressInput;
  baseMint: AddressInput;
  quoteMint: AddressInput;
  /** `swap_type` — `Buy` (quote→base) or `Sell` (base→quote). */
  swapType: SwapType;
  /** `input_amount` (u64). */
  inputAmount: bigint | number;
  /** `output_amount_min` (u64, default 0). */
  outputAmountMin?: bigint | number;
  /**
   * Trader's base-token account (defaults to `ata(payer, baseMint)`). Override to
   * keep split/swap/merge pointing at the SAME account when they are not ATAs.
   */
  userBase?: AddressInput;
  /** Trader's quote-token account (defaults to `ata(payer, quoteMint)`). */
  userQuote?: AddressInput;
}

/**
 * `amm::swap` — 9 accounts. Data:
 * `disc[8] ++ swap_type:u8 ++ input_amount:u64 ++ output_amount_min:u64`.
 */
export async function swap(a: SwapArgs): Promise<TransactionInstruction> {
  const ammAddr = (await pda.amm(a.baseMint, a.quoteMint)).address;
  const userBase = a.userBase ? addr(a.userBase) : await pda.ata(a.payer, a.baseMint);
  const userQuote = a.userQuote ? addr(a.userQuote) : await pda.ata(a.payer, a.quoteMint);
  const vaultBase = await pda.ata(ammAddr, a.baseMint);
  const vaultQuote = await pda.ata(ammAddr, a.quoteMint);
  const eventAuthority = (await pda.ammEventAuthority()).address;
  return new TransactionInstruction({
    programId: AMM_V04_ID,
    keys: [
      w(a.payer, true),
      w(ammAddr),
      w(userBase),
      w(userQuote),
      w(vaultBase),
      w(vaultQuote),
      ro(TOKEN_PROGRAM_ID),
      ro(eventAuthority),
      ro(AMM_V04_ID),
    ],
    data: concat([
      DISC.swap,
      u8b(a.swapType),
      u64le(a.inputAmount),
      u64le(a.outputAmountMin ?? 0),
    ]),
  });
}

// ── crank_that_twap ───────────────────────────────────────────────────────────

export interface CrankThatTwapArgs {
  /** The `Amm` PDA whose TWAP observation to fold. */
  amm: AddressInput;
}

/**
 * `amm::crank_that_twap` — 3 accounts `[amm(w), event_authority, amm_program]`.
 * No args (discriminator only).
 */
export async function crankThatTwap(a: CrankThatTwapArgs): Promise<TransactionInstruction> {
  const eventAuthority = (await pda.ammEventAuthority()).address;
  return new TransactionInstruction({
    programId: AMM_V04_ID,
    keys: [w(a.amm), ro(eventAuthority), ro(AMM_V04_ID)],
    data: DISC.crankThatTwap,
  });
}

/**
 * Instruction builders for the MetaDAO **v0.4 standalone AMM** (`AMMyu265…`).
 *
 * Each builder returns a web3.js (classic) `TransactionInstruction` whose
 * `data == [disc, ...borsh_args]` and whose `keys` are the EXACT account-meta
 * order proven against the real `metadao_amm.so` in
 * `programs/oracles/tests/challenge_e2e.rs:676-769`. Discriminators + arg
 * layouts are the binary-validated values from
 * `programs/oracles/src/cpi/metadao.rs:82-94`.
 *
 * Every AMM instruction is Anchor `#[event_cpi]`: the two trailing accounts
 * (event_authority PDA, program id) are appended by the builders.
 *
 * NOTE: this is the v0.4 STANDALONE AMM — a different program from the v0.6
 * futarchy embedded AMM (`src/futarchy`). The two are NOT interchangeable.
 */
import { Address, TransactionInstruction } from "@solana/web3.js";
import type { AccountMeta } from "@solana/web3.js";

import {
  concatBytes as concat,
  u128LE as u128le,
  u64LE as u64le,
  u8 as u8b,
} from "../bytes.js";
import { SYSTEM_PROGRAM_ID, TOKEN_PROGRAM_ID } from "../constants.js";
import type { AddressInput } from "../pda.js";
import { AMM_V04_ID, ATA_PROGRAM_ID, DISC, SwapType } from "./constants.js";
import * as apda from "./pda.js";

// ── meta + borsh helpers ─────────────────────────────────────────────────────

function addr(a: AddressInput): Address {
  return a instanceof Address ? a : new Address(a);
}
function w(pubkey: AddressInput, isSigner = false): AccountMeta {
  return { pubkey: addr(pubkey), isSigner, isWritable: true };
}
function ro(pubkey: AddressInput, isSigner = false): AccountMeta {
  return { pubkey: addr(pubkey), isSigner, isWritable: false };
}


// ════════════════════════════════════════════════════════════════════════════
// create_amm
// ════════════════════════════════════════════════════════════════════════════

export interface CreateAmmArgs {
  /** Rent payer + signer. */
  payer: AddressInput;
  /** Base-token mint (the conditional-KASS mint in the challenge flow). */
  baseMint: AddressInput;
  /** Quote-token mint (the conditional-USDC mint). */
  quoteMint: AddressInput;
  /** `CreateAmmArgs.twap_initial_observation` (u128) — `quote·1e12/base` in the e2e. */
  twapInitialObservation: bigint | number;
  /** `CreateAmmArgs.twap_max_observation_change_per_update` (u128). */
  twapMaxObservationChangePerUpdate: bigint | number;
  /** `CreateAmmArgs.twap_start_delay_slots` (u64) — the v0.4.1+ delayed-twap field. */
  twapStartDelaySlots?: bigint | number;
}

/**
 * `amm::create_amm` — creates the `Amm` PDA, its LP mint, and the two vault ATAs.
 *
 * Args (`metadao.rs:78-82`, Borsh, 40 bytes): `twap_initial_observation: u128 ++
 * twap_max_observation_change_per_update: u128 ++ twap_start_delay_slots: u64`.
 * Accounts (`challenge_e2e.rs:676-689`).
 */
export async function createAmm(a: CreateAmmArgs): Promise<TransactionInstruction> {
  const ammAddr = (await apda.amm(a.baseMint, a.quoteMint)).address;
  const lp = (await apda.lpMint(ammAddr)).address;
  const vaultBase = await apda.ata(ammAddr, a.baseMint);
  const vaultQuote = await apda.ata(ammAddr, a.quoteMint);
  const eventAuthority = (await apda.eventAuthority()).address;
  return new TransactionInstruction({
    programId: addr(AMM_V04_ID),
    keys: [
      w(a.payer, true),
      w(ammAddr),
      w(lp),
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

// ════════════════════════════════════════════════════════════════════════════
// add_liquidity
// ════════════════════════════════════════════════════════════════════════════

export interface AddLiquidityArgs {
  /** Liquidity provider + signer (owns the user token accounts). */
  payer: AddressInput;
  baseMint: AddressInput;
  quoteMint: AddressInput;
  /** `AddLiquidityArgs.quote_amount` (u64). */
  quoteAmount: bigint | number;
  /** `AddLiquidityArgs.max_base_amount` (u64). */
  maxBaseAmount: bigint | number;
  /** `AddLiquidityArgs.min_lp_tokens` (u64). */
  minLpTokens?: bigint | number;
}

/**
 * `amm::add_liquidity` — deposits base+quote, mints LP to the payer's LP ATA.
 *
 * Args (`metadao.rs:83-85`): `quote_amount: u64 ++ max_base_amount: u64 ++
 * min_lp_tokens: u64`. Accounts (`challenge_e2e.rs:703-714`). The user base/quote
 * /lp accounts are the payer's ATAs.
 */
export async function addLiquidity(a: AddLiquidityArgs): Promise<TransactionInstruction> {
  const ammAddr = (await apda.amm(a.baseMint, a.quoteMint)).address;
  const lp = (await apda.lpMint(ammAddr)).address;
  const userLp = await apda.ata(a.payer, lp);
  const userBase = await apda.ata(a.payer, a.baseMint);
  const userQuote = await apda.ata(a.payer, a.quoteMint);
  const vaultBase = await apda.ata(ammAddr, a.baseMint);
  const vaultQuote = await apda.ata(ammAddr, a.quoteMint);
  const eventAuthority = (await apda.eventAuthority()).address;
  return new TransactionInstruction({
    programId: addr(AMM_V04_ID),
    keys: [
      w(a.payer, true),
      w(ammAddr),
      w(lp),
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

// ════════════════════════════════════════════════════════════════════════════
// swap
// ════════════════════════════════════════════════════════════════════════════

export interface SwapArgs {
  /** Trader + signer (owns the user token accounts). */
  payer: AddressInput;
  baseMint: AddressInput;
  quoteMint: AddressInput;
  /** `SwapArgs.swap_type` — `Buy` (quote→base) or `Sell` (base→quote). */
  swapType: SwapType;
  /** `SwapArgs.input_amount` (u64). */
  inputAmount: bigint | number;
  /** `SwapArgs.output_amount_min` (u64). */
  minOutputAmount?: bigint | number;
}

/**
 * `amm::swap` — trades against the pool, folding the new price into the oracle.
 *
 * Args (`metadao.rs:88-90`): `swap_type: u8 (0=Buy,1=Sell) ++ input_amount: u64
 * ++ output_amount_min: u64`. Accounts (`challenge_e2e.rs:736-749`).
 */
export async function swap(a: SwapArgs): Promise<TransactionInstruction> {
  const ammAddr = (await apda.amm(a.baseMint, a.quoteMint)).address;
  const userBase = await apda.ata(a.payer, a.baseMint);
  const userQuote = await apda.ata(a.payer, a.quoteMint);
  const vaultBase = await apda.ata(ammAddr, a.baseMint);
  const vaultQuote = await apda.ata(ammAddr, a.quoteMint);
  const eventAuthority = (await apda.eventAuthority()).address;
  return new TransactionInstruction({
    programId: addr(AMM_V04_ID),
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
      u64le(a.minOutputAmount ?? 0),
    ]),
  });
}

// ════════════════════════════════════════════════════════════════════════════
// crank_that_twap
// ════════════════════════════════════════════════════════════════════════════

export interface CrankThatTwapArgs {
  /** The `Amm` PDA whose TWAP observation to fold. */
  amm: AddressInput;
}

/**
 * `amm::crank_that_twap` — folds the current price into the TWAP observation
 * (only once per `ONE_MINUTE_IN_SLOTS == 150` slots). No args.
 *
 * Accounts (`metadao.rs:93`, `challenge_e2e.rs:760-769`): `[amm(w),
 * event_authority, amm_program]`.
 */
export async function crankThatTwap(a: CrankThatTwapArgs): Promise<TransactionInstruction> {
  const eventAuthority = (await apda.eventAuthority()).address;
  return new TransactionInstruction({
    programId: addr(AMM_V04_ID),
    keys: [w(a.amm), ro(eventAuthority), ro(AMM_V04_ID)],
    data: DISC.crankThatTwap,
  });
}

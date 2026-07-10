/**
 * Instruction builders for Meteora **DAMM v2** (cp-amm) — the position-based
 * spot-path lifecycle: `initializePool`, `createPosition`, `addLiquidity`,
 * `removeLiquidity`, `swap`, `claimPositionFee`.
 *
 * Each returns a web3.js (classic) `TransactionInstruction` whose
 * `data == [disc, ...borsh_args]` (LE) and whose `keys` are the EXACT account-meta
 * order/roles from the pinned `#[derive(Accounts)]` structs
 * (commit `bdd8a1e355f484b3cff131578a662c560b97b72f`,
 * `programs/cp-amm/src/instructions/…`). Every instruction is Anchor
 * `#[event_cpi]`, so the two trailing accounts (event_authority PDA, program id)
 * are appended by the builders.
 *
 * KEY cp-amm specifics (differ from the MetaDAO AMMs):
 *  - POSITION-based: `initializePool` ALSO mints the first position NFT (it takes
 *    `liquidity` + `sqrt_price` directly); `createPosition` opens an empty one.
 *  - `swap` has NO direction/`swap_type` arg — the trade direction is implicit in
 *    which token account is `inputTokenAccount` vs `outputTokenAccount`. Args are
 *    just `amount_in: u64 ++ minimum_amount_out: u64` (SwapParameters).
 *  - the position NFT mint + its token account live under Token-2022.
 *  - the `Pool` PDA is keyed by a `config` account + the SORTED mint pair.
 */
import { Address, TransactionInstruction } from "@solana/web3.js";
import type { AccountMeta } from "@solana/web3.js";

import { SYSTEM_PROGRAM_ID, TOKEN_PROGRAM_ID } from "../constants.js";
import type { AddressInput } from "../pda.js";
import { DISC, METEORA_DAMM_V2_ID, TOKEN_2022_PROGRAM_ID } from "./constants.js";
import * as mpda from "./pda.js";

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

function u8b(v: number): Uint8Array {
  return Uint8Array.from([v & 0xff]);
}
function u64le(v: bigint | number): Uint8Array {
  const o = new Uint8Array(8);
  new DataView(o.buffer).setBigUint64(0, BigInt(v), true);
  return o;
}
function u128le(v: bigint | number): Uint8Array {
  const o = new Uint8Array(16);
  const dv = new DataView(o.buffer);
  const x = BigInt(v);
  dv.setBigUint64(0, x & 0xffffffffffffffffn, true);
  dv.setBigUint64(8, x >> 64n, true);
  return o;
}
/** Borsh `Option<u64>`: `0x00` (None) or `0x01 ++ u64le` (Some). */
function optionU64le(v: bigint | number | null | undefined): Uint8Array {
  if (v === null || v === undefined) return Uint8Array.from([0]);
  return concat([Uint8Array.from([1]), u64le(v)]);
}
function concat(parts: Array<Uint8Array>): Uint8Array {
  const total = parts.reduce((n, p) => n + p.length, 0);
  const out = new Uint8Array(total);
  let off = 0;
  for (const p of parts) {
    out.set(p, off);
    off += p.length;
  }
  return out;
}

// ════════════════════════════════════════════════════════════════════════════
// initialize_pool  (ix_initialize_pool.rs)
// ════════════════════════════════════════════════════════════════════════════

export interface InitializePoolArgs {
  /** Pool creator (recorded as `Pool.creator`; UncheckedAccount, not a signer). */
  creator: AddressInput;
  /** Fee/rent payer + signer. */
  payer: AddressInput;
  /** New position-NFT mint — a fresh keypair the caller must also sign with. */
  positionNftMint: AddressInput;
  /** The `Config` account the pool belongs to (fee/price-range params). */
  config: AddressInput;
  /** Token A mint. */
  tokenAMint: AddressInput;
  /** Token B mint. */
  tokenBMint: AddressInput;
  /** Payer's token-A source account. */
  payerTokenA: AddressInput;
  /** Payer's token-B source account. */
  payerTokenB: AddressInput;
  /** `InitializePoolParameters.liquidity` (u128). */
  liquidity: bigint | number;
  /** `InitializePoolParameters.sqrt_price` (u128, Q64.64). */
  sqrtPrice: bigint | number;
  /** `InitializePoolParameters.activation_point` (Option<u64>). Omit → None. */
  activationPoint?: bigint | number | null;
  /** Token program owning mint A (SPL Token by default). */
  tokenAProgram?: AddressInput;
  /** Token program owning mint B (SPL Token by default). */
  tokenBProgram?: AddressInput;
}

/**
 * `cp_amm::initialize_pool` — creates the Pool, its two token vaults, and the
 * FIRST position (minting a Token-2022 position NFT to `creator`), then deposits
 * `liquidity` at `sqrt_price`.
 *
 * Args (ix_initialize_pool.rs:39-47): `liquidity: u128 ++ sqrt_price: u128 ++
 * activation_point: Option<u64>`. Accounts (InitializePoolCtx, lines 51-185) +
 * `#[event_cpi]` trailer.
 */
export async function initializePool(a: InitializePoolArgs): Promise<TransactionInstruction> {
  const tokenAProgram = a.tokenAProgram ?? TOKEN_PROGRAM_ID;
  const tokenBProgram = a.tokenBProgram ?? TOKEN_PROGRAM_ID;
  const poolAddr = (await mpda.pool(a.config, a.tokenAMint, a.tokenBMint)).address;
  const positionAddr = (await mpda.position(a.positionNftMint)).address;
  const positionNftAccount = (await mpda.positionNftAccount(a.positionNftMint)).address;
  const tokenAVault = (await mpda.tokenVault(a.tokenAMint, poolAddr)).address;
  const tokenBVault = (await mpda.tokenVault(a.tokenBMint, poolAddr)).address;
  const poolAuthority = (await mpda.poolAuthority()).address;
  const eventAuthority = (await mpda.eventAuthority()).address;
  return new TransactionInstruction({
    programId: addr(METEORA_DAMM_V2_ID),
    keys: [
      ro(a.creator),
      w(a.positionNftMint, true),
      w(positionNftAccount),
      w(a.payer, true),
      ro(a.config),
      ro(poolAuthority),
      w(poolAddr),
      w(positionAddr),
      ro(a.tokenAMint),
      ro(a.tokenBMint),
      w(tokenAVault),
      w(tokenBVault),
      w(a.payerTokenA),
      w(a.payerTokenB),
      ro(tokenAProgram),
      ro(tokenBProgram),
      ro(TOKEN_2022_PROGRAM_ID),
      ro(SYSTEM_PROGRAM_ID),
      ro(eventAuthority),
      ro(METEORA_DAMM_V2_ID),
    ],
    data: concat([
      DISC.initializePool,
      u128le(a.liquidity),
      u128le(a.sqrtPrice),
      optionU64le(a.activationPoint),
    ]),
  });
}

// ════════════════════════════════════════════════════════════════════════════
// create_position  (ix_create_position.rs)
// ════════════════════════════════════════════════════════════════════════════

export interface CreatePositionArgs {
  /** Position-NFT recipient (UncheckedAccount, not a signer). */
  owner: AddressInput;
  /** New position-NFT mint — a fresh keypair the caller must also sign with. */
  positionNftMint: AddressInput;
  /** The Pool to open a position in. */
  pool: AddressInput;
  /** Fee/rent payer + signer. */
  payer: AddressInput;
}

/**
 * `cp_amm::create_position` — opens an EMPTY position (zero liquidity), minting a
 * Token-2022 position NFT to `owner`. No args.
 *
 * Accounts (CreatePositionCtx, ix_create_position.rs:411-469) + `#[event_cpi]`.
 */
export async function createPosition(a: CreatePositionArgs): Promise<TransactionInstruction> {
  const positionAddr = (await mpda.position(a.positionNftMint)).address;
  const positionNftAccount = (await mpda.positionNftAccount(a.positionNftMint)).address;
  const poolAuthority = (await mpda.poolAuthority()).address;
  const eventAuthority = (await mpda.eventAuthority()).address;
  return new TransactionInstruction({
    programId: addr(METEORA_DAMM_V2_ID),
    keys: [
      ro(a.owner),
      w(a.positionNftMint, true),
      w(positionNftAccount),
      w(a.pool),
      w(positionAddr),
      ro(poolAuthority),
      w(a.payer, true),
      ro(TOKEN_2022_PROGRAM_ID),
      ro(SYSTEM_PROGRAM_ID),
      ro(eventAuthority),
      ro(METEORA_DAMM_V2_ID),
    ],
    data: DISC.createPosition,
  });
}

// ════════════════════════════════════════════════════════════════════════════
// add_liquidity  (ix_add_liquidity.rs)
// ════════════════════════════════════════════════════════════════════════════

export interface ModifyLiquidityArgs {
  pool: AddressInput;
  position: AddressInput;
  /** Signer's token-A account. */
  tokenAAccount: AddressInput;
  /** Signer's token-B account. */
  tokenBAccount: AddressInput;
  tokenAVault: AddressInput;
  tokenBVault: AddressInput;
  tokenAMint: AddressInput;
  tokenBMint: AddressInput;
  /** Token account holding the position NFT (proves ownership; amount == 1). */
  positionNftAccount: AddressInput;
  /** Position owner / delegate + signer. */
  signer: AddressInput;
  /** `liquidity_delta` (u128). */
  liquidityDelta: bigint | number;
  /** Token-A threshold (u64): MAX to spend on add, MIN to receive on remove. */
  tokenAAmountThreshold: bigint | number;
  /** Token-B threshold (u64): MAX to spend on add, MIN to receive on remove. */
  tokenBAmountThreshold: bigint | number;
  tokenAProgram?: AddressInput;
  tokenBProgram?: AddressInput;
}

/**
 * `cp_amm::add_liquidity` — deposits into an existing position.
 *
 * Args (AddLiquidityParameters, ix_add_liquidity.rs:575-583): `liquidity_delta:
 * u128 ++ token_a_amount_threshold: u64 ++ token_b_amount_threshold: u64` (the
 * thresholds are the MAX amounts to spend). Accounts (AddLiquidityCtx, lines
 * 587-634) + `#[event_cpi]`.
 */
export async function addLiquidity(a: ModifyLiquidityArgs): Promise<TransactionInstruction> {
  const tokenAProgram = a.tokenAProgram ?? TOKEN_PROGRAM_ID;
  const tokenBProgram = a.tokenBProgram ?? TOKEN_PROGRAM_ID;
  const eventAuthority = (await mpda.eventAuthority()).address;
  return new TransactionInstruction({
    programId: addr(METEORA_DAMM_V2_ID),
    keys: [
      w(a.pool),
      w(a.position),
      w(a.tokenAAccount),
      w(a.tokenBAccount),
      w(a.tokenAVault),
      w(a.tokenBVault),
      ro(a.tokenAMint),
      ro(a.tokenBMint),
      ro(a.positionNftAccount),
      ro(a.signer, true),
      ro(tokenAProgram),
      ro(tokenBProgram),
      ro(eventAuthority),
      ro(METEORA_DAMM_V2_ID),
    ],
    data: concat([
      DISC.addLiquidity,
      u128le(a.liquidityDelta),
      u64le(a.tokenAAmountThreshold),
      u64le(a.tokenBAmountThreshold),
    ]),
  });
}

// ════════════════════════════════════════════════════════════════════════════
// remove_liquidity  (ix_remove_liquidity.rs)
// ════════════════════════════════════════════════════════════════════════════

/**
 * `cp_amm::remove_liquidity` — withdraws from an existing position.
 *
 * Args (RemoveLiquidityParameters, ix_remove_liquidity.rs:765-773): `liquidity_delta:
 * u128 ++ token_a_amount_threshold: u64 ++ token_b_amount_threshold: u64` (the
 * thresholds are the MIN amounts to receive). Accounts (RemoveLiquidityCtx, lines
 * 777-828) — same as add_liquidity but PREFIXED with the `pool_authority` account
 * (the vault-transfer signer) — + `#[event_cpi]`.
 */
export async function removeLiquidity(a: ModifyLiquidityArgs): Promise<TransactionInstruction> {
  const tokenAProgram = a.tokenAProgram ?? TOKEN_PROGRAM_ID;
  const tokenBProgram = a.tokenBProgram ?? TOKEN_PROGRAM_ID;
  const poolAuthority = (await mpda.poolAuthority()).address;
  const eventAuthority = (await mpda.eventAuthority()).address;
  return new TransactionInstruction({
    programId: addr(METEORA_DAMM_V2_ID),
    keys: [
      ro(poolAuthority),
      w(a.pool),
      w(a.position),
      w(a.tokenAAccount),
      w(a.tokenBAccount),
      w(a.tokenAVault),
      w(a.tokenBVault),
      ro(a.tokenAMint),
      ro(a.tokenBMint),
      ro(a.positionNftAccount),
      ro(a.signer, true),
      ro(tokenAProgram),
      ro(tokenBProgram),
      ro(eventAuthority),
      ro(METEORA_DAMM_V2_ID),
    ],
    data: concat([
      DISC.removeLiquidity,
      u128le(a.liquidityDelta),
      u64le(a.tokenAAmountThreshold),
      u64le(a.tokenBAmountThreshold),
    ]),
  });
}

// ════════════════════════════════════════════════════════════════════════════
// swap  (swap/ix_swap.rs)
// ════════════════════════════════════════════════════════════════════════════

export interface SwapArgs {
  pool: AddressInput;
  /** The trader's INPUT token account (the token being sold). */
  inputTokenAccount: AddressInput;
  /** The trader's OUTPUT token account (the token being bought). */
  outputTokenAccount: AddressInput;
  tokenAVault: AddressInput;
  tokenBVault: AddressInput;
  tokenAMint: AddressInput;
  tokenBMint: AddressInput;
  /** Trader + signer. */
  payer: AddressInput;
  /** `SwapParameters.amount_in` (u64). */
  amountIn: bigint | number;
  /** `SwapParameters.minimum_amount_out` (u64). */
  minimumAmountOut?: bigint | number;
  /** Optional referral token account; omitted → the program id (the None sentinel). */
  referralTokenAccount?: AddressInput;
  tokenAProgram?: AddressInput;
  tokenBProgram?: AddressInput;
}

/**
 * `cp_amm::swap` — trades against the pool. Direction is IMPLICIT: whichever of
 * A/B the `inputTokenAccount`/`outputTokenAccount` correspond to. There is NO
 * `swap_type` arg.
 *
 * Args (SwapParameters, swap/ix_swap.rs:986-990): `amount_in: u64 ++
 * minimum_amount_out: u64`. Accounts (SwapCtx, lines 1014-1059) + `#[event_cpi]`.
 * The `referral_token_account` is an `Option<…>`: when absent, Anchor expects the
 * program id in its slot (checked at ix_swap.rs:1156).
 */
export async function swap(a: SwapArgs): Promise<TransactionInstruction> {
  const tokenAProgram = a.tokenAProgram ?? TOKEN_PROGRAM_ID;
  const tokenBProgram = a.tokenBProgram ?? TOKEN_PROGRAM_ID;
  const poolAuthority = (await mpda.poolAuthority()).address;
  const eventAuthority = (await mpda.eventAuthority()).address;
  const referral = a.referralTokenAccount ?? METEORA_DAMM_V2_ID;
  return new TransactionInstruction({
    programId: addr(METEORA_DAMM_V2_ID),
    keys: [
      ro(poolAuthority),
      w(a.pool),
      w(a.inputTokenAccount),
      w(a.outputTokenAccount),
      w(a.tokenAVault),
      w(a.tokenBVault),
      ro(a.tokenAMint),
      ro(a.tokenBMint),
      ro(a.payer, true),
      ro(tokenAProgram),
      ro(tokenBProgram),
      a.referralTokenAccount ? w(referral) : ro(referral),
      ro(eventAuthority),
      ro(METEORA_DAMM_V2_ID),
    ],
    data: concat([DISC.swap, u64le(a.amountIn), u64le(a.minimumAmountOut ?? 0)]),
  });
}

// ════════════════════════════════════════════════════════════════════════════
// claim_position_fee  (ix_claim_position_fee.rs)
// ════════════════════════════════════════════════════════════════════════════

export interface ClaimPositionFeeArgs {
  pool: AddressInput;
  position: AddressInput;
  /** Owner's token-A destination account. */
  tokenAAccount: AddressInput;
  /** Owner's token-B destination account. */
  tokenBAccount: AddressInput;
  tokenAVault: AddressInput;
  tokenBVault: AddressInput;
  tokenAMint: AddressInput;
  tokenBMint: AddressInput;
  positionNftAccount: AddressInput;
  /** Position owner / delegate + signer. */
  signer: AddressInput;
  tokenAProgram?: AddressInput;
  tokenBProgram?: AddressInput;
}

/**
 * `cp_amm::claim_position_fee` — sweeps a position's pending fees to the owner.
 * No args. NOTE the `pool` account is READ-ONLY here (fees live on the Position).
 *
 * Accounts (ClaimPositionFeeCtx, ix_claim_position_fee.rs:1176-1233) + `#[event_cpi]`.
 */
export async function claimPositionFee(a: ClaimPositionFeeArgs): Promise<TransactionInstruction> {
  const tokenAProgram = a.tokenAProgram ?? TOKEN_PROGRAM_ID;
  const tokenBProgram = a.tokenBProgram ?? TOKEN_PROGRAM_ID;
  const poolAuthority = (await mpda.poolAuthority()).address;
  const eventAuthority = (await mpda.eventAuthority()).address;
  return new TransactionInstruction({
    programId: addr(METEORA_DAMM_V2_ID),
    keys: [
      ro(poolAuthority),
      ro(a.pool),
      w(a.position),
      w(a.tokenAAccount),
      w(a.tokenBAccount),
      w(a.tokenAVault),
      w(a.tokenBVault),
      ro(a.tokenAMint),
      ro(a.tokenBMint),
      ro(a.positionNftAccount),
      ro(a.signer, true),
      ro(tokenAProgram),
      ro(tokenBProgram),
      ro(eventAuthority),
      ro(METEORA_DAMM_V2_ID),
    ],
    data: DISC.claimPositionFee,
  });
}

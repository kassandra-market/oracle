/**
 * CU2 — the challenge-market TRADE / CRANK action layer (pure ix-builders, NO
 * React). A challenge round runs over an externally-composed MetaDAO v0.4
 * market: two pass/fail standalone AMMs, each trading a conditional-KASS (base)
 * against a conditional-USDC (quote). CU1 decodes those pools (read); CU2 lets a
 * connected wallet TRADE against a pool (swap) and permissionlessly CRANK its
 * TWAP oracle — the two writes that MOVE the swap-driven verdict `settle` reads.
 *
 * Neither pool address nor the trade mints are stored in a form here: they are
 * DERIVED client-side from the decoded {@link Market} (its `kassVault` /
 * `usdcVault`) exactly as the challenge-market composition does —
 *
 *   conditional mint = PDA `[b"conditional_token", vault, index]` under the
 *                      MetaDAO conditional-vault program (`VLTX…`), index 0 =
 *                      pass, index 1 = fail (mirrors `composeVault`);
 *   amm PDA          = `ammV04.pda.amm(baseMint, quoteMint)` = `[b"amm__",
 *                      base, quote]` (base = conditional-KASS, quote =
 *                      conditional-USDC — the SAME base/quote order the pools
 *                      were built with, `buildPool(kass, usdc)`);
 *   vault ATAs       = `ammV04.pda.ata(amm, mint)`; user ATAs = `ata(user, mint)`.
 *
 * `buildSwapIxs` idempotently create-ATAs the USER's base+quote conditional
 * token accounts (absent → prepend a create), then appends `ammV04.swap`. A
 * `slippageBps` (or an explicit `minAmountOut`) bounds the output against a
 * constant-product estimate from the CU1-decoded reserves.
 *
 * `buildCrankTwapIxs` is trivial: the amm PDA → `ammV04.crankThatTwap`.
 *
 * (settle: the caller reuses RF4's `buildSettleChallengeIxs`.)
 */
import { Address, TransactionInstruction, type Connection } from "@solana/web3.js";
import {
  ATA_PROGRAM_ID,
  EXTERNAL_PROGRAM_IDS,
  SYSTEM_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  ammV04,
  type Market,
} from "@kassandra-market/oracles";

import { ValidationError, type AddressInput } from "../actions";
import type { AmmV04 } from "../ammV04";

/** The MetaDAO conditional-vault program the conditional-token mints live under. */
const CONDITIONAL_VAULT_ID = EXTERNAL_PROGRAM_IDS.conditionalVault;
const CONDITIONAL_TOKEN_SEED = new TextEncoder().encode("conditional_token");

/** Which pass/fail pool of the market to trade / crank. */
export type Pool = "pass" | "fail";
/** Trade direction: `buy` = quote(USDC)→base(KASS); `sell` = base→quote. */
export type Side = "buy" | "sell";

/** The conditional-token index for a pool (mirrors `composeVault`: 0=pass,1=fail). */
function poolIndex(pool: Pool): number {
  return pool === "pass" ? 0 : 1;
}

/** Coerce an {@link AddressInput}, re-typing a parse failure as a field error. */
function addr(field: string, a: AddressInput): Address {
  if (a instanceof Address) return a;
  try {
    return new Address(a);
  } catch {
    throw new ValidationError(field, `${field} is not a valid base58 address.`);
  }
}

function requirePool(pool: Pool): Pool {
  if (pool !== "pass" && pool !== "fail") {
    throw new ValidationError("pool", 'pool must be "pass" or "fail".');
  }
  return pool;
}

function requireSide(side: Side): Side {
  if (side !== "buy" && side !== "sell") {
    throw new ValidationError("side", 'side must be "buy" or "sell".');
  }
  return side;
}

function requirePositiveAmount(field: string, amount: bigint): bigint {
  if (amount <= 0n) throw new ValidationError(field, `${field} must be greater than zero.`);
  return amount;
}

/**
 * The conditional-token MINT PDA `[b"conditional_token", vault, index]` under the
 * MetaDAO conditional-vault program — index 0 = pass outcome, 1 = fail (the exact
 * derivation the challenge-market `composeVault` uses for the two mints it mints).
 */
export async function conditionalTokenMint(
  vault: AddressInput,
  index: number,
): Promise<Address> {
  const [mint] = await Address.findProgramAddress(
    [CONDITIONAL_TOKEN_SEED, addr("vault", vault).toBytes(), Uint8Array.of(index & 0xff)],
    CONDITIONAL_VAULT_ID,
  );
  return mint;
}

/** The base (conditional-KASS) + quote (conditional-USDC) mints of a market pool. */
export interface PoolMints {
  /** Conditional-KASS mint (`[b"conditional_token", market.kassVault, idx]`). */
  base: Address;
  /** Conditional-USDC mint (`[b"conditional_token", market.usdcVault, idx]`). */
  quote: Address;
}

/**
 * Derive a pool's base/quote conditional-token mints off the market's KASS/USDC
 * vaults (index by pass/fail). Base = conditional-KASS, quote = conditional-USDC
 * — the SAME order the pool AMM was created with (`amm(kassMint, usdcMint)`).
 */
export async function poolMints(market: Market, pool: Pool): Promise<PoolMints> {
  const idx = poolIndex(requirePool(pool));
  const [base, quote] = await Promise.all([
    conditionalTokenMint(market.kassVault, idx),
    conditionalTokenMint(market.usdcVault, idx),
  ]);
  return { base, quote };
}

/**
 * The idempotent `createAssociatedTokenAccountIdempotent` ix (ATA program
 * discriminant `1`) — same hand-built layout as the WF1 write layer (no
 * `@solana/spl-token` dep). Accounts: payer(w,signer), ata(w), owner(ro),
 * mint(ro), system program(ro), token program(ro).
 */
function createAtaIdempotentIx(
  payer: Address,
  ata: Address,
  owner: Address,
  mint: Address,
): TransactionInstruction {
  return new TransactionInstruction({
    programId: ATA_PROGRAM_ID,
    keys: [
      { pubkey: payer, isSigner: true, isWritable: true },
      { pubkey: ata, isSigner: false, isWritable: true },
      { pubkey: owner, isSigner: false, isWritable: false },
      { pubkey: mint, isSigner: false, isWritable: false },
      { pubkey: SYSTEM_PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
    ],
    data: Uint8Array.of(1),
  });
}

/**
 * The constant-product output estimate for swapping `amountIn` of the INPUT
 * reserve into the OUTPUT reserve (`out = amountIn·outRes / (inRes + amountIn)`,
 * no fee — a preview only; the on-chain swap applies the real fee/curve). For a
 * BUY the input is quote, output base; for a SELL the reverse. Returns `0n` when
 * a reserve is empty (no meaningful quote).
 */
export function constantProductOut(
  amountIn: bigint,
  inReserve: bigint,
  outReserve: bigint,
): bigint {
  if (amountIn <= 0n || inReserve <= 0n || outReserve <= 0n) return 0n;
  return (amountIn * outReserve) / (inReserve + amountIn);
}

/**
 * The expected out + the fraction of price impact for a swap against `amm`'s
 * decoded reserves — a pure preview the swap form renders (the on-chain swap is
 * the ultimate guard). `impact` is the relative move of the marginal price
 * `1 - (outReserve-out)·inReserve / ((inReserve+amountIn)·outReserve)`,
 * clamped `0..1`. `null` reserves / empty pool → a zero estimate.
 */
export function swapEstimate(
  amm: AmmV04 | null,
  side: Side,
  amountIn: bigint,
): { expectedOut: bigint; impact: number } {
  if (!amm || amountIn <= 0n) return { expectedOut: 0n, impact: 0 };
  // Buy: in=quote, out=base. Sell: in=base, out=quote.
  const inReserve = side === "buy" ? amm.quoteAmount : amm.baseAmount;
  const outReserve = side === "buy" ? amm.baseAmount : amm.quoteAmount;
  const expectedOut = constantProductOut(amountIn, inReserve, outReserve);
  if (inReserve <= 0n || outReserve <= 0n || expectedOut <= 0n) {
    return { expectedOut, impact: 0 };
  }
  // Spot (pre-trade) vs effective (out/in) execution price → price-impact fraction.
  const spot = Number(outReserve) / Number(inReserve);
  const effective = Number(expectedOut) / Number(amountIn);
  const impact = spot > 0 ? Math.min(Math.max(1 - effective / spot, 0), 1) : 0;
  return { expectedOut, impact };
}

/**
 * The minimum-output floor from a constant-product estimate + a slippage
 * tolerance in basis points: `floor = estimateOut · (10_000 - slippageBps) /
 * 10_000`. `0` estimate (empty pool / no reserves) → `0n` (unbounded; the
 * on-chain swap still guards). A caller may instead pass an explicit
 * `minAmountOut` to {@link buildSwapIxs}.
 */
export function minOutFromSlippage(estimateOut: bigint, slippageBps: number): bigint {
  if (estimateOut <= 0n) return 0n;
  const bps = Math.min(Math.max(Math.trunc(slippageBps), 0), 10_000);
  return (estimateOut * BigInt(10_000 - bps)) / 10_000n;
}

// ---------------------------------------------------------------------------
// swap — trade a pool (buy quote→base / sell base→quote), user-signed.
// ---------------------------------------------------------------------------
export interface BuildSwapArgs {
  connection: Connection;
  /** The decoded challenge {@link Market} (derives the vaults → mints → amm). */
  market: Market;
  /** Which pool to trade. */
  pool: Pool;
  /** Trade direction. */
  side: Side;
  /** Input amount (raw base units of the INPUT mint: quote for buy, base for sell). */
  amountIn: bigint | number;
  /** Trader + signer (owns the user token accounts, pays ATA rent). */
  user: AddressInput;
  /**
   * Slippage tolerance in basis points; the builder computes `minAmountOut` from
   * the CU1-decoded reserves + this. Ignored when `minAmountOut` is given.
   */
  slippageBps?: number;
  /** An explicit output floor (overrides the slippage estimate). */
  minAmountOut?: bigint | number;
  /**
   * The CU1-decoded pool (its reserves drive the `minAmountOut` estimate). When
   * omitted only an explicit `minAmountOut` bounds the swap (else `0n`).
   */
  amm?: AmmV04 | null;
}

export async function buildSwapIxs(args: BuildSwapArgs): Promise<TransactionInstruction[]> {
  const pool = requirePool(args.pool);
  const side = requireSide(args.side);
  const amountIn = requirePositiveAmount(
    "amountIn",
    typeof args.amountIn === "bigint" ? args.amountIn : BigInt(Math.trunc(args.amountIn)),
  );
  const user = addr("user", args.user);

  const { base, quote } = await poolMints(args.market, pool);
  const [userBase, userQuote] = await Promise.all([
    ammV04.pda.ata(user, base),
    ammV04.pda.ata(user, quote),
  ]);

  // Idempotent-create the user's base+quote conditional-token ATAs when absent.
  const [baseInfo, quoteInfo] = await Promise.all([
    args.connection.getAccountInfo(userBase),
    args.connection.getAccountInfo(userQuote),
  ]);
  const pre: TransactionInstruction[] = [];
  if (!baseInfo) pre.push(createAtaIdempotentIx(user, userBase, user, base));
  if (!quoteInfo) pre.push(createAtaIdempotentIx(user, userQuote, user, quote));

  const minAmountOut =
    args.minAmountOut !== undefined
      ? typeof args.minAmountOut === "bigint"
        ? args.minAmountOut
        : BigInt(Math.trunc(args.minAmountOut))
      : minOutFromSlippage(
          swapEstimate(args.amm ?? null, side, amountIn).expectedOut,
          args.slippageBps ?? 0,
        );

  const swapIx = await ammV04.swap({
    payer: user,
    baseMint: base,
    quoteMint: quote,
    swapType: side === "buy" ? ammV04.SwapType.Buy : ammV04.SwapType.Sell,
    inputAmount: amountIn,
    minOutputAmount: minAmountOut,
  });
  return [...pre, swapIx];
}

// ---------------------------------------------------------------------------
// crank_that_twap — permissionless; folds the current price into the pool's
// TWAP observation (rate-limited to once per 150 slots on-chain). No signer.
// ---------------------------------------------------------------------------
/** The AMM crank rate-limit window in slots (`ONE_MINUTE_IN_SLOTS`, `metadao.rs`). */
export const CRANK_MIN_SLOTS = 150n;

export interface BuildCrankTwapArgs {
  /** The decoded challenge {@link Market} (derives the vaults → mints → amm). */
  market: Market;
  /** Which pool's TWAP to crank. */
  pool: Pool;
}

export async function buildCrankTwapIxs(
  args: BuildCrankTwapArgs,
): Promise<TransactionInstruction[]> {
  const { base, quote } = await poolMints(args.market, requirePool(args.pool));
  const amm = (await ammV04.pda.amm(base, quote)).address;
  const ix = await ammV04.crankThatTwap({ amm });
  return [ix];
}

/**
 * Whether a pool's TWAP was cranked too recently to crank again — `true` when
 * `currentSlot - amm.lastUpdatedSlot < 150` (the on-chain rate limit would
 * reject / no-op). An unknown `currentSlot` (`null`) or pool (`null`) → `false`
 * (never blocks; the on-chain guard is authoritative).
 */
export function crankRateLimited(amm: AmmV04 | null, currentSlot: bigint | null): boolean {
  if (!amm || currentSlot === null) return false;
  return currentSlot - amm.lastUpdatedSlot < CRANK_MIN_SLOTS;
}

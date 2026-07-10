/**
 * CU3 — CLIENT-SIDE challenge-market COMPOSITION (pure ix-builders, NO React).
 *
 * A challenge round trades over an externally-composed MetaDAO v0.4 market. RF4's
 * {@link buildOpenChallengeIxs} only THREADS an already-composed account set into
 * `open_challenge` (the runner used to emit it as pasted JSON). CU3 removes the
 * paste: it COMPOSES that whole market client-side, mirroring the SDK
 * challenge-market E2E's REAL bits (`composeQuestion` / `composeVault` /
 * `buildPool`) — the production equivalents of the E2E's `surfnet_setAccount`
 * cheatcodes:
 *
 *   - the E2E's `fabricateTokenAccountMint(passKass, oracle, 0)` (an oracle-owned
 *     holder) → an idempotent ATA-create of the ORACLE PDA's conditional-KASS
 *     ATA (`oraclePassKass = ATA(oracle, passKassMint)`);
 *   - the E2E's `setTokenAccountAt(userBase, …, reserve*4)` (a fabricated
 *     conditional-token balance to seed the pools) → the challenger funds its OWN
 *     KASS/USDC, then `split_tokens` mints EQUAL pass+fail conditional tokens
 *     from that underlying into the challenger's conditional-token ATAs, which
 *     `add_liquidity` then seeds the pools with.
 *
 * The whole choreography FAR exceeds one transaction, so this returns an ORDERED
 * list of {@link ComposeStep}s — each a labelled ix-group that fits a single tx —
 * for the UI to send as a SEQUENCE of `sendAndConfirm` calls with per-step status
 * and resume-from-failure. The steps, in order:
 *
 *   1. "Create question"        initialize_question (binary, resolver == oracle)
 *   2. "Create KASS vault"      initialize_conditional_vault (KASS underlying)
 *   3. "Create USDC vault"      initialize_conditional_vault (USDC underlying)
 *   4. "Fund + split"           create the challenger's KASS/USDC + conditional +
 *                               oracle-holder ATAs, then split KASS & USDC into
 *                               pass/fail conditional tokens to seed the pools
 *   5. "Seed pass pool"         create_amm(pass) + add_liquidity(pass)
 *   6. "Seed fail pool"         create_amm(fail) + add_liquidity(fail)
 *   7. "Open challenge"         open_challenge (RF4 builder, fed the composed set)
 *
 * The twap_initial_observation / decimals / seed-liquidity math mirrors
 * `buildPool` EXACTLY: `twap_initial_observation = quoteReserve · 1e12 /
 * baseReserve`, `twap_max_observation_change_per_update = (2^64−1) · 1e12`
 * (single-crank folds the price with no clamp), `twap_start_delay_slots = 0`, and
 * `add_liquidity(quote_amount = quoteReserve, max_base_amount = baseReserve)`.
 *
 * NO core / SDK change: every ix comes from the SDK `futarchy` / `ammV04`
 * builders + RF4's `buildOpenChallengeIxs`; only PDAs/ATAs are derived here.
 */
import { Address, TransactionInstruction, type Connection } from "@solana/web3.js";
import {
  ATA_PROGRAM_ID,
  SYSTEM_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  associatedTokenAccount,
  ammV04,
  futarchy,
  pda,
} from "@kassandra-market/oracles";

import { ValidationError, type AddressInput } from "../actions";
import { conditionalTokenMint } from "./challengeTrade";
import { buildOpenChallengeIxs } from "./challenge";

// ── seed / TWAP math (mirror challenge-market-e2e buildPool) ─────────────────

/** PRICE_SCALE — the v0.4 AMM's fixed-point scale for the TWAP observation. */
export const PRICE_SCALE = 1_000_000_000_000n;
/** `twap_max_observation_change_per_update` — `(2^64−1) · 1e12` (no clamp; a
 * single crank folds the current price into the TWAP verbatim, exactly as the
 * E2E's `MAX_PRICE`). */
export const MAX_OBSERVATION_CHANGE = ((1n << 64n) - 1n) * PRICE_SCALE;
/** Default base reserve: 100 conditional-KASS (9 dp) — the E2E's `BASE_RESERVE`. */
export const DEFAULT_BASE_RESERVE = 100_000_000_000n;
/** Default quote reserve: 100 conditional-USDC (6 dp) → seeded price 1e12-scaled 1.0 (the E2E's `QUOTE_NEUTRAL`). */
export const DEFAULT_QUOTE_RESERVE = 100_000_000n;

/**
 * The v0.4 `twap_initial_observation` for a pool seeded with `baseReserve` base
 * and `quoteReserve` quote — `quoteReserve · PRICE_SCALE / baseReserve` (the
 * scaled spot price the pool opens at). Mirrors `buildPool`'s `initialObs`.
 */
export function twapInitialObservation(baseReserve: bigint, quoteReserve: bigint): bigint {
  if (baseReserve <= 0n) throw new ValidationError("baseReserve", "baseReserve must be greater than zero.");
  return (quoteReserve * PRICE_SCALE) / baseReserve;
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

function requirePositive(field: string, v: bigint): bigint {
  if (v <= 0n) throw new ValidationError(field, `${field} must be greater than zero.`);
  return v;
}

function toBig(field: string, v: bigint | number): bigint {
  const b = typeof v === "bigint" ? v : BigInt(Math.trunc(v));
  return requirePositive(field, b);
}

/**
 * The idempotent `createAssociatedTokenAccountIdempotent` ix (ATA program
 * discriminant `1`) — the same hand-built layout the WF1 write layer uses (no
 * `@solana/spl-token` dep). Accounts: payer(w,signer), ata(w), owner(ro),
 * mint(ro), system program(ro), token program(ro).
 */
function createAtaIdempotentIx(
  payer: Address,
  ataAddr: Address,
  owner: Address,
  mint: Address,
): TransactionInstruction {
  return new TransactionInstruction({
    programId: ATA_PROGRAM_ID,
    keys: [
      { pubkey: payer, isSigner: true, isWritable: true },
      { pubkey: ataAddr, isSigner: false, isWritable: true },
      { pubkey: owner, isSigner: false, isWritable: false },
      { pubkey: mint, isSigner: false, isWritable: false },
      { pubkey: SYSTEM_PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
    ],
    data: Uint8Array.of(1),
  });
}

/** A labelled, single-tx group of instructions in the compose→open sequence. */
export interface ComposeStep {
  /** A stable id for resume/skip logic. */
  id: string;
  /** A human label for the progress UI (e.g. "Create question"). */
  label: string;
  /** The instructions to send in ONE transaction for this step. */
  ixs: TransactionInstruction[];
  /**
   * Optional compute-unit budget hint for this step (some steps CPI heavily —
   * split_tokens / open_challenge). The UI prepends a setComputeUnitLimit ix.
   */
  computeUnits?: number;
}

/**
 * The fully-derived account set the compose produces — the question, the two
 * conditional vaults + their pass/fail mints, the two AMM pool PDAs, and the
 * oracle-owned pass/fail KASS holder ATAs. Returned alongside the steps so a
 * caller (the E2E) can assert against the on-chain accounts.
 */
export interface ComposedMarket {
  oracle: Address;
  question: Address;
  kassVault: Address;
  usdcVault: Address;
  kassVaultUnderlying: Address;
  usdcVaultUnderlying: Address;
  passKassMint: Address;
  failKassMint: Address;
  passUsdcMint: Address;
  failUsdcMint: Address;
  passAmm: Address;
  failAmm: Address;
  oraclePassKass: Address;
  oracleFailKass: Address;
  /** The challenger's USDC source account funding the escrow (its USDC ATA). */
  challengerUsdcSrc: Address;
}

export interface BuildComposeArgs {
  connection: Connection;
  /** Oracle nonce (re-derives the oracle PDA that resolves the question / signs). */
  oracleNonce: bigint | number;
  /** The challenged claim's Proposer PDA (open_challenge derives ai_claim/market). */
  proposer: AddressInput;
  /** Challenger (signer): composes + funds everything, opens the Market. */
  challenger: AddressInput;
  /** The oracle's KASS mint (`oracle.kassMint`). */
  kassMint: AddressInput;
  /** The oracle's USDC mint (`oracle.usdcMint`). */
  usdcMint: AddressInput;
  /** The futarchy `Dao` (`== protocol.kass_dao`) — kass_price source for the escrow. */
  kassDao: AddressInput;
  /**
   * 32-byte question id (seeds the Question PDA). Defaults to a deterministic
   * fill so the same challenger→oracle produces the same market. A caller may
   * pass a distinct id.
   */
  questionId?: Uint8Array;
  /** Base (conditional-KASS) reserve to seed each pool with. Default 100 KASS (9 dp). */
  baseReserve?: bigint | number;
  /** Quote (conditional-USDC) reserve to seed each pool with. Default 100 USDC (6 dp). */
  quoteReserve?: bigint | number;
  programId?: Address;
}

/** The default deterministic question id (mirrors the E2E's `fill(0x07)`). */
export const DEFAULT_QUESTION_ID = new Uint8Array(32).fill(0x07);

/**
 * Compose the FULL MetaDAO v0.4 challenge market client-side + open the challenge,
 * as an ORDERED sequence of single-tx {@link ComposeStep}s. Returns the steps to
 * send in order plus the {@link ComposedMarket} account set.
 *
 * The seed math mirrors `buildPool` verbatim: each pool opens at
 * `twap_initial_observation = quoteReserve·1e12/baseReserve`, with
 * `twap_max_observation_change_per_update = (2^64−1)·1e12` and
 * `twap_start_delay_slots = 0`; `add_liquidity(quote_amount = quoteReserve,
 * max_base_amount = baseReserve)`. Because `split_tokens` mints EQUAL pass+fail
 * conditional tokens from one underlying, the challenger splits `baseReserve`
 * KASS (→ baseReserve pass-KASS + baseReserve fail-KASS) and `quoteReserve` USDC
 * (→ quoteReserve pass-USDC + quoteReserve fail-USDC) to seed BOTH pools.
 */
export async function buildComposeAndOpenChallengeIxs(
  args: BuildComposeArgs,
): Promise<{ steps: ComposeStep[]; composed: ComposedMarket }> {
  const nonce =
    typeof args.oracleNonce === "bigint" ? args.oracleNonce : BigInt(Math.trunc(args.oracleNonce ?? -1));
  if (nonce < 0n) {
    throw new ValidationError("oracleNonce", "The oracle nonce is required to compose this market.");
  }
  const challenger = addr("challenger", args.challenger);
  const kassMint = addr("kassMint", args.kassMint);
  const usdcMint = addr("usdcMint", args.usdcMint);
  const questionId = args.questionId ?? DEFAULT_QUESTION_ID;
  if (!(questionId instanceof Uint8Array) || questionId.length !== 32) {
    throw new ValidationError("questionId", "questionId must be exactly 32 bytes.");
  }
  const baseReserve = toBig("baseReserve", args.baseReserve ?? DEFAULT_BASE_RESERVE);
  const quoteReserve = toBig("quoteReserve", args.quoteReserve ?? DEFAULT_QUOTE_RESERVE);

  // ── PDA / account derivations (all deterministic; no cheatcodes) ──
  const oracle = (await pda.oracle(nonce, args.programId)).address;
  const question = (await futarchy.pda.question(questionId, oracle, 2)).address;

  const kassVault = (await futarchy.pda.conditionalVault(question, kassMint)).address;
  const usdcVault = (await futarchy.pda.conditionalVault(question, usdcMint)).address;
  const [
    passKassMint,
    failKassMint,
    passUsdcMint,
    failUsdcMint,
  ] = await Promise.all([
    conditionalTokenMint(kassVault, 0),
    conditionalTokenMint(kassVault, 1),
    conditionalTokenMint(usdcVault, 0),
    conditionalTokenMint(usdcVault, 1),
  ]);
  const [kassVaultUnderlying, usdcVaultUnderlying] = await Promise.all([
    associatedTokenAccount(kassVault, kassMint).then((p) => p.address),
    associatedTokenAccount(usdcVault, usdcMint).then((p) => p.address),
  ]);

  // Pool PDAs (base = conditional-KASS, quote = conditional-USDC, per buildPool).
  const [passAmm, failAmm] = await Promise.all([
    ammV04.pda.amm(passKassMint, passUsdcMint).then((p) => p.address),
    ammV04.pda.amm(failKassMint, failUsdcMint).then((p) => p.address),
  ]);

  // Oracle-PDA-owned pass/fail conditional-KASS holder ATAs (the split_tokens
  // destinations open_challenge mints into). PRODUCTION equivalent of the E2E's
  // `fabricateTokenAccountMint(passKass, oracle, 0)`.
  const [oraclePassKass, oracleFailKass] = await Promise.all([
    associatedTokenAccount(oracle, passKassMint).then((p) => p.address),
    associatedTokenAccount(oracle, failKassMint).then((p) => p.address),
  ]);

  // The challenger's own token accounts.
  const [
    challengerKass,
    challengerUsdcSrc,
    challengerPassKass,
    challengerFailKass,
    challengerPassUsdc,
    challengerFailUsdc,
  ] = await Promise.all([
    associatedTokenAccount(challenger, kassMint).then((p) => p.address),
    associatedTokenAccount(challenger, usdcMint).then((p) => p.address),
    associatedTokenAccount(challenger, passKassMint).then((p) => p.address),
    associatedTokenAccount(challenger, failKassMint).then((p) => p.address),
    associatedTokenAccount(challenger, passUsdcMint).then((p) => p.address),
    associatedTokenAccount(challenger, failUsdcMint).then((p) => p.address),
  ]);

  const composed: ComposedMarket = {
    oracle,
    question,
    kassVault,
    usdcVault,
    kassVaultUnderlying,
    usdcVaultUnderlying,
    passKassMint,
    failKassMint,
    passUsdcMint,
    failUsdcMint,
    passAmm,
    failAmm,
    oraclePassKass,
    oracleFailKass,
    challengerUsdcSrc,
  };

  // ── Step 1: create the binary question (resolver == oracle) ──
  const questionIx = await futarchy.initializeQuestion({
    questionId,
    oracle,
    numOutcomes: 2,
    payer: challenger,
  });

  // ── Step 2/3: the KASS + USDC conditional vaults (each creates the vault +
  // its two pass/fail conditional-token mints). ──
  const kassVaultIx = await futarchy.initializeConditionalVault({
    question,
    underlyingMint: kassMint,
    payer: challenger,
    numOutcomes: 2,
  });
  const usdcVaultIx = await futarchy.initializeConditionalVault({
    question,
    underlyingMint: usdcMint,
    payer: challenger,
    numOutcomes: 2,
  });

  // ── Step 4: create the challenger's + oracle-holder ATAs, then split the
  // challenger's KASS/USDC into pass/fail conditional tokens to seed the pools. ──
  const fundSplitIxs: TransactionInstruction[] = [];
  // Oracle-owned pass/fail KASS holders (idempotent; the split_tokens targets).
  fundSplitIxs.push(createAtaIdempotentIx(challenger, oraclePassKass, oracle, passKassMint));
  fundSplitIxs.push(createAtaIdempotentIx(challenger, oracleFailKass, oracle, failKassMint));
  // The challenger's conditional-token ATAs (split destinations + add_liquidity sources).
  fundSplitIxs.push(createAtaIdempotentIx(challenger, challengerPassKass, challenger, passKassMint));
  fundSplitIxs.push(createAtaIdempotentIx(challenger, challengerFailKass, challenger, failKassMint));
  fundSplitIxs.push(createAtaIdempotentIx(challenger, challengerPassUsdc, challenger, passUsdcMint));
  fundSplitIxs.push(createAtaIdempotentIx(challenger, challengerFailUsdc, challenger, failUsdcMint));

  // split KASS → baseReserve pass-KASS + baseReserve fail-KASS.
  fundSplitIxs.push(
    await futarchy.splitTokens({
      question,
      vault: kassVault,
      vaultUnderlying: kassVaultUnderlying,
      authority: challenger,
      userUnderlying: challengerKass,
      conditionalMints: [passKassMint, failKassMint],
      userConditionalAccounts: [challengerPassKass, challengerFailKass],
      amount: baseReserve,
    }),
  );
  // split USDC → quoteReserve pass-USDC + quoteReserve fail-USDC.
  fundSplitIxs.push(
    await futarchy.splitTokens({
      question,
      vault: usdcVault,
      vaultUnderlying: usdcVaultUnderlying,
      authority: challenger,
      userUnderlying: challengerUsdcSrc,
      conditionalMints: [passUsdcMint, failUsdcMint],
      userConditionalAccounts: [challengerPassUsdc, challengerFailUsdc],
      amount: quoteReserve,
    }),
  );

  const initialObs = twapInitialObservation(baseReserve, quoteReserve);

  // The pools' LP mints + the challenger's LP ATAs — `add_liquidity` mints LP to
  // the payer's LP ATA but does NOT create it (the E2E's `setTokenAccountAt(userLp
  // …, 0)` cheatcode); production idempotent-creates it before add_liquidity.
  const [passLpMint, failLpMint] = await Promise.all([
    ammV04.pda.lpMint(passAmm).then((p) => p.address),
    ammV04.pda.lpMint(failAmm).then((p) => p.address),
  ]);
  const [challengerPassLp, challengerFailLp] = await Promise.all([
    associatedTokenAccount(challenger, passLpMint).then((p) => p.address),
    associatedTokenAccount(challenger, failLpMint).then((p) => p.address),
  ]);

  // ── Step 5/6: create + seed the pass / fail pools. ──
  const passPoolIxs = [
    await ammV04.createAmm({
      payer: challenger,
      baseMint: passKassMint,
      quoteMint: passUsdcMint,
      twapInitialObservation: initialObs,
      twapMaxObservationChangePerUpdate: MAX_OBSERVATION_CHANGE,
      twapStartDelaySlots: 0n,
    }),
    createAtaIdempotentIx(challenger, challengerPassLp, challenger, passLpMint),
    await ammV04.addLiquidity({
      payer: challenger,
      baseMint: passKassMint,
      quoteMint: passUsdcMint,
      quoteAmount: quoteReserve,
      maxBaseAmount: baseReserve,
      minLpTokens: 0n,
    }),
  ];
  const failPoolIxs = [
    await ammV04.createAmm({
      payer: challenger,
      baseMint: failKassMint,
      quoteMint: failUsdcMint,
      twapInitialObservation: initialObs,
      twapMaxObservationChangePerUpdate: MAX_OBSERVATION_CHANGE,
      twapStartDelaySlots: 0n,
    }),
    createAtaIdempotentIx(challenger, challengerFailLp, challenger, failLpMint),
    await ammV04.addLiquidity({
      payer: challenger,
      baseMint: failKassMint,
      quoteMint: failUsdcMint,
      quoteAmount: quoteReserve,
      maxBaseAmount: baseReserve,
      minLpTokens: 0n,
    }),
  ];

  // ── Step 7: open the challenge over the composed market (RF4 builder). ──
  const cvEventAuthority = (await futarchy.pda.vaultEventAuthority()).address;
  const openIxs = await buildOpenChallengeIxs({
    oracleNonce: nonce,
    proposer: args.proposer,
    challenger,
    question,
    kassVault,
    usdcVault,
    passAmm,
    failAmm,
    kassVaultUnderlying,
    passKassMint,
    failKassMint,
    oraclePassKass,
    oracleFailKass,
    cvEventAuthority,
    kassDao: args.kassDao,
    usdcMint,
    challengerUsdcSrc,
    programId: args.programId,
  });

  const steps: ComposeStep[] = [
    { id: "question", label: "Create question", ixs: [questionIx], computeUnits: 400_000 },
    { id: "kass-vault", label: "Create KASS vault", ixs: [kassVaultIx], computeUnits: 400_000 },
    { id: "usdc-vault", label: "Create USDC vault", ixs: [usdcVaultIx], computeUnits: 400_000 },
    { id: "fund-split", label: "Fund + split conditional tokens", ixs: fundSplitIxs, computeUnits: 600_000 },
    { id: "pass-pool", label: "Seed pass pool", ixs: passPoolIxs, computeUnits: 1_400_000 },
    { id: "fail-pool", label: "Seed fail pool", ixs: failPoolIxs, computeUnits: 1_400_000 },
    { id: "open", label: "Open challenge", ixs: openIxs, computeUnits: 1_400_000 },
  ];

  return { steps, composed };
}

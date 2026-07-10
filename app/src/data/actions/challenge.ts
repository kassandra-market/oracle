/**
 * RF4 — the CHALLENGE (open / settle) + AI-CLAIM action layer (pure ix-builders,
 * NO React). The dispute's challenge round runs over EXTERNALLY-COMPOSED MetaDAO
 * v0.4 markets (a binary question, KASS/USDC conditional vaults, two pass/fail
 * AMMs), so — exactly like the SDK challenge builders
 * (`sdk/src/instructions/challenge.ts`) — the SDK does NOT create that market:
 * the caller composes it in its own transactions (the runner / an integrator
 * emits the account set) and passes the pubkeys here. These builders only
 * validate + thread those accounts (plus the challenger + the oracle nonce) into
 * the SDK builders, which derive the Kassandra-owned PDAs internally.
 *
 *   open_challenge   → {@link buildOpenChallengeIxs}   (opens the Market + escrow)
 *   settle_challenge → {@link buildSettleChallengeIxs} (slot-based TWAP verdict)
 *   submit_ai_claim  → {@link buildSubmitAiClaimIxs}   (the AI-claim payload)
 *
 * --- the oracle nonce ---
 * `open_challenge` / `settle_challenge` carry `oracle_nonce: u64 LE` (payload +
 * re-derives the oracle PDA that signs the split/redeem CPIs). It is NOT stored
 * on the Oracle account, so the caller supplies it (the UI recalls it from the
 * nonce store or the pure {@link resolveOracleNonce} scan, exactly like the RF1 /
 * RF2 builders). A missing/invalid nonce throws a typed {@link ValidationError}.
 *
 * --- the AI-claim hashes ---
 * `submit_ai_claim` commits `model_id[32] ++ params_hash[32] ++ io_hash[32] ++
 * option u8` — the runner produces the three 32-byte hashes (the form accepts
 * them as hex, or a pasted runner payload). Each must be exactly 32 bytes and the
 * option in `0..options_count`, else a typed {@link ValidationError}.
 *
 * `buildSubmitAiClaimIxs` is the only builder here that touches KASS-free
 * accounts (no ATA prep): the challenge open/settle move conditional tokens the
 * caller already composed, and submit_ai_claim only writes the claim PDA.
 */
import { Address, type TransactionInstruction } from "@solana/web3.js";
import { openChallenge, pda, settleChallenge, submitAiClaim } from "@kassandra-market/oracles";
import { ValidationError, type AddressInput } from "../actions";

/** Coerce an {@link AddressInput} into an `Address`, re-typing a parse failure as a field error. */
function addr(field: string, a: AddressInput): Address {
  if (a instanceof Address) return a;
  try {
    return new Address(a);
  } catch {
    throw new ValidationError(field, `${field} is not a valid base58 address.`);
  }
}

/** Validate + coerce the oracle nonce (u64) the challenge ixs commit to. */
function requireNonce(nonce: bigint | number | undefined): bigint {
  if (nonce === undefined || nonce === null) {
    throw new ValidationError(
      "oracleNonce",
      "The oracle nonce is required to sign this challenge (recall it or resolve it first).",
    );
  }
  let v: bigint;
  try {
    v = typeof nonce === "bigint" ? nonce : BigInt(Math.trunc(nonce));
  } catch {
    throw new ValidationError("oracleNonce", "The oracle nonce must be an integer.");
  }
  if (v < 0n) throw new ValidationError("oracleNonce", "The oracle nonce must be non-negative.");
  return v;
}

/** Require an exactly-32-byte hash, else a typed {@link ValidationError}. */
function requireBytes32(field: string, bytes: Uint8Array): Uint8Array {
  if (!(bytes instanceof Uint8Array) || bytes.length !== 32) {
    throw new ValidationError(
      field,
      `${field} must be exactly 32 bytes (got ${bytes instanceof Uint8Array ? bytes.length : "non-bytes"}).`,
    );
  }
  return bytes;
}

/** Validate the claimed categorical option (non-negative, optionally in range). */
function requireOption(option: number, optionsCount?: number): number {
  if (!Number.isInteger(option) || option < 0) {
    throw new ValidationError("option", "option must be a non-negative integer.");
  }
  if (optionsCount !== undefined && option >= optionsCount) {
    throw new ValidationError(
      "option",
      `option ${option} is out of range (options_count = ${optionsCount}).`,
    );
  }
  return option;
}

// ---------------------------------------------------------------------------
// open_challenge — opens the Market + USDC escrow against the composed MetaDAO
// market; the program-signed split_tokens CPI runs against the conditional vault.
// ---------------------------------------------------------------------------
export interface BuildOpenChallengeArgs {
  /** Oracle nonce (payload + re-derives the oracle/stake-vault signer PDAs). */
  oracleNonce: bigint | number;
  /** The challenged claim's Proposer PDA (derives ai_claim / market). */
  proposer: AddressInput;
  /** Challenger (signer): pays the Market + escrow rent, funds the USDC escrow. */
  challenger: AddressInput;
  // --- externally-composed MetaDAO market accounts ---
  /** Binary MetaDAO `Question` (resolver == oracle PDA). */
  question: AddressInput;
  /** KASS conditional vault (underlying == oracle.kass_mint). */
  kassVault: AddressInput;
  /** USDC conditional vault (underlying == oracle.usdc_mint). */
  usdcVault: AddressInput;
  /** Pass-side AMM (owned by the AMM program). */
  passAmm: AddressInput;
  /** Fail-side AMM. */
  failAmm: AddressInput;
  /** `kass_vault.underlying_token_account`. */
  kassVaultUnderlying: AddressInput;
  /** Conditional-KASS mint idx 0 of kass_vault (pass). */
  passKassMint: AddressInput;
  /** Conditional-KASS mint idx 1 of kass_vault (fail). */
  failKassMint: AddressInput;
  /** Oracle-PDA-owned pass-KASS holder token account. */
  oraclePassKass: AddressInput;
  /** Oracle-PDA-owned fail-KASS holder token account. */
  oracleFailKass: AddressInput;
  /** Conditional-vault `#[event_cpi]` event authority PDA. */
  cvEventAuthority: AddressInput;
  /** The futarchy `Dao` (`== protocol.kass_dao`), kass_price source. */
  kassDao: AddressInput;
  /** Canonical USDC mint (`== oracle.usdc_mint`). */
  usdcMint: AddressInput;
  /** Challenger's USDC source token account. */
  challengerUsdcSrc: AddressInput;
  programId?: Address;
}

export async function buildOpenChallengeIxs(
  args: BuildOpenChallengeArgs,
): Promise<TransactionInstruction[]> {
  const nonce = requireNonce(args.oracleNonce);
  const ix = await openChallenge({
    nonce,
    proposer: addr("proposer", args.proposer),
    challenger: addr("challenger", args.challenger),
    question: addr("question", args.question),
    kassVault: addr("kassVault", args.kassVault),
    usdcVault: addr("usdcVault", args.usdcVault),
    passAmm: addr("passAmm", args.passAmm),
    failAmm: addr("failAmm", args.failAmm),
    kassVaultUnderlying: addr("kassVaultUnderlying", args.kassVaultUnderlying),
    passKassMint: addr("passKassMint", args.passKassMint),
    failKassMint: addr("failKassMint", args.failKassMint),
    oraclePassKass: addr("oraclePassKass", args.oraclePassKass),
    oracleFailKass: addr("oracleFailKass", args.oracleFailKass),
    cvEventAuthority: addr("cvEventAuthority", args.cvEventAuthority),
    kassDao: addr("kassDao", args.kassDao),
    usdcMint: addr("usdcMint", args.usdcMint),
    challengerUsdcSrc: addr("challengerUsdcSrc", args.challengerUsdcSrc),
    programId: args.programId,
  });
  return [ix];
}

// ---------------------------------------------------------------------------
// settle_challenge — permissionless; reads the swap-driven AMM TWAP verdict,
// resolves the question, and pays out the bond/escrow. Slot-based gate: only
// after market.twap_end.
// ---------------------------------------------------------------------------
export interface BuildSettleChallengeArgs {
  /** Oracle nonce (payload + re-derives the oracle/stake-vault signer PDAs). */
  oracleNonce: bigint | number;
  /** The challenged claim's AiClaim (`== market.ai_claim`); derives market. */
  aiClaim: AddressInput;
  /** The claim's Proposer PDA (`== market.proposer`). */
  proposer: AddressInput;
  // --- externally-composed MetaDAO market accounts ---
  /** The MetaDAO `Question` (`== market.question`); resolved here. */
  question: AddressInput;
  /** Pass-side AMM (`== market.pass_amm`). */
  passAmm: AddressInput;
  /** Fail-side AMM (`== market.fail_amm`). */
  failAmm: AddressInput;
  /** Conditional-vault `#[event_cpi]` event authority PDA. */
  cvEventAuthority: AddressInput;
  /** KASS conditional vault (`== market.kass_vault`). */
  kassVault: AddressInput;
  /** `kass_vault.underlying_token_account`. */
  kassVaultUnderlying: AddressInput;
  /** Conditional-KASS mint idx 0 of kass_vault (pass). */
  passKassMint: AddressInput;
  /** Conditional-KASS mint idx 1 of kass_vault (fail). */
  failKassMint: AddressInput;
  /** Oracle-PDA-owned pass-KASS holder (`== market.oracle_pass_kass`). */
  oraclePassKass: AddressInput;
  /** Oracle-PDA-owned fail-KASS holder (`== market.oracle_fail_kass`). */
  oracleFailKass: AddressInput;
  /** Proposer's USDC payout account (owner == proposer.authority). */
  proposerUsdc: AddressInput;
  /** Challenger's USDC payout account (owner == market.challenger). */
  challengerUsdcDest: AddressInput;
  /** Challenger's KASS payout account (owner == market.challenger). */
  challengerKass: AddressInput;
  programId?: Address;
}

export async function buildSettleChallengeIxs(
  args: BuildSettleChallengeArgs,
): Promise<TransactionInstruction[]> {
  const nonce = requireNonce(args.oracleNonce);
  const ix = await settleChallenge({
    nonce,
    aiClaim: addr("aiClaim", args.aiClaim),
    proposer: addr("proposer", args.proposer),
    question: addr("question", args.question),
    passAmm: addr("passAmm", args.passAmm),
    failAmm: addr("failAmm", args.failAmm),
    cvEventAuthority: addr("cvEventAuthority", args.cvEventAuthority),
    kassVault: addr("kassVault", args.kassVault),
    kassVaultUnderlying: addr("kassVaultUnderlying", args.kassVaultUnderlying),
    passKassMint: addr("passKassMint", args.passKassMint),
    failKassMint: addr("failKassMint", args.failKassMint),
    oraclePassKass: addr("oraclePassKass", args.oraclePassKass),
    oracleFailKass: addr("oracleFailKass", args.oracleFailKass),
    proposerUsdc: addr("proposerUsdc", args.proposerUsdc),
    challengerUsdcDest: addr("challengerUsdcDest", args.challengerUsdcDest),
    challengerKass: addr("challengerKass", args.challengerKass),
    programId: args.programId,
  });
  return [ix];
}

// ---------------------------------------------------------------------------
// submit_ai_claim — a proposer stamps its AI claim (model/params/io hashes +
// option) in the AiClaim phase. Payload: model_id[32] ++ params_hash[32] ++
// io_hash[32] ++ option u8. The submitter must be `proposer.authority`.
// ---------------------------------------------------------------------------
export interface BuildSubmitAiClaimArgs {
  /** The oracle (must be in the AiClaim phase). */
  oracle: AddressInput;
  /**
   * The submitter's Proposer PDA (`proposer.authority == submitter`). Optional —
   * when omitted it is derived from `[b"proposer", oracle, submitter]`, so
   * callers need only pass the oracle + submitter.
   */
  proposer?: AddressInput;
  /** Proposer authority (signer): pays the AiClaim rent. */
  submitter: AddressInput;
  /** 32-byte pinned model id (runner-produced). */
  modelId: Uint8Array;
  /** 32-byte model-params hash (runner-produced). */
  paramsHash: Uint8Array;
  /** 32-byte input/output hash (runner-produced). */
  ioHash: Uint8Array;
  /** The claimed categorical option (< oracle.options_count). */
  option: number;
  /** When supplied, validates `option < optionsCount`. */
  optionsCount?: number;
  programId?: Address;
}

export async function buildSubmitAiClaimIxs(
  args: BuildSubmitAiClaimArgs,
): Promise<TransactionInstruction[]> {
  const modelId = requireBytes32("modelId", args.modelId);
  const paramsHash = requireBytes32("paramsHash", args.paramsHash);
  const ioHash = requireBytes32("ioHash", args.ioHash);
  const option = requireOption(args.option, args.optionsCount);
  const oracle = addr("oracle", args.oracle);
  const submitter = addr("submitter", args.submitter);
  const proposer = args.proposer
    ? addr("proposer", args.proposer)
    : (await pda.proposer(oracle, submitter)).address;
  const ix = await submitAiClaim({
    oracle,
    proposer,
    authority: submitter,
    modelId,
    paramsHash,
    ioHash,
    option,
    programId: args.programId,
  });
  return [ix];
}

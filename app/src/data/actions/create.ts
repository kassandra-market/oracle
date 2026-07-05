/**
 * RF3 — the create-oracle write ACTION (pure ix-builder, NO React).
 *
 * {@link buildCreateOracleIxs} hashes the human-readable `question` into the
 * 32-byte `prompt_hash` the Oracle PDA commits to (SHA-256, via the WF1
 * {@link hashToContentHash} seam), derives the creator's KASS Associated Token
 * Account (`ATA(creator, kassMint)` — the burn source for the dynamic creation
 * fee), PREPENDS an idempotent create-ATA instruction when that account is
 * absent, and appends the `@kassandra/sdk` `createOracle` builder.
 *
 * Unlike the participation builders it returns a RICHER result — the resolved
 * `nonce`, the derived Oracle PDA `oracle`, and the `promptHash` — so the create
 * page can show the hash and navigate to the new oracle's detail on success
 * (the Oracle PDA is `[b"oracle", nonce_le8]`, fully determined by the nonce).
 *
 * Nonce: the oracle PDA is seeded by a CALLER-PICKED arbitrary u64 (there is no
 * protocol counter — see `pda.oracle` + the lifecycle E2E, which just pick 1n,
 * 2n). When omitted a cryptographically-random u64 is generated so two creators
 * don't collide. Validation (question non-empty, optionsCount 2..255, deadline a
 * future unix timestamp, twapWindow > 0, valid mints) throws a typed
 * {@link ValidationError} the form surfaces inline.
 */
import { Address, TransactionInstruction, type Connection } from "@solana/web3.js";
import {
  ATA_PROGRAM_ID,
  SYSTEM_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  associatedTokenAccount,
  createOracle,
  pda,
} from "@kassandra/sdk";
import { ValidationError, hashToContentHash } from "../actions";

/** Anything that names an account: a web3.js `Address` or a base58 string. */
export type AddressInput = Address | string;

/** Default TWAP window (seconds) baked into the oracle at creation. */
export const DEFAULT_TWAP_WINDOW = 3600n;

/** Inclusive upper bound on `options_count` (u8; index 255 is the CLAIM_OPTION_NONE sentinel). */
export const MAX_OPTIONS_COUNT = 255;

/** The SPL Memo program — carries the off-chain oracle metadata (see below). */
const MEMO_PROGRAM_ID = new Address("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");

/**
 * The chain stores only a prompt HASH + options_count, so the human-readable
 * SUBJECT (== the hashed question) and the option LABELS are attached as an SPL
 * Memo instruction in the create tx. The indexer captures it (keyed by the oracle
 * PDA) and serves it; the client re-hashes `subject` against the on-chain
 * `prompt_hash` to verify it before trusting it. Options are advisory (not hashed).
 */
export function buildOracleMetaMemoIx(subject: string, options: string[]): TransactionInstruction {
  const payload = JSON.stringify({ v: 1, subject, options });
  return new TransactionInstruction({
    programId: MEMO_PROGRAM_ID,
    keys: [],
    data: new TextEncoder().encode(payload),
  });
}

/** Coerce an {@link AddressInput} into an `Address`, re-typing a parse failure as a field error. */
function mint(field: string, a: AddressInput): Address {
  if (a instanceof Address) return a;
  try {
    return new Address(a);
  } catch {
    throw new ValidationError(field, `${field} is not a valid base58 address.`);
  }
}

/**
 * The idempotent `createAssociatedTokenAccountIdempotent` instruction (ATA
 * program discriminant `1`). Accounts (SPL ATA order): funding payer(w,signer),
 * ata(w), owner(ro), mint(ro), system program(ro), token program(ro). Mirrors
 * the WF1 `actions.ts` helper (kept local so the WF1 data file is import-only).
 */
function createAtaIdempotentIx(
  payer: Address,
  ata: Address,
  owner: Address,
  kassMint: Address,
): TransactionInstruction {
  return new TransactionInstruction({
    programId: ATA_PROGRAM_ID,
    keys: [
      { pubkey: payer, isSigner: true, isWritable: true },
      { pubkey: ata, isSigner: false, isWritable: true },
      { pubkey: owner, isSigner: false, isWritable: false },
      { pubkey: kassMint, isSigner: false, isWritable: false },
      { pubkey: SYSTEM_PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
    ],
    data: Uint8Array.of(1),
  });
}

/**
 * Derive `ATA(creator, kassMint)` and, when the account is absent
 * (`getAccountInfo` null), return an idempotent create-ATA ix to prepend
 * (payer == owner == creator).
 */
async function ensureCreatorKassAta(
  connection: Connection,
  creator: Address,
  kassMint: Address,
): Promise<{ ata: Address; createIx?: TransactionInstruction }> {
  const ata = (await associatedTokenAccount(creator, kassMint)).address;
  const info = await connection.getAccountInfo(ata);
  const createIx = info ? undefined : createAtaIdempotentIx(creator, ata, creator, kassMint);
  return { ata, createIx };
}

/** A cryptographically-random u64 nonce (seeds the Oracle PDA when none is supplied). */
export function randomNonce(): bigint {
  const buf = new BigUint64Array(1);
  crypto.getRandomValues(buf);
  return buf[0];
}

export interface BuildCreateOracleArgs {
  connection: Connection;
  /** Human-readable question text — SHA-256'd into the on-chain `prompt_hash`. */
  question: string;
  /**
   * The option LABELS (2..255). When given, they drive `options_count` AND are
   * attached (with the subject) as a memo so the browse/detail views can show
   * them. Prefer this over the bare `optionsCount`.
   */
  options?: string[];
  /** Categorical option count — used only when `options` is not provided. */
  optionsCount?: number;
  /** Resolution deadline as a unix timestamp (seconds); must be in the future. */
  deadline: bigint | number;
  /** Creator authority (the signer): pays rent + is the creation-fee burn source. */
  creator: AddressInput;
  /** Canonical KASS mint (must equal `protocol.kass_mint`). */
  kassMint: AddressInput;
  /** Canonical USDC mint (must equal `protocol.usdc_mint`). */
  usdcMint: AddressInput;
  /** Oracle nonce — seeds the PDA `[b"oracle", nonce_le8]`. Random u64 when omitted. */
  nonce?: bigint | number;
  /** TWAP window (seconds, > 0). Defaults to {@link DEFAULT_TWAP_WINDOW}. */
  twapWindow?: bigint | number;
  programId?: Address;
}

/** The create-oracle build result: the ixs to send plus the derived identifiers. */
export interface CreateOracleBuild {
  /** The instruction list (optional create-ATA, then `createOracle`). */
  ixs: TransactionInstruction[];
  /** The resolved oracle nonce (caller-supplied or generated). */
  nonce: bigint;
  /** The derived Oracle PDA (`[b"oracle", nonce_le8]`) — the navigation target. */
  oracle: Address;
  /** The 32-byte SHA-256 of `question` written on-chain as `prompt_hash`. */
  promptHash: Uint8Array;
}

/**
 * Assemble the create-oracle instruction list. Validates the inputs, hashes the
 * question, derives the creator's KASS ATA (prepending an idempotent create when
 * absent), and appends the SDK `createOracle` ix. Returns the ixs plus the
 * resolved nonce / Oracle PDA / promptHash for the form.
 */
export async function buildCreateOracleIxs(
  args: BuildCreateOracleArgs,
): Promise<CreateOracleBuild> {
  if (args.question.trim().length === 0) {
    throw new ValidationError("question", "Question must not be empty.");
  }
  // Option labels (preferred) drive the count + the memo; a bare `optionsCount`
  // is still accepted (legacy / no-metadata path).
  if (args.options) {
    if (args.options.some((o) => o.trim().length === 0)) {
      throw new ValidationError("options", "Option labels must not be empty.");
    }
  }
  const optionsCount = args.options ? args.options.length : args.optionsCount;
  if (optionsCount === undefined || !Number.isInteger(optionsCount) || optionsCount < 2) {
    throw new ValidationError("optionsCount", "There must be at least 2 options.");
  }
  if (optionsCount > MAX_OPTIONS_COUNT) {
    throw new ValidationError(
      "optionsCount",
      `Options count must be <= ${MAX_OPTIONS_COUNT}.`,
    );
  }
  const deadline = BigInt(args.deadline);
  const nowUnix = BigInt(Math.floor(Date.now() / 1000));
  if (deadline <= nowUnix) {
    throw new ValidationError("deadline", "Deadline must be a future unix timestamp.");
  }
  const twapWindow = args.twapWindow === undefined ? DEFAULT_TWAP_WINDOW : BigInt(args.twapWindow);
  if (twapWindow <= 0n) {
    throw new ValidationError("twapWindow", "TWAP window must be greater than zero.");
  }

  const creator = mint("creator", args.creator);
  const kassMint = mint("kassMint", args.kassMint);
  const usdcMint = mint("usdcMint", args.usdcMint);

  const nonce = args.nonce === undefined ? randomNonce() : BigInt(args.nonce);
  const promptHash = await hashToContentHash(args.question);
  const oracle = (await pda.oracle(nonce, args.programId)).address;

  const { ata, createIx } = await ensureCreatorKassAta(args.connection, creator, kassMint);

  const ix = await createOracle({
    nonce,
    promptHash,
    optionsCount,
    deadline,
    twapWindow,
    creator,
    creatorKassToken: ata,
    kassMint,
    usdcMint,
    programId: args.programId,
  });

  const ixs: TransactionInstruction[] = createIx ? [createIx, ix] : [ix];
  // Attach the plaintext subject + option labels as a memo so they can be indexed
  // and shown when browsing (the chain keeps only the prompt hash).
  if (args.options) {
    ixs.push(buildOracleMetaMemoIx(args.question, args.options));
  }

  return { ixs, nonce, oracle, promptHash };
}

/**
 * WF1 — the write ACTION layer (pure ix-builders, NO React).
 *
 * Each builder takes a {@link Connection} (for an ATA-existence check), derives
 * the authority's KASS Associated Token Account (`ATA(authority, kassMint)`),
 * PREPENDS an idempotent create-ATA instruction when that account is absent, and
 * appends the corresponding `@kassandra-market/oracles` write builder
 * (`propose` / `submitFact` / `voteFact`). The returned list is handed straight
 * to {@link sendAndConfirm} with a wallet- or keypair-backed {@link TxSender} —
 * the SAME action works in the UI (WF2) and in the gated surfpool E2E.
 *
 * Inputs are validated up front (positive bond/stake, uri <= 200 bytes, a valid
 * vote `kind`, and — when `optionsCount` is supplied — an in-range option) with a
 * typed {@link ValidationError} the form surfaces inline.
 */
import { Address, TransactionInstruction, type Connection } from "@solana/web3.js";
import {
  ATA_PROGRAM_ID,
  SYSTEM_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  VOTE_APPROVE,
  VOTE_DUPLICATE,
  associatedTokenAccount,
  propose,
  submitFact,
  voteFact,
} from "@kassandra-market/oracles";

/** Anything that names an account: a web3.js `Address` or a base58 string. */
export type AddressInput = Address | string;

/** Coerce an {@link AddressInput} into an `Address`. */
function addr(a: AddressInput): Address {
  return a instanceof Address ? a : new Address(a);
}

/** Thrown when a builder's input fails validation before any ix is assembled. */
export class ValidationError extends Error {
  /** The offending field (`bond` / `stake` / `uri` / `kind` / `option`). */
  readonly field: string;
  constructor(field: string, message: string) {
    super(message);
    this.name = "ValidationError";
    this.field = field;
  }
}

const enc = new TextEncoder();

function requirePositiveAmount(field: string, amount: bigint | number): void {
  const v = typeof amount === "bigint" ? amount : BigInt(Math.trunc(amount));
  if (v <= 0n) throw new ValidationError(field, `${field} must be greater than zero.`);
}

function requireOption(option: number, optionsCount?: number): void {
  if (!Number.isInteger(option) || option < 0) {
    throw new ValidationError("option", "option must be a non-negative integer.");
  }
  if (optionsCount !== undefined && option >= optionsCount) {
    throw new ValidationError(
      "option",
      `option ${option} is out of range (options_count = ${optionsCount}).`,
    );
  }
}

function requireUri(uri: string | Uint8Array): void {
  const len = typeof uri === "string" ? enc.encode(uri).length : uri.length;
  if (len > 200) throw new ValidationError("uri", `uri is ${len} bytes (max 200).`);
}

function requireVoteKind(kind: number): void {
  if (kind !== VOTE_APPROVE && kind !== VOTE_DUPLICATE) {
    throw new ValidationError(
      "kind",
      `kind must be VOTE_APPROVE (${VOTE_APPROVE}) or VOTE_DUPLICATE (${VOTE_DUPLICATE}).`,
    );
  }
}

/**
 * The idempotent `createAssociatedTokenAccountIdempotent` instruction (ATA
 * program discriminant `1`). Accounts (SPL ATA order): funding payer(w,signer),
 * ata(w), owner(ro), mint(ro), system program(ro), token program(ro). Built by
 * hand (no `@solana/spl-token` dep) — the layout is a stable public contract.
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
 * Derive `ATA(authority, kassMint)` and, if the account is absent
 * (`getAccountInfo` null), return an idempotent create-ATA ix to prepend
 * (payer == owner == authority). The ATA address is always returned so the
 * caller passes it as the token account to the SDK builder.
 */
async function ensureKassAta(
  connection: Connection,
  authority: Address,
  kassMint: Address,
): Promise<{ ata: Address; createIx?: TransactionInstruction }> {
  const ata = (await associatedTokenAccount(authority, kassMint)).address;
  const info = await connection.getAccountInfo(ata);
  const createIx = info
    ? undefined
    : createAtaIdempotentIx(authority, ata, authority, kassMint);
  return { ata, createIx };
}

/**
 * SHA-256 a fact's text/bytes into the 32-byte `content_hash` the Fact PDA is
 * seeded by. A helper the submit-fact form can call on a pasted string/URI; the
 * builder itself takes the raw 32 bytes.
 */
export async function hashToContentHash(input: string | Uint8Array): Promise<Uint8Array> {
  const bytes = typeof input === "string" ? enc.encode(input) : input;
  const digest = await crypto.subtle.digest("SHA-256", bytes as Uint8Array<ArrayBuffer>);
  return new Uint8Array(digest);
}

// ---------------------------------------------------------------------------
// propose
// ---------------------------------------------------------------------------
export interface BuildProposeArgs {
  connection: Connection;
  /** The oracle being proposed against (must be in the Proposal phase). */
  oracle: AddressInput;
  /** The oracle's KASS mint (`oracle.kassMint` from the read layer). */
  kassMint: AddressInput;
  /** Proposer authority (the signer): funds rent + bond. */
  authority: AddressInput;
  /** Categorical option proposed. */
  option: number;
  /** KASS bond escrowed into the stake vault (> 0). */
  bond: bigint | number;
  /** When supplied, validates `option < optionsCount`. */
  optionsCount?: number;
  programId?: Address;
}

export async function buildProposeIxs(args: BuildProposeArgs): Promise<TransactionInstruction[]> {
  requirePositiveAmount("bond", args.bond);
  requireOption(args.option, args.optionsCount);
  const authority = addr(args.authority);
  const { ata, createIx } = await ensureKassAta(args.connection, authority, addr(args.kassMint));
  const ix = await propose({
    oracle: args.oracle,
    authority,
    authorityKass: ata,
    option: args.option,
    bond: args.bond,
    programId: args.programId,
  });
  return createIx ? [createIx, ix] : [ix];
}

// ---------------------------------------------------------------------------
// submitFact
// ---------------------------------------------------------------------------
export interface BuildSubmitFactArgs {
  connection: Connection;
  /** The oracle being supported (must be in the FactProposal phase). */
  oracle: AddressInput;
  kassMint: AddressInput;
  /** Submitter authority (the signer): funds rent + stake. */
  submitter: AddressInput;
  /** 32-byte fact content hash (see {@link hashToContentHash}). */
  contentHash: Uint8Array;
  /** KASS stake escrowed for the fact (> 0). */
  stake: bigint | number;
  /** Fact uri (<= 200 bytes). */
  uri: string | Uint8Array;
  programId?: Address;
}

export async function buildSubmitFactIxs(
  args: BuildSubmitFactArgs,
): Promise<TransactionInstruction[]> {
  requirePositiveAmount("stake", args.stake);
  requireUri(args.uri);
  const submitter = addr(args.submitter);
  const { ata, createIx } = await ensureKassAta(args.connection, submitter, addr(args.kassMint));
  const ix = await submitFact({
    oracle: args.oracle,
    submitter,
    submitterKass: ata,
    contentHash: args.contentHash,
    stake: args.stake,
    uri: args.uri,
    programId: args.programId,
  });
  return createIx ? [createIx, ix] : [ix];
}

// ---------------------------------------------------------------------------
// voteFact
// ---------------------------------------------------------------------------
export interface BuildVoteFactArgs {
  connection: Connection;
  /** The oracle (must be in the FactVoting phase). */
  oracle: AddressInput;
  kassMint: AddressInput;
  /** The fact being voted on. */
  fact: AddressInput;
  /** Voter authority (the signer): funds rent + stake. */
  voter: AddressInput;
  /** `VOTE_APPROVE` (0) or `VOTE_DUPLICATE` (1). */
  kind: number;
  /** KASS stake escrowed for the vote (> 0). */
  stake: bigint | number;
  programId?: Address;
}

export async function buildVoteFactIxs(
  args: BuildVoteFactArgs,
): Promise<TransactionInstruction[]> {
  requirePositiveAmount("stake", args.stake);
  requireVoteKind(args.kind);
  const voter = addr(args.voter);
  const { ata, createIx } = await ensureKassAta(args.connection, voter, addr(args.kassMint));
  const ix = await voteFact({
    oracle: args.oracle,
    fact: args.fact,
    voter,
    voterKass: ata,
    kind: args.kind,
    stake: args.stake,
    programId: args.programId,
  });
  return createIx ? [createIx, ix] : [ix];
}

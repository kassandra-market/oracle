/**
 * RF3 — the create-oracle write ACTION (pure ix-builder, NO React).
 *
 * {@link buildCreateOracleIxs} hashes the human-readable `question` into the
 * 32-byte `prompt_hash` the Oracle PDA commits to (SHA-256, via the WF1
 * {@link hashToContentHash} seam), derives the creator's KASS Associated Token
 * Account (`ATA(creator, kassMint)` — the burn source for the dynamic creation
 * fee), PREPENDS an idempotent create-ATA instruction when that account is
 * absent, and appends the `@kassandra-market/oracles` `createOracle` builder.
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
  writeOracleMeta,
} from "@kassandra-market/oracles";
import { ValidationError, hashToContentHash } from "../actions";

/** Anything that names an account: a web3.js `Address` or a base58 string. */
export type AddressInput = Address | string;

/** Default TWAP window (seconds) baked into the oracle at creation. */
export const DEFAULT_TWAP_WINDOW = 3600n;

/** Inclusive upper bound on `options_count` (u8; index 255 is the CLAIM_OPTION_NONE sentinel). */
export const MAX_OPTIONS_COUNT = 255;

/** Version tag of the extended off-chain metadata JSON schema. */
export const ORACLE_META_JSON_VERSION = 1;

/**
 * The default AI-runner interpretation (the `promptTemplate`) generated from the
 * question. The runner reads this verbatim as the resolution rule, so it embeds
 * the concrete subject rather than a placeholder template. Creators can override
 * it via the create form's "Advanced" disclosure.
 */
export function defaultPromptTemplate(subject: string): string {
  return (
    `Resolve the question "${subject}" by selecting exactly one of the listed options. ` +
    `Base the decision ONLY on the verified facts provided. If the facts are insufficient ` +
    `to decide, choose the option that best reflects the most likely outcome given those facts.`
  );
}

/** The extended off-chain metadata JSON (hosted at the oracle's `uri`, bound by `uri_hash`). */
export interface OracleMetadataJson {
  version: number;
  subject: string;
  options: string[];
  promptTemplate: string;
  interpretation?: string;
  category?: string;
  createdAt: number;
}

/**
 * Build the extended-metadata JSON object. The on-chain `oracle_meta` stores the
 * subject + option labels (program-readable) plus this JSON's `uri` + its
 * `sha256` (`uri_hash`); the extended fields (prompt template, interpretation,
 * category) live here, off chain, bound by that hash.
 */
export function buildOracleMetadataJson(args: {
  subject: string;
  options: string[];
  promptTemplate?: string;
  interpretation?: string;
  category?: string;
  createdAt?: number;
}): OracleMetadataJson {
  const json: OracleMetadataJson = {
    version: ORACLE_META_JSON_VERSION,
    subject: args.subject,
    options: args.options,
    promptTemplate: args.promptTemplate?.trim() || defaultPromptTemplate(args.subject),
    createdAt: args.createdAt ?? Date.now(),
  };
  if (args.interpretation?.trim()) json.interpretation = args.interpretation.trim();
  if (args.category?.trim()) json.category = args.category.trim();
  return json;
}

/** The public URL the app server hosts an oracle's metadata JSON at. */
export function oracleMetadataUri(appOrigin: string, oracle: Address): string {
  return `${appOrigin.replace(/\/$/, "")}/api/oracle/${oracle.toString()}/metadata.json`;
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
  /**
   * The app's public origin (e.g. `https://app.kassandra.fi` or, in dev,
   * `window.location.origin`). REQUIRED when `options` is provided: the oracle's
   * metadata `uri` is hosted at `${appOrigin}/api/oracle/{oracle}/metadata.json`.
   */
  appOrigin?: string;
  /** AI-runner interpretation template. Defaults to {@link defaultPromptTemplate}. */
  promptTemplate?: string;
  /** Optional human resolution rules (stored in the off-chain JSON). */
  interpretation?: string;
  /** Optional category tag (stored in the off-chain JSON). */
  category?: string;
  programId?: Address;
}

/** The create-oracle build result: the ixs to send plus the derived identifiers. */
export interface CreateOracleBuild {
  /** The instruction list (optional create-ATA, `createOracle`, then `writeOracleMeta`). */
  ixs: TransactionInstruction[];
  /** The resolved oracle nonce (caller-supplied or generated). */
  nonce: bigint;
  /** The derived Oracle PDA (`[b"oracle", nonce_le8]`) — the navigation target. */
  oracle: Address;
  /**
   * The extended metadata JSON to POST to the app server (which hosts it at the
   * on-chain `uri`, gated by `uri_hash`). Present only when `options` was given.
   * `jsonString` is the EXACT bytes that were hashed into `uri_hash` — POST it
   * verbatim so the served content matches the commitment.
   */
  metadata?: {
    json: OracleMetadataJson;
    jsonString: string;
    uri: string;
    uriHash: Uint8Array;
  };
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
  const oracle = (await pda.oracle(nonce, args.programId)).address;

  const { ata, createIx } = await ensureCreatorKassAta(args.connection, creator, kassMint);

  const ix = await createOracle({
    nonce,
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

  // With option labels, write the on-chain metadata (subject + labels + uri/uri_hash)
  // in the SAME tx (atomic: no oracle without its metadata). The extended JSON is
  // returned for the caller to POST to the app's metadata host.
  let metadata: CreateOracleBuild["metadata"];
  if (args.options) {
    const json = buildOracleMetadataJson({
      subject: args.question,
      options: args.options,
      promptTemplate: args.promptTemplate,
      interpretation: args.interpretation,
      category: args.category,
    });
    const jsonString = JSON.stringify(json);
    const uriHash = await hashToContentHash(jsonString);
    const uri = args.appOrigin ? oracleMetadataUri(args.appOrigin, oracle) : "";

    ixs.push(
      await writeOracleMeta({
        oracle,
        creator,
        subject: args.question,
        options: args.options,
        uri,
        uriHash,
        programId: args.programId,
      }),
    );
    metadata = { json, jsonString, uri, uriHash };
  }

  return { ixs, nonce, oracle, metadata };
}

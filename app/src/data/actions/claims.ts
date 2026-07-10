/**
 * RF2 — the CLAIM / CLOSE / SWEEP settlement action layer (pure ix-builders, NO
 * React). The payout tail of a Resolved oracle:
 *
 *   claim_proposer    → {@link buildClaimProposerIxs}   (bond ± slash + reward)
 *   claim_fact        → {@link buildClaimFactIxs}        (fact stake + reward)
 *   claim_fact_vote   → {@link buildClaimFactVoteIxs}    (vote stake ± slash + reward)
 *   close_ai_claim    → {@link buildCloseAiClaimIxs}     (rent → ai_claim.authority)
 *   close_market      → {@link buildCloseMarketIxs}      (rent → market.challenger)
 *   sweep_oracle      → {@link buildSweepOracleIxs}      (residual → treasury, close)
 *
 * The three CLAIMS pay KASS out of the oracle's stake-vault into the
 * participant's KASS Associated Token Account (`ATA(authority, kassMint)`) and
 * refund the child-account rent to that same authority. Each claim builder
 * therefore derives that ATA (idempotently PREPENDING a create-ATA when it is
 * absent, mirroring the WF1/RF3 seam) and passes it as `destKass` +
 * `rentRecipient == authority`. The two CLOSES and the SWEEP move no KASS to a
 * participant ATA, so they carry no ATA prep — just the single SDK ix.
 *
 * --- the oracle nonce ---
 * `claim_*` / `close_market` / `sweep_oracle` all carry `oracle_nonce: u64 LE`
 * (payload + re-derives the oracle PDA that signs the vault payout / the account
 * closes). It is NOT stored on the Oracle account, so the caller supplies it
 * (the UI recalls it from {@link recallNonce} or the pure
 * {@link resolveOracleNonce} scan, exactly like RF1's finalize builders). A
 * missing/invalid nonce throws a typed {@link ValidationError}.
 *
 * These builders match the SDK settlement account/arg shapes EXACTLY (see
 * `sdk/src/instructions/settlement.ts`): `nonce` (not oracle) for the three
 * claims / close_market / sweep, `destKass` = the participant ATA, the rent
 * recipients, and the sweep's dao_treasury = ATA(daoAuthority, kassMint) derived
 * inside the SDK from `kassMint` + `daoAuthority`.
 */
import { Address, TransactionInstruction, type Connection } from "@solana/web3.js";
import {
  ATA_PROGRAM_ID,
  SYSTEM_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  associatedTokenAccount,
  claimFact,
  claimFactVote,
  claimProposer,
  closeAiClaim,
  closeMarket,
  decodeProtocol,
  pda,
  sweepOracle,
} from "@kassandra-market/oracles";
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

/** Validate + coerce the oracle nonce (u64) the settlement ixs commit to. */
function requireNonce(nonce: bigint | number | undefined): bigint {
  if (nonce === undefined || nonce === null) {
    throw new ValidationError(
      "oracleNonce",
      "The oracle nonce is required to sign this settlement (recall it or resolve it first).",
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

/**
 * The idempotent `createAssociatedTokenAccountIdempotent` instruction (ATA
 * program discriminant `1`). Accounts (SPL ATA order): funding payer(w,signer),
 * ata(w), owner(ro), mint(ro), system program(ro), token program(ro). Kept local
 * so the WF1 `actions.ts` data file stays import-only (mirrors create.ts).
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
 * Derive `ATA(owner, kassMint)` — the claim's KASS payout destination — and, when
 * the account is absent (`getAccountInfo` null), return an idempotent create-ATA
 * ix to prepend (payer == owner). The ATA address is always returned so the
 * builder passes it as the SDK `destKass`.
 */
async function ensureKassAta(
  connection: Connection,
  owner: Address,
  kassMint: Address,
): Promise<{ ata: Address; createIx?: TransactionInstruction }> {
  const ata = (await associatedTokenAccount(owner, kassMint)).address;
  const info = await connection.getAccountInfo(ata);
  const createIx = info ? undefined : createAtaIdempotentIx(owner, ata, owner, kassMint);
  return { ata, createIx };
}

// ---------------------------------------------------------------------------
// claim_proposer — bond (±slash) + proposer reward → ATA(authority), close.
// ---------------------------------------------------------------------------
export interface BuildClaimProposerArgs {
  connection: Connection;
  /** Oracle nonce (payload + re-derives the vault-authority PDA). */
  oracleNonce: bigint | number;
  /** The Proposer PDA being claimed + closed (`detail.proposers[].pubkey`). */
  proposer: AddressInput;
  /** `proposer.authority` — the KASS payout owner + rent recipient. */
  authority: AddressInput;
  /** The oracle's KASS mint (`oracle.kassMint`). */
  kassMint: AddressInput;
  programId?: Address;
}

export async function buildClaimProposerIxs(
  args: BuildClaimProposerArgs,
): Promise<TransactionInstruction[]> {
  const nonce = requireNonce(args.oracleNonce);
  const authority = addr("authority", args.authority);
  const { ata, createIx } = await ensureKassAta(args.connection, authority, addr("kassMint", args.kassMint));
  const ix = await claimProposer({
    nonce,
    proposer: args.proposer,
    destKass: ata,
    rentRecipient: authority,
    programId: args.programId,
  });
  return createIx ? [createIx, ix] : [ix];
}

// ---------------------------------------------------------------------------
// claim_fact — fact stake + fact reward → ATA(proposer), close.
// ---------------------------------------------------------------------------
export interface BuildClaimFactArgs {
  connection: Connection;
  oracleNonce: bigint | number;
  /** The Fact PDA being claimed + closed (`detail.facts[].pubkey`). */
  fact: AddressInput;
  /** `fact.proposer` — the KASS payout owner + rent recipient. */
  authority: AddressInput;
  kassMint: AddressInput;
  programId?: Address;
}

export async function buildClaimFactIxs(
  args: BuildClaimFactArgs,
): Promise<TransactionInstruction[]> {
  const nonce = requireNonce(args.oracleNonce);
  const authority = addr("authority", args.authority);
  const { ata, createIx } = await ensureKassAta(args.connection, authority, addr("kassMint", args.kassMint));
  const ix = await claimFact({
    nonce,
    fact: args.fact,
    destKass: ata,
    rentRecipient: authority,
    programId: args.programId,
  });
  return createIx ? [createIx, ix] : [ix];
}

// ---------------------------------------------------------------------------
// claim_fact_vote — vote stake (±slash) + fact reward → ATA(voter), close.
// ---------------------------------------------------------------------------
export interface BuildClaimFactVoteArgs {
  connection: Connection;
  oracleNonce: bigint | number;
  /** The FactVote PDA being claimed + closed (`[b"vote", fact, voter]`). */
  factVote: AddressInput;
  /** The fact this vote belongs to (`fact_vote.fact`); writable in the ix. */
  fact: AddressInput;
  /** `fact_vote.voter` — the KASS payout owner + rent recipient. */
  voter: AddressInput;
  kassMint: AddressInput;
  programId?: Address;
}

export async function buildClaimFactVoteIxs(
  args: BuildClaimFactVoteArgs,
): Promise<TransactionInstruction[]> {
  const nonce = requireNonce(args.oracleNonce);
  const voter = addr("voter", args.voter);
  const { ata, createIx } = await ensureKassAta(args.connection, voter, addr("kassMint", args.kassMint));
  const ix = await claimFactVote({
    nonce,
    factVote: args.factVote,
    fact: args.fact,
    destKass: ata,
    rentRecipient: voter,
    programId: args.programId,
  });
  return createIx ? [createIx, ix] : [ix];
}

// ---------------------------------------------------------------------------
// close_ai_claim — permissionless; rent → ai_claim.authority. No nonce, no ATA.
// ---------------------------------------------------------------------------
export interface BuildCloseAiClaimArgs {
  /** The terminal oracle (read-only in the ix). */
  oracle: AddressInput;
  /** The AiClaim PDA being closed (`detail.aiClaims[].pubkey`). */
  aiClaim: AddressInput;
  /** Rent recipient (`== ai_claim.authority`). */
  rentRecipient: AddressInput;
  programId?: Address;
}

export async function buildCloseAiClaimIxs(
  args: BuildCloseAiClaimArgs,
): Promise<TransactionInstruction[]> {
  const ix = await closeAiClaim({
    oracle: args.oracle,
    aiClaim: args.aiClaim,
    rentRecipient: args.rentRecipient,
    programId: args.programId,
  });
  return [ix];
}

// ---------------------------------------------------------------------------
// close_market — permissionless; closes the settled Market + escrow, rent →
// market.challenger. Carries the oracle nonce (the vault-authority signer).
// ---------------------------------------------------------------------------
export interface BuildCloseMarketArgs {
  oracleNonce: bigint | number;
  /** The settled Market PDA (derives the escrow vault in the SDK). */
  market: AddressInput;
  /** Rent recipient (`== market.challenger`). */
  rentRecipient: AddressInput;
  programId?: Address;
}

export async function buildCloseMarketIxs(
  args: BuildCloseMarketArgs,
): Promise<TransactionInstruction[]> {
  const nonce = requireNonce(args.oracleNonce);
  const ix = await closeMarket({
    nonce,
    market: args.market,
    rentRecipient: args.rentRecipient,
    programId: args.programId,
  });
  return [ix];
}

// ---------------------------------------------------------------------------
// sweep_oracle — permissionless, grace-gated; residual vault dust → DAO treasury
// (= ATA(daoAuthority, kassMint), derived in the SDK), then vault + Oracle closed
// with both rents refunded to `creator`.
// ---------------------------------------------------------------------------
export interface BuildSweepOracleArgs {
  oracleNonce: bigint | number;
  /** `Protocol.kass_mint` — the vault/treasury mint; derives the treasury ATA. */
  kassMint: AddressInput;
  /** `Protocol.dao_authority` — owner of the treasury ATA. */
  daoAuthority: AddressInput;
  /** Rent recipient for both reclaimed rents (`== oracle.creator`). */
  creator: AddressInput;
  programId?: Address;
}

/**
 * Resolve the DAO treasury owner (`Protocol.dao_authority`) from the on-chain
 * Protocol singleton — the sweep target owner. Encapsulates the protocol PDA
 * derivation + decode so callers (e.g. the sweep control) don't reach into the
 * SDK's `pda` / `decodeProtocol` directly. Throws a clear error if the protocol
 * account is missing or governance has not been linked yet.
 */
export async function resolveDaoAuthority(conn: Connection): Promise<Address> {
  const protocolPda = (await pda.protocol()).address;
  const info = await conn.getAccountInfo(protocolPda);
  if (!info || info.data.length === 0) {
    throw new Error(
      "Protocol account not found — governance is not initialized on this cluster.",
    );
  }
  const protocol = decodeProtocol(info.data);
  if (!protocol.governanceSet) {
    throw new Error("Governance is not set yet — the DAO treasury sweep is unavailable.");
  }
  return protocol.daoAuthority;
}

export async function buildSweepOracleIxs(
  args: BuildSweepOracleArgs,
): Promise<TransactionInstruction[]> {
  const nonce = requireNonce(args.oracleNonce);
  const ix = await sweepOracle({
    nonce,
    kassMint: args.kassMint,
    daoAuthority: args.daoAuthority,
    creator: args.creator,
    programId: args.programId,
  });
  return [ix];
}

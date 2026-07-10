/**
 * RF1 — the FINALIZE / CRANK action layer (pure ix-builders, NO React).
 *
 * Five permissionless "crank" builders that advance an oracle through its
 * dispute phases by assembling the `@kassandra-market/oracles` finalize builders with the
 * oracle + its child tail (the proposer / fact PDAs the read layer
 * `fetchOracleDetail` already returns):
 *
 *   Proposal      → {@link buildFinalizeProposalsIxs}  (FULL proposer set, ro tail)
 *   FactProposal  → {@link buildAdvancePhaseIxs}       (no tail)
 *   FactVoting    → {@link buildFinalizeFactsIxs}       (fact subset, writable tail)
 *   AiClaim       → {@link buildFinalizeAiClaimsIxs}    (proposer subset, writable tail)
 *   Challenge     → {@link buildFinalizeOracleIxs}      (FULL proposer set, ro tail)
 *
 * These are CRANK-like: the finalize/advance instructions carry NO required
 * signer — the connected wallet (or a test keypair) is only the fee-payer, so
 * ANY party may crank. The same builder drives the UI (WF2) and the gated
 * surfpool E2E (a keypair), exactly like the WF1 write actions.
 *
 * --- near-cap v0/ALT (the FULL-set finalizes) ---
 * `finalize_proposals` / `finalize_oracle` thread the FULL proposer set (up to
 * `MAX_PROPOSERS = 60`) as a READ-ONLY account tail. Past ~28 keys a LEGACY tx's
 * compiled message exceeds the 1232-byte packet, so each of those two builders
 * returns a {@link FinalizeAction} carrying `needsAlt` + the `altAddresses` to
 * pack into an Address Lookup Table. {@link sendFinalizeSmart} then picks the
 * legacy path (small tail) or the SDK's v0/ALT path (`sendFinalizeViaAlt`, a
 * keypair-driven multi-tx setup) automatically. The subset-capable finalizes
 * (`finalize_facts` / `finalize_ai_claims`) are chunked instead (call again with
 * the next subset), so they never set `needsAlt`.
 *
 * --- the oracle nonce ---
 * `finalize_facts` / `finalize_oracle` need the oracle's `nonce` (payload +
 * re-derives the oracle-PDA signer for the burn-back). The nonce is NOT stored
 * on the Oracle account and cannot be sourced from the read layer, so it is
 * recovered by {@link resolveOracleNonce} — a PURE (no-RPC) scan that re-derives
 * `[b"oracle", nonce_le]` for `nonce = 0..maxNonce` until it matches the oracle
 * pubkey. Callers that already know the nonce (tests, a create flow) pass it
 * explicitly and skip the scan.
 */
import {
  Address,
  ComputeBudgetProgram,
  type Connection,
  type Keypair,
  type TransactionInstruction,
} from "@solana/web3.js";
import {
  advancePhase,
  finalizeAiClaims,
  finalizeFacts,
  finalizeOracle,
  finalizeProposals,
  pda,
  sendFinalizeViaAlt,
} from "@kassandra-market/oracles";
import { ValidationError, type AddressInput } from "../actions";
import { sendAndConfirm, type SendResult, type TxSender } from "../send";

/** Coerce an {@link AddressInput} into an `Address`. */
function addr(a: AddressInput): Address {
  return a instanceof Address ? a : new Address(a);
}

/**
 * The read-only proposer-tail length beyond which a LEGACY `finalize_proposals`
 * / `finalize_oracle` transaction overflows the 1232-byte packet (each inlined
 * key is 32 bytes; the empirical overflow is ~28). Conservative — leaves
 * headroom for `finalize_oracle`'s 4 leading accounts — so anything strictly
 * greater is routed through the v0/ALT path.
 */
export const MAX_LEGACY_TAIL = 24;

/** Default upper bound for the pure {@link resolveOracleNonce} PDA scan. */
export const DEFAULT_MAX_NONCE_SCAN = 4096;

/**
 * The output of a finalize builder: the instruction(s) for a LEGACY send, plus
 * whether the read-only tail is large enough to REQUIRE the v0/ALT path and the
 * addresses to pack into the ALT. For builders that never overflow (advance /
 * the subset-capable finalizes) `needsAlt` is `false` and `altAddresses` empty.
 */
export interface FinalizeAction {
  /** The finalize/advance instruction(s) — a single ix, ready for a legacy tx. */
  ixs: TransactionInstruction[];
  /** True when the tail overflows a legacy tx and must be sent via v0/ALT. */
  needsAlt: boolean;
  /** The read-only tail addresses to pack into an ALT (empty when `!needsAlt`). */
  altAddresses: Address[];
}

/** The near-cap decision for a FULL read-only tail (finalizeProposals/Oracle). */
function altDecision(tail: Address[]): { needsAlt: boolean; altAddresses: Address[] } {
  const needsAlt = tail.length > MAX_LEGACY_TAIL;
  return { needsAlt, altAddresses: needsAlt ? tail : [] };
}

/** Require a non-empty account tail, else a typed {@link ValidationError}. */
function requireTail(field: string, tail: ReadonlyArray<AddressInput>): Address[] {
  if (tail.length === 0) {
    throw new ValidationError(field, `${field} must not be empty for this finalize.`);
  }
  return tail.map(addr);
}

/** Thrown by {@link resolveOracleNonce} when the nonce is not found within `maxNonce`. */
export class OracleNonceUnresolvedError extends Error {
  readonly oracle: string;
  readonly maxNonce: number;
  constructor(oracle: string, maxNonce: number) {
    super(
      `Could not resolve the nonce for oracle ${oracle} by scanning 0..${maxNonce}. ` +
        `Pass an explicit oracleNonce (the oracle account does not store it).`,
    );
    this.name = "OracleNonceUnresolvedError";
    this.oracle = oracle;
    this.maxNonce = maxNonce;
  }
}

/**
 * Recover an oracle's `nonce` by re-deriving `[b"oracle", nonce_le]` for
 * `nonce = 0..maxNonce` and matching the oracle pubkey. PURE (no RPC) — the
 * derivation is local. The nonce is not stored on-chain, so this is the only way
 * to source it for `finalize_facts` / `finalize_oracle` from a browsed oracle;
 * oracles created with a nonce beyond `maxNonce` must supply it explicitly.
 */
export async function resolveOracleNonce(
  oracle: AddressInput,
  opts?: { maxNonce?: number; programId?: Address },
): Promise<bigint> {
  const target = addr(oracle).toString();
  const max = opts?.maxNonce ?? DEFAULT_MAX_NONCE_SCAN;
  for (let n = 0; n <= max; n++) {
    const derived = (await pda.oracle(n, opts?.programId)).address;
    if (derived.toString() === target) return BigInt(n);
  }
  throw new OracleNonceUnresolvedError(target, max);
}

// ---------------------------------------------------------------------------
// finalize_proposals (Proposal → FactProposal / Resolved) — FULL ro tail.
// ---------------------------------------------------------------------------
export interface BuildFinalizeProposalsArgs {
  /** The oracle to finalize (past its proposal window). */
  oracle: AddressInput;
  /** The FULL proposer-PDA set (`fetchOracleDetail(...).proposers[].pubkey`). */
  proposers: ReadonlyArray<AddressInput>;
  programId?: Address;
}

export async function buildFinalizeProposalsIxs(
  args: BuildFinalizeProposalsArgs,
): Promise<FinalizeAction> {
  const proposers = requireTail("proposers", args.proposers);
  const ix = await finalizeProposals({
    oracle: args.oracle,
    proposers,
    programId: args.programId,
  });
  return { ixs: [ix], ...altDecision(proposers) };
}

// ---------------------------------------------------------------------------
// advance_phase (FactProposal → FactVoting) — no tail, never near-cap.
// ---------------------------------------------------------------------------
export interface BuildAdvancePhaseArgs {
  /** The oracle to tick (FactProposal window elapsed). */
  oracle: AddressInput;
  programId?: Address;
}

export async function buildAdvancePhaseIxs(
  args: BuildAdvancePhaseArgs,
): Promise<FinalizeAction> {
  const ix = await advancePhase({ oracle: args.oracle, programId: args.programId });
  return { ixs: [ix], needsAlt: false, altAddresses: [] };
}

// ---------------------------------------------------------------------------
// finalize_facts (FactVoting → AiClaim) — WRITABLE subset tail (chunk, no ALT).
// ---------------------------------------------------------------------------
export interface BuildFinalizeFactsArgs {
  /** The oracle to finalize (past its fact-voting window). */
  oracle: AddressInput;
  /** Canonical KASS mint (`oracle.kassMint` from the read layer). */
  kassMint: AddressInput;
  /**
   * The writable tail — a non-empty subset of the oracle's Fact PDAs
   * (`detail.facts[].pubkey`), or its Proposer PDAs in the no-facts dead-end.
   * Large sets are chunked: call again with the next subset.
   */
  facts: ReadonlyArray<AddressInput>;
  /** The oracle nonce; resolved via {@link resolveOracleNonce} when omitted. */
  oracleNonce?: bigint | number;
  /** Scan bound for the nonce resolution (default {@link DEFAULT_MAX_NONCE_SCAN}). */
  maxNonce?: number;
  programId?: Address;
}

export async function buildFinalizeFactsIxs(
  args: BuildFinalizeFactsArgs,
): Promise<FinalizeAction> {
  const tail = requireTail("facts", args.facts);
  const nonce =
    args.oracleNonce ??
    (await resolveOracleNonce(args.oracle, {
      maxNonce: args.maxNonce,
      programId: args.programId,
    }));
  const ix = await finalizeFacts({
    nonce,
    kassMint: args.kassMint,
    tail,
    programId: args.programId,
  });
  return { ixs: [ix], needsAlt: false, altAddresses: [] };
}

// ---------------------------------------------------------------------------
// finalize_ai_claims (AiClaim → Challenge) — WRITABLE subset tail (chunk, no ALT).
// ---------------------------------------------------------------------------
export interface BuildFinalizeAiClaimsArgs {
  /** The oracle to finalize (past its ai-claim window). */
  oracle: AddressInput;
  /** A non-empty subset of the oracle's Proposer PDAs (`detail.proposers[].pubkey`). */
  proposers: ReadonlyArray<AddressInput>;
  programId?: Address;
}

export async function buildFinalizeAiClaimsIxs(
  args: BuildFinalizeAiClaimsArgs,
): Promise<FinalizeAction> {
  const proposers = requireTail("proposers", args.proposers);
  const ix = await finalizeAiClaims({
    oracle: args.oracle,
    proposers,
    programId: args.programId,
  });
  return { ixs: [ix], needsAlt: false, altAddresses: [] };
}

// ---------------------------------------------------------------------------
// finalize_oracle (Challenge / FinalRecompute → Resolved) — FULL ro tail.
// ---------------------------------------------------------------------------
export interface BuildFinalizeOracleArgs {
  /** The oracle to finalize (past its challenge window, no open markets). */
  oracle: AddressInput;
  /** Canonical KASS mint (`oracle.kassMint` from the read layer). */
  kassMint: AddressInput;
  /** The FULL proposer-PDA set (`detail.proposers[].pubkey`), read-only tail. */
  proposers: ReadonlyArray<AddressInput>;
  /** The oracle nonce; resolved via {@link resolveOracleNonce} when omitted. */
  oracleNonce?: bigint | number;
  /** Scan bound for the nonce resolution (default {@link DEFAULT_MAX_NONCE_SCAN}). */
  maxNonce?: number;
  programId?: Address;
}

export async function buildFinalizeOracleIxs(
  args: BuildFinalizeOracleArgs,
): Promise<FinalizeAction> {
  const proposers = requireTail("proposers", args.proposers);
  const nonce =
    args.oracleNonce ??
    (await resolveOracleNonce(args.oracle, {
      maxNonce: args.maxNonce,
      programId: args.programId,
    }));
  const ix = await finalizeOracle({
    nonce,
    kassMint: args.kassMint,
    proposers,
    programId: args.programId,
  });
  return { ixs: [ix], ...altDecision(proposers) };
}

// ---------------------------------------------------------------------------
// send: legacy-or-ALT, picked by the tail size.
// ---------------------------------------------------------------------------
export interface SendFinalizeSmartArgs {
  connection: Connection;
  /** The built finalize action (from any of the builders above). */
  action: FinalizeAction;
  /** A legacy sender (wallet- or keypair-backed) — used when `!needsAlt`. */
  sender?: TxSender;
  /**
   * Fee-payer + ALT authority for the v0/ALT path (required when `needsAlt`).
   * The ALT setup is a keypair-driven multi-tx sequence, so the near-cap path is
   * keypair/CLI-only — a browser wallet uses the legacy path (small tails).
   */
  altKeypair?: Keypair;
  /** Compute-unit limit for the ALT finalize (the full-set loop needs > 200k default). */
  computeUnitLimit?: number;
  /** Confirm callback for the ALT setup txs (defaults to the SDK poll over `connection`). */
  confirm?: (signature: string) => Promise<void>;
}

/**
 * Send a {@link FinalizeAction}, automatically picking the LEGACY path (a single
 * tx via `sender`) or the v0/ALT path (`sendFinalizeViaAlt`, via `altKeypair`)
 * from `action.needsAlt`. Returns the confirmed signature.
 */
export async function sendFinalizeSmart(args: SendFinalizeSmartArgs): Promise<SendResult> {
  if (args.action.needsAlt) {
    if (!args.altKeypair) {
      throw new Error(
        "This finalize's proposer tail exceeds the legacy transaction limit; " +
          "supply altKeypair to send it via the v0/ALT path.",
      );
    }
    const { signature } = await sendFinalizeViaAlt({
      connection: args.connection,
      payer: args.altKeypair,
      // The finalize ix is the last (and only) instruction in the action.
      instruction: args.action.ixs[args.action.ixs.length - 1],
      lookupAddresses: args.action.altAddresses,
      prependInstructions: [
        ComputeBudgetProgram.setComputeUnitLimit({ units: args.computeUnitLimit ?? 600_000 }),
      ],
      confirm: args.confirm,
    });
    return { signature };
  }
  if (!args.sender) throw new Error("A legacy sender is required to send this finalize.");
  return sendAndConfirm(args.connection, args.sender, args.action.ixs);
}

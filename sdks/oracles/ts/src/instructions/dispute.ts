/**
 * D3b — instruction builders for the dispute round: fact submission/voting,
 * fact + AI-claim finalization, AI-claim submission, and the final oracle
 * recompute.
 *
 * Same conventions as `lifecycle.ts` (D3a): classic v3 `TransactionInstruction`,
 * `{pubkey, isSigner, isWritable}` metas in the EXACT processor order, and
 * `data = [disc, ...payload_LE]`. Account orders + payload layouts are mirrored
 * from each processor's `# Accounts` / `# Instruction payload` header AND the
 * test-harness `*_ix` builders (`programs/oracles/tests/{settlement_e2e,
 * common/mod}.rs`). PDAs are derived internally via `../pda.js`.
 */
import { Address, TransactionInstruction } from "@solana/web3.js";

import { Ix, KASSANDRA_PROGRAM_ID, SYSTEM_PROGRAM_ID, TOKEN_PROGRAM_ID } from "../constants.js";
import * as pda from "../pda.js";
import type { AddressInput } from "../pda.js";
import { addr, fixedBytes, ro, u16LE, u64LE, u8, w, withDisc } from "./payload.js";

const enc = new TextEncoder();

/** Coerce a `string`/`Uint8Array` uri into raw bytes (utf-8 for strings). */
function uriBytes(uri: Uint8Array | string): Uint8Array {
  return typeof uri === "string" ? enc.encode(uri) : uri;
}

// ---------------------------------------------------------------------------
// SubmitFact (Ix=0) — processor/submit_fact.rs
// Accounts: 0 oracle(w) 1 fact(w,PDA) 2 submitter(w,signer) 3 submitter_kass(w)
//           4 stake_vault(w,PDA) 5 token program(ro) 6 system program(ro).
// Payload: content_hash[32] ++ stake u64 ++ uri_len u16 ++ uri[uri_len].
// ---------------------------------------------------------------------------
export interface SubmitFactArgs {
  /** The oracle being supported (must be in `FactProposal`). */
  oracle: AddressInput;
  /** Submitter (signer): pays the Fact rent + authorizes the stake transfer. */
  submitter: AddressInput;
  /** Submitter's KASS token account — the stake source. */
  submitterKass: AddressInput;
  /** 32-byte fact content hash (seeds the Fact PDA `[b"fact", oracle, hash]`). */
  contentHash: Uint8Array;
  /** KASS stake escrowed for the fact (> 0). */
  stake: bigint | number;
  /** Fact uri (<= 200 bytes); a string is utf-8 encoded. */
  uri: Uint8Array | string;
  programId?: Address;
}

export async function submitFact(args: SubmitFactArgs): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  const oracle = addr(args.oracle);
  const fact = await pda.fact(oracle, fixedBytes(args.contentHash, 32), programId);
  const stakeVault = await pda.stakeVault(oracle, programId);
  const uri = uriBytes(args.uri);

  const data = withDisc(
    Ix.SubmitFact,
    fixedBytes(args.contentHash, 32),
    u64LE(args.stake),
    u16LE(uri.length),
    uri,
  );

  return new TransactionInstruction({
    programId,
    keys: [
      w(oracle),
      w(fact.address),
      w(addr(args.submitter), true),
      w(addr(args.submitterKass)),
      w(stakeVault.address),
      ro(TOKEN_PROGRAM_ID),
      ro(SYSTEM_PROGRAM_ID),
    ],
    data,
  });
}

// ---------------------------------------------------------------------------
// VoteFact (Ix=1) — processor/vote_fact.rs
// Accounts: 0 oracle(w) 1 fact(w) 2 fact_vote(w,PDA) 3 voter(w,signer)
//           4 voter_kass(w) 5 stake_vault(w,PDA) 6 token program(ro) 7 system(ro).
// Payload: kind u8 ++ stake u64.
// ---------------------------------------------------------------------------
export interface VoteFactArgs {
  /** The oracle (must be in `FactVoting`). */
  oracle: AddressInput;
  /** The fact being voted on (`fact.oracle == oracle`). */
  fact: AddressInput;
  /** Voter (signer): pays the FactVote rent + authorizes the stake transfer. */
  voter: AddressInput;
  /** Voter's KASS token account — the stake source. */
  voterKass: AddressInput;
  /** `VOTE_APPROVE = 0` / `VOTE_DUPLICATE = 1`. */
  kind: number;
  /** KASS stake escrowed for the vote (> 0). */
  stake: bigint | number;
  programId?: Address;
}

export async function voteFact(args: VoteFactArgs): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  const oracle = addr(args.oracle);
  const fact = addr(args.fact);
  const factVote = await pda.factVote(fact, addr(args.voter), programId);
  const stakeVault = await pda.stakeVault(oracle, programId);

  const data = withDisc(Ix.VoteFact, u8(args.kind), u64LE(args.stake));

  return new TransactionInstruction({
    programId,
    keys: [
      w(oracle),
      w(fact),
      w(factVote.address),
      w(addr(args.voter), true),
      w(addr(args.voterKass)),
      w(stakeVault.address),
      ro(TOKEN_PROGRAM_ID),
      ro(SYSTEM_PROGRAM_ID),
    ],
    data,
  });
}

// ---------------------------------------------------------------------------
// FinalizeFacts (Ix=2) — processor/finalize_facts.rs
// Accounts: 0 oracle(w) 1 kass_mint(w) 2 stake_vault(w,PDA) 3 token program(ro),
//           then a WRITABLE tail (a non-empty subset of the oracle's facts, or
//           its proposers in the no-facts dead-end branch).
// Payload: oracle_nonce u64 (re-derives the oracle PDA signer for the no-facts
//          dead-end `bond_pool` + emission burn-back).
// ---------------------------------------------------------------------------
export interface FinalizeFactsArgs {
  /** Oracle nonce — payload + derives the oracle/stake_vault PDAs. */
  nonce: bigint | number;
  /** Canonical KASS mint (`== oracle.kass_mint`); the no-facts dead-end burn target. */
  kassMint: AddressInput;
  /**
   * The tail: a non-empty subset of the oracle's Fact PDAs, or (when the oracle
   * has no facts) a subset of its Proposer PDAs. Each is writable.
   */
  tail: ReadonlyArray<AddressInput>;
  programId?: Address;
}

export async function finalizeFacts(args: FinalizeFactsArgs): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  const oracle = await pda.oracle(BigInt(args.nonce), programId);
  const stakeVault = await pda.stakeVault(oracle.address, programId);

  return new TransactionInstruction({
    programId,
    keys: [
      w(oracle.address),
      w(addr(args.kassMint)),
      w(stakeVault.address),
      ro(TOKEN_PROGRAM_ID),
      ...args.tail.map((k) => w(addr(k))),
    ],
    data: withDisc(Ix.FinalizeFacts, u64LE(args.nonce)),
  });
}

// ---------------------------------------------------------------------------
// SubmitAiClaim (Ix=3) — processor/submit_ai_claim.rs
// Accounts: 0 oracle(w) 1 proposer(w) 2 ai_claim(w,PDA) 3 authority(w,signer)
//           4 system program(ro).
// Payload: model_id[32] ++ params_hash[32] ++ io_hash[32] ++ option u8.
// ---------------------------------------------------------------------------
export interface SubmitAiClaimArgs {
  /** The oracle (must be in `AiClaim`). */
  oracle: AddressInput;
  /** The submitter's Proposer PDA (`proposer.authority == authority`). */
  proposer: AddressInput;
  /** Proposer authority (signer): must equal `proposer.authority`, pays rent. */
  authority: AddressInput;
  /** 32-byte pinned model id. */
  modelId: Uint8Array;
  /** 32-byte model-params hash. */
  paramsHash: Uint8Array;
  /** 32-byte input/output hash. */
  ioHash: Uint8Array;
  /** The claimed categorical option (< oracle.options_count). */
  option: number;
  programId?: Address;
}

export async function submitAiClaim(args: SubmitAiClaimArgs): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  const oracle = addr(args.oracle);
  const proposer = addr(args.proposer);
  const aiClaim = await pda.aiClaim(oracle, proposer, programId);

  const data = withDisc(
    Ix.SubmitAiClaim,
    fixedBytes(args.modelId, 32),
    fixedBytes(args.paramsHash, 32),
    fixedBytes(args.ioHash, 32),
    u8(args.option),
  );

  return new TransactionInstruction({
    programId,
    keys: [
      w(oracle),
      w(proposer),
      w(aiClaim.address),
      w(addr(args.authority), true),
      ro(SYSTEM_PROGRAM_ID),
    ],
    data,
  });
}

// ---------------------------------------------------------------------------
// FinalizeAiClaims (Ix=8) — processor/finalize_ai_claims.rs
// Accounts: 0 oracle(w), then a WRITABLE tail (a non-empty subset of the
//           oracle's proposers). Payload: empty.
// ---------------------------------------------------------------------------
export interface FinalizeAiClaimsArgs {
  /** The oracle to finalize (must be in `AiClaim`, window elapsed). */
  oracle: AddressInput;
  /** A non-empty subset of the oracle's Proposer PDAs, each writable. */
  proposers: ReadonlyArray<AddressInput>;
  programId?: Address;
}

export async function finalizeAiClaims(
  args: FinalizeAiClaimsArgs,
): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  return new TransactionInstruction({
    programId,
    keys: [w(addr(args.oracle)), ...args.proposers.map((p) => w(addr(p)))],
    data: withDisc(Ix.FinalizeAiClaims),
  });
}

// ---------------------------------------------------------------------------
// FinalizeOracle (Ix=6) — processor/finalize_oracle.rs
// Accounts: 0 oracle(w) 1 kass_mint(w) 2 stake_vault(w,PDA) 3 token program(ro),
//           then the FULL proposer set as a READ-ONLY tail (`proposer_count`).
// Payload: oracle_nonce u64 (re-derives the oracle PDA signer for the
//          InvalidDeadend emission burn-back).
// ---------------------------------------------------------------------------
export interface FinalizeOracleArgs {
  /** Oracle nonce — payload + derives the oracle/stake_vault PDAs. */
  nonce: bigint | number;
  /** Canonical KASS mint (`== oracle.kass_mint`); the burn-back target. */
  kassMint: AddressInput;
  /** The FULL proposer-PDA set (exactly `proposer_count`), each read-only. */
  proposers: ReadonlyArray<AddressInput>;
  programId?: Address;
}

export async function finalizeOracle(args: FinalizeOracleArgs): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  const oracle = await pda.oracle(BigInt(args.nonce), programId);
  const stakeVault = await pda.stakeVault(oracle.address, programId);

  return new TransactionInstruction({
    programId,
    keys: [
      w(oracle.address),
      w(addr(args.kassMint)),
      w(stakeVault.address),
      ro(TOKEN_PROGRAM_ID),
      ...args.proposers.map((p) => ro(addr(p))),
    ],
    data: withDisc(Ix.FinalizeOracle, u64LE(args.nonce)),
  });
}

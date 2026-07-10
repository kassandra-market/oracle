/**
 * D3b — instruction builders for the staker-settlement layer (Task S2/S4):
 * the per-staker pull claims (`claim_proposer` / `claim_fact` /
 * `claim_fact_vote`) and the post-resolution account closes (`close_ai_claim` /
 * `close_market`).
 *
 * All three claims + `close_market` carry `oracle_nonce: u64 LE` (re-derives the
 * oracle PDA, the SPL authority of `stake_vault` / escrow that signs the
 * payouts). `close_ai_claim` has an EMPTY payload. The SDK derives the oracle +
 * stake_vault from the nonce (and the escrow vault for `close_market`); the
 * claimant/destination accounts are caller-supplied. Account orders mirror the
 * harness `*_ix` helpers in `programs/kassandra/tests/common/mod.rs`.
 */
import { Address, TransactionInstruction } from "@solana/web3.js";

import { Ix, KASSANDRA_PROGRAM_ID, TOKEN_PROGRAM_ID } from "../constants.js";
import * as pda from "../pda.js";
import type { AddressInput } from "../pda.js";
import { addr, ro, u64LE, w, withDisc } from "./payload.js";

// ---------------------------------------------------------------------------
// ClaimProposer (Ix=17) — processor/claims.rs
// Accounts: 0 oracle(ro) 1 proposer(w,closed) 2 dest_kass(w) 3 stake_vault(w,PDA)
//           4 rent_recipient(w) 5 token program(ro). Payload: oracle_nonce u64.
// ---------------------------------------------------------------------------
export interface ClaimProposerArgs {
  /** Oracle nonce — payload + derives the oracle/stake_vault PDAs. */
  nonce: bigint | number;
  /** The Proposer PDA being claimed + closed. */
  proposer: AddressInput;
  /** KASS payout dest (mint==oracle.kass_mint, owner==proposer.authority). */
  destKass: AddressInput;
  /** Rent recipient (`== proposer.authority`). */
  rentRecipient: AddressInput;
  programId?: Address;
}

export async function claimProposer(args: ClaimProposerArgs): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  const oracle = await pda.oracle(BigInt(args.nonce), programId);
  const stakeVault = await pda.stakeVault(oracle.address, programId);

  return new TransactionInstruction({
    programId,
    keys: [
      ro(oracle.address),
      w(addr(args.proposer)),
      w(addr(args.destKass)),
      w(stakeVault.address),
      w(addr(args.rentRecipient)),
      ro(TOKEN_PROGRAM_ID),
    ],
    data: withDisc(Ix.ClaimProposer, u64LE(args.nonce)),
  });
}

// ---------------------------------------------------------------------------
// ClaimFact (Ix=18) — processor/claims.rs
// Accounts: 0 oracle(ro) 1 fact(w,closed) 2 dest_kass(w) 3 stake_vault(w,PDA)
//           4 rent_recipient(w) 5 token program(ro). Payload: oracle_nonce u64.
// ---------------------------------------------------------------------------
export interface ClaimFactArgs {
  /** Oracle nonce — payload + derives the oracle/stake_vault PDAs. */
  nonce: bigint | number;
  /** The Fact PDA being claimed + closed. */
  fact: AddressInput;
  /** KASS payout dest (mint==oracle.kass_mint, owner==fact.proposer). */
  destKass: AddressInput;
  /** Rent recipient (`== fact.proposer`). */
  rentRecipient: AddressInput;
  programId?: Address;
}

export async function claimFact(args: ClaimFactArgs): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  const oracle = await pda.oracle(BigInt(args.nonce), programId);
  const stakeVault = await pda.stakeVault(oracle.address, programId);

  return new TransactionInstruction({
    programId,
    keys: [
      ro(oracle.address),
      w(addr(args.fact)),
      w(addr(args.destKass)),
      w(stakeVault.address),
      w(addr(args.rentRecipient)),
      ro(TOKEN_PROGRAM_ID),
    ],
    data: withDisc(Ix.ClaimFact, u64LE(args.nonce)),
  });
}

// ---------------------------------------------------------------------------
// ClaimFactVote (Ix=19) — processor/claims.rs
// Accounts: 0 oracle(ro) 1 fact_vote(w,closed) 2 fact(w) 3 dest_kass(w)
//           4 stake_vault(w,PDA) 5 rent_recipient(w) 6 token program(ro).
// Payload: oracle_nonce u64.
// ---------------------------------------------------------------------------
export interface ClaimFactVoteArgs {
  /** Oracle nonce — payload + derives the oracle/stake_vault PDAs. */
  nonce: bigint | number;
  /** The FactVote PDA being claimed + closed. */
  factVote: AddressInput;
  /** The fact this vote belongs to (`fact_vote.fact == fact`); writable. */
  fact: AddressInput;
  /** KASS payout dest (mint==oracle.kass_mint, owner==fact_vote.voter). */
  destKass: AddressInput;
  /** Rent recipient (`== fact_vote.voter`). */
  rentRecipient: AddressInput;
  programId?: Address;
}

export async function claimFactVote(args: ClaimFactVoteArgs): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  const oracle = await pda.oracle(BigInt(args.nonce), programId);
  const stakeVault = await pda.stakeVault(oracle.address, programId);

  return new TransactionInstruction({
    programId,
    keys: [
      ro(oracle.address),
      w(addr(args.factVote)),
      w(addr(args.fact)),
      w(addr(args.destKass)),
      w(stakeVault.address),
      w(addr(args.rentRecipient)),
      ro(TOKEN_PROGRAM_ID),
    ],
    data: withDisc(Ix.ClaimFactVote, u64LE(args.nonce)),
  });
}

// ---------------------------------------------------------------------------
// CloseAiClaim (Ix=20) — processor/close_ai_claim.rs
// Accounts: 0 oracle(ro) 1 ai_claim(w,closed) 2 rent_recipient(w). Payload: none.
// ---------------------------------------------------------------------------
export interface CloseAiClaimArgs {
  /** The terminal oracle (read-only). */
  oracle: AddressInput;
  /** The AiClaim account being closed. */
  aiClaim: AddressInput;
  /** Rent recipient (`== ai_claim.authority`). */
  rentRecipient: AddressInput;
  programId?: Address;
}

export async function closeAiClaim(args: CloseAiClaimArgs): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  return new TransactionInstruction({
    programId,
    keys: [
      ro(addr(args.oracle)),
      w(addr(args.aiClaim)),
      w(addr(args.rentRecipient)),
    ],
    data: withDisc(Ix.CloseAiClaim),
  });
}

// ---------------------------------------------------------------------------
// CloseMarket (Ix=21) — processor/close_market.rs
// Accounts: 0 oracle(ro) 1 market(w,closed) 2 challenger_usdc_vault(w,PDA,closed)
//           3 rent_recipient(w) 4 token program(ro). Payload: oracle_nonce u64.
// ---------------------------------------------------------------------------
export interface CloseMarketArgs {
  /** Oracle nonce — payload + derives the oracle PDA signer. */
  nonce: bigint | number;
  /** The settled Market PDA being closed (derives the escrow vault). */
  market: AddressInput;
  /** Rent recipient (`== market.challenger`). */
  rentRecipient: AddressInput;
  programId?: Address;
}

export async function closeMarket(args: CloseMarketArgs): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  const oracle = await pda.oracle(BigInt(args.nonce), programId);
  const market = addr(args.market);
  const escrowVault = await pda.challengeUsdcVault(market, programId);

  return new TransactionInstruction({
    programId,
    keys: [
      ro(oracle.address),
      w(market),
      w(escrowVault.address),
      w(addr(args.rentRecipient)),
      ro(TOKEN_PROGRAM_ID),
    ],
    data: withDisc(Ix.CloseMarket, u64LE(args.nonce)),
  });
}

// ---------------------------------------------------------------------------
// SweepOracle (Ix=22) — processor/sweep_oracle.rs
// Permissionless, grace-gated dust sweep + terminal Oracle/stake_vault closure.
// After the grace (now >= oracle.phase_ends_at + SWEEP_GRACE) the residual vault
// balance (bounded dust, or a no-show staker's FORFEITED principal) is
// transferred to the DAO treasury = ATA(dao_authority, kass_mint), then the vault
// and the Oracle are closed with both rents refunded to oracle.creator.
// Accounts: 0 oracle(w,closed) 1 stake_vault(w,PDA,closed) 2 protocol(ro)
//           3 dao_treasury(w) 4 creator(w) 5 token program(ro).
// Payload: oracle_nonce u64.
// ---------------------------------------------------------------------------
export interface SweepOracleArgs {
  /** Oracle nonce — payload + derives the oracle/stake_vault PDAs (the vault authority). */
  nonce: bigint | number;
  /** `Protocol.kass_mint` — the vault/treasury mint; derives the treasury ATA. */
  kassMint: AddressInput;
  /** `Protocol.dao_authority` (the Squads vault) — owner of the treasury ATA. */
  daoAuthority: AddressInput;
  /** Rent recipient for both reclaimed rents (`== oracle.creator`). */
  creator: AddressInput;
  programId?: Address;
}

export async function sweepOracle(args: SweepOracleArgs): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  const oracle = await pda.oracle(BigInt(args.nonce), programId);
  const stakeVault = await pda.stakeVault(oracle.address, programId);
  const protocol = await pda.protocol(programId);
  // DAO treasury = canonical KASS ATA of dao_authority (derived under the ATA
  // program, matching the in-program ATA(dao_authority, kass_mint) validation).
  const daoTreasury = await pda.associatedTokenAccount(args.daoAuthority, args.kassMint);

  return new TransactionInstruction({
    programId,
    keys: [
      w(oracle.address),
      w(stakeVault.address),
      ro(protocol.address),
      w(daoTreasury.address),
      w(addr(args.creator)),
      ro(TOKEN_PROGRAM_ID),
    ],
    data: withDisc(Ix.SweepOracle, u64LE(args.nonce)),
  });
}

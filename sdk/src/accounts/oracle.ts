/**
 * Decoder for the `Oracle` dispute account (`state.rs::Oracle`, 392 bytes) — the
 * largest Pod struct. Carries the dispute lifecycle state, the running tallies,
 * the governable-param snapshot taken at `create_oracle`, the challenge-fee
 * snapshot, and the settlement resolution totals. Field offsets are the EXACT
 * values pinned in `programs/kassandra/tests/state_layout.rs`.
 */
import { Address } from "@solana/web3.js";

import { AccountType, ACCOUNT_SIZES, Phase } from "../constants.js";
import {
  assertAccount,
  readI64LE,
  readPubkey,
  readU16LE,
  readU64LE,
  readU8,
  view,
} from "./common.js";

/** Decoded `Oracle` account. */
export interface Oracle {
  accountType: AccountType.Oracle;
  creator: Address;
  kassMint: Address;
  usdcMint: Address;
  /** PDA token account holding all KASS bonds/stakes. */
  stakeVault: Address;
  /** Unix deadline; proposals rejected before this. */
  deadline: bigint;
  /** End of the current phase window (unix). */
  phaseEndsAt: bigint;
  /** Per-oracle TWAP window (seconds). */
  twapWindow: bigint;
  /** Number of categorical options. */
  optionsCount: number;
  /** Raw stored phase discriminant. */
  phaseRaw: number;
  /** Decoded {@link Phase} (or undefined if the discriminant is unknown). */
  phase: Phase | undefined;
  proposerCount: number;
  /** Proposers not disqualified. */
  survivingCount: number;
  factCount: number;
  /** Conservation accumulator (KASS base units). */
  totalOracleStake: bigint;
  /** Accumulated slashed KASS (base units). */
  bondPool: bigint;
  /** Σ proposer bonds, fixed at dispute start; fact-quorum denominator. */
  disputeBondTotal: bigint;
  /** Facts settled so far. */
  settledCount: number;
  /** Proposers ai-finalized so far. */
  aiFinalizedCount: number;
  bump: number;
  /** Final resolved option — meaningful ONLY when `phase === Resolved` (0xFF sentinel on dead-end). */
  resolvedOption: number;
  /** Number of open (created-but-not-settled) challenge markets. */
  openChallengeCount: number;
  // Governable params snapshotted from Protocol at create_oracle.
  thresholdNum: bigint;
  thresholdDen: bigint;
  marketThresholdNum: bigint;
  marketThresholdDen: bigint;
  flipSlashNum: bigint;
  flipSlashDen: bigint;
  phaseWindow: bigint;
  proposalWindow: bigint;
  factVoteSlashNum: bigint;
  factVoteSlashDen: bigint;
  rewardProposerWeight: bigint;
  rewardFactWeight: bigint;
  // Challenge-fee config snapshot.
  challengeFailUsdcFeeNum: bigint;
  challengeFailUsdcFeeDen: bigint;
  challengeSuccessKassFeeNum: bigint;
  challengeSuccessKassFeeDen: bigint;
  // Settlement resolution totals (0 until resolution).
  totalCorrectProposerStake: bigint;
  totalApprovedFactStake: bigint;
  rewardPool: bigint;
  /** KASS emission minted into the stake vault at creation. */
  rewardEmission: bigint;
  /**
   * Activity-scaled minimum stake (KASS base units) for propose / submit_fact /
   * vote_fact on this oracle, snapshotted at creation. 0 at genesis / low activity
   * (free participation) or while the floor is disabled.
   */
  minStake: bigint;
}

/**
 * Decode an `Oracle` account from its raw bytes. Throws on wrong size or tag.
 */
export function decodeOracle(data: Uint8Array): Oracle {
  assertAccount(data, AccountType.Oracle, ACCOUNT_SIZES.Oracle, "Oracle");
  const dv = view(data);
  const phaseRaw = readU8(dv, 161);
  return {
    accountType: AccountType.Oracle,
    creator: readPubkey(data, 8),
    kassMint: readPubkey(data, 40),
    usdcMint: readPubkey(data, 72),
    stakeVault: readPubkey(data, 104),
    deadline: readI64LE(dv, 136),
    phaseEndsAt: readI64LE(dv, 144),
    twapWindow: readI64LE(dv, 152),
    optionsCount: readU8(dv, 160),
    phaseRaw,
    phase: phaseRaw in Phase ? (phaseRaw as Phase) : undefined,
    proposerCount: readU16LE(dv, 162),
    survivingCount: readU16LE(dv, 164),
    factCount: readU16LE(dv, 166),
    totalOracleStake: readU64LE(dv, 168),
    bondPool: readU64LE(dv, 176),
    disputeBondTotal: readU64LE(dv, 184),
    settledCount: readU16LE(dv, 192),
    aiFinalizedCount: readU16LE(dv, 194),
    bump: readU8(dv, 196),
    resolvedOption: readU8(dv, 197),
    openChallengeCount: readU16LE(dv, 198),
    // (former `prompt_hash` [u8;32] @200 removed — everything below shifted -32.)
    thresholdNum: readU64LE(dv, 200),
    thresholdDen: readU64LE(dv, 208),
    marketThresholdNum: readU64LE(dv, 216),
    marketThresholdDen: readU64LE(dv, 224),
    flipSlashNum: readU64LE(dv, 232),
    flipSlashDen: readU64LE(dv, 240),
    phaseWindow: readI64LE(dv, 248),
    proposalWindow: readI64LE(dv, 256),
    factVoteSlashNum: readU64LE(dv, 264),
    factVoteSlashDen: readU64LE(dv, 272),
    rewardProposerWeight: readU64LE(dv, 280),
    rewardFactWeight: readU64LE(dv, 288),
    challengeFailUsdcFeeNum: readU64LE(dv, 296),
    challengeFailUsdcFeeDen: readU64LE(dv, 304),
    challengeSuccessKassFeeNum: readU64LE(dv, 312),
    challengeSuccessKassFeeDen: readU64LE(dv, 320),
    totalCorrectProposerStake: readU64LE(dv, 328),
    totalApprovedFactStake: readU64LE(dv, 336),
    rewardPool: readU64LE(dv, 344),
    rewardEmission: readU64LE(dv, 352),
    minStake: readU64LE(dv, 360),
  };
}

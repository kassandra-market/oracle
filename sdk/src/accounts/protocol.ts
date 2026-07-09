/**
 * Decoder for the `Protocol` singleton account (`state.rs::Protocol`, 368 bytes).
 *
 * The program's global config record: canonical mints, the fee-EMA state, the
 * one-time governance linkage, and the governable monetary/behavioral/challenge
 * params. Field offsets are the EXACT values pinned in
 * `programs/kassandra/tests/state_layout.rs` (`field_offsets_are_pinned`).
 */
import { Address } from "@solana/web3.js";

import { AccountType, ACCOUNT_SIZES } from "../constants.js";
import {
  assertAccount,
  readBool,
  readI64LE,
  readPubkey,
  readU64LE,
  readU8,
  view,
} from "./common.js";

/** Decoded `Protocol` singleton. All `u64`/`i64` fields are `bigint`; mints/keys are `Address`. */
export interface Protocol {
  accountType: AccountType.Protocol;
  /** The initializer; gates the one-time `set_governance`. */
  admin: Address;
  /** Canonical KASS mint (oracles must match this). */
  kassMint: Address;
  /** Canonical USDC mint (oracles must match this). */
  usdcMint: Address;
  /** Fixed-point EMA accumulator of recent oracle-creation activity. */
  feeEma: bigint;
  /** Unix timestamp of the most recent oracle creation (0 at genesis). */
  lastCreationUnix: bigint;
  /** Protocol PDA bump. */
  bump: number;
  /** True once `set_governance` recorded the DAO linkage. */
  governanceSet: boolean;
  /** Squads v4 multisig vault PDA gating `set_config`/`resolve_deadend` (zero until set). */
  daoAuthority: Address;
  /** Futarchy `Dao` account whose embedded AMM is the KASS price source (zero until set). */
  kassDao: Address;
  /** Emission rate fraction `emission_num/emission_den`. */
  emissionNum: bigint;
  emissionDen: bigint;
  /** Hard cap on circulating KASS supply (0 = disabled). */
  totalSupplyCap: bigint;
  /** Fee-EMA params mirrored from `config.rs`. */
  feeEmaHalflife: bigint;
  feePerEmaUnit: bigint;
  feeEmaIncrement: bigint;
  /** Fact-quorum supermajority fraction `threshold_num/threshold_den`. */
  thresholdNum: bigint;
  thresholdDen: bigint;
  /** Market slash-trigger margin fraction. */
  marketThresholdNum: bigint;
  marketThresholdDen: bigint;
  /** Flip-slash fraction. */
  flipSlashNum: bigint;
  flipSlashDen: bigint;
  /** Dispute phase window (seconds). */
  phaseWindow: bigint;
  /** Proposal-registration window (seconds). */
  proposalWindow: bigint;
  /** Approve-voter rejected-fact slash fraction. */
  factVoteSlashNum: bigint;
  factVoteSlashDen: bigint;
  /** Reward bucket weights. */
  rewardProposerWeight: bigint;
  rewardFactWeight: bigint;
  /** USDC fee on a FAILED challenge (→ proposer). */
  challengeFailUsdcFeeNum: bigint;
  challengeFailUsdcFeeDen: bigint;
  /** KASS fee on a SUCCESSFUL challenge (→ challenger). */
  challengeSuccessKassFeeNum: bigint;
  challengeSuccessKassFeeDen: bigint;
  /** Activity-scaled stake-floor curve (bootstrapping). fee-EMA below which the
   * floor is 0; at which it reaches the max; and the max floor (0 = disabled). */
  stakeFloorEmaThreshold: bigint;
  stakeFloorEmaCap: bigint;
  stakeFloorMax: bigint;
}

/**
 * Decode a `Protocol` account from its raw bytes. Throws if the buffer is not
 * exactly {@link ACCOUNT_SIZES.Protocol} bytes or carries the wrong account_type tag.
 */
export function decodeProtocol(data: Uint8Array): Protocol {
  assertAccount(data, AccountType.Protocol, ACCOUNT_SIZES.Protocol, "Protocol");
  const dv = view(data);
  return {
    accountType: AccountType.Protocol,
    admin: readPubkey(data, 8),
    kassMint: readPubkey(data, 40),
    usdcMint: readPubkey(data, 72),
    feeEma: readU64LE(dv, 104),
    lastCreationUnix: readI64LE(dv, 112),
    bump: readU8(dv, 120),
    governanceSet: readBool(dv, 121),
    daoAuthority: readPubkey(data, 128),
    kassDao: readPubkey(data, 160),
    emissionNum: readU64LE(dv, 192),
    emissionDen: readU64LE(dv, 200),
    totalSupplyCap: readU64LE(dv, 208),
    feeEmaHalflife: readI64LE(dv, 216),
    feePerEmaUnit: readU64LE(dv, 224),
    feeEmaIncrement: readU64LE(dv, 232),
    thresholdNum: readU64LE(dv, 240),
    thresholdDen: readU64LE(dv, 248),
    marketThresholdNum: readU64LE(dv, 256),
    marketThresholdDen: readU64LE(dv, 264),
    flipSlashNum: readU64LE(dv, 272),
    flipSlashDen: readU64LE(dv, 280),
    phaseWindow: readI64LE(dv, 288),
    proposalWindow: readI64LE(dv, 296),
    factVoteSlashNum: readU64LE(dv, 304),
    factVoteSlashDen: readU64LE(dv, 312),
    rewardProposerWeight: readU64LE(dv, 320),
    rewardFactWeight: readU64LE(dv, 328),
    challengeFailUsdcFeeNum: readU64LE(dv, 336),
    challengeFailUsdcFeeDen: readU64LE(dv, 344),
    challengeSuccessKassFeeNum: readU64LE(dv, 352),
    challengeSuccessKassFeeDen: readU64LE(dv, 360),
    stakeFloorEmaThreshold: readU64LE(dv, 368),
    stakeFloorEmaCap: readU64LE(dv, 376),
    stakeFloorMax: readU64LE(dv, 384),
  };
}

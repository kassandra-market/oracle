/**
 * D3a — instruction builders for the protocol + oracle-lifecycle instructions.
 *
 * Each builder returns a `@solana/web3.js@3.0.0-rc.2` (classic API)
 * `TransactionInstruction` with:
 *   - `programId` = {@link KASSANDRA_PROGRAM_ID} (overridable per call),
 *   - `keys` = the EXACT account-meta list in the processor's documented order,
 *     each with the correct `isSigner`/`isWritable` role,
 *   - `data` = `[disc, ...payload_LE]`, mirroring the processor's payload bytes.
 *
 * The account orders + payload layouts are mirrored from each processor's
 * `# Accounts` / `# Instruction payload` module-doc header AND cross-checked
 * against the test harness `*_ix` builders in
 * `programs/oracles/tests/common/mod.rs` (the authoritative reference).
 *
 * PDAs are derived internally (via `../pda.js`) so callers pass only the
 * "real" pubkeys; derivation is async, so every builder is `async`.
 */
import { Address, TransactionInstruction } from "@solana/web3.js";
import type { AccountMeta } from "@solana/web3.js";

import {
  Ix,
  KASSANDRA_PROGRAM_ID,
  SYSTEM_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "../constants.js";
import * as pda from "../pda.js";
import type { AddressInput } from "../pda.js";
import {
  concatBytes,
  fixedBytes,
  i64LE,
  pubkeyBytes,
  u16LE,
  u64LE,
  u8,
  withDisc,
} from "./payload.js";

/** Coerce an `AddressInput` into a web3.js `Address`. */
function addr(a: AddressInput): Address {
  return a instanceof Address ? a : new Address(a);
}

/** Writable account meta. */
function w(pubkey: Address, isSigner = false): AccountMeta {
  return { pubkey, isSigner, isWritable: true };
}

/** Read-only account meta. */
function ro(pubkey: Address, isSigner = false): AccountMeta {
  return { pubkey, isSigner, isWritable: false };
}

// ---------------------------------------------------------------------------
// InitProtocol (Ix=9) — processor/init_protocol.rs
// Accounts: 0 protocol(w) 1 admin(w,signer) 2 kass_mint(ro) 3 usdc_mint(ro)
//           4 system program(ro). Payload: none.
// ---------------------------------------------------------------------------
export interface InitProtocolArgs {
  /** Admin (signer): tops up rent, recorded as `Protocol.admin`. */
  admin: AddressInput;
  /** Canonical KASS mint (SPL token-program owned). */
  kassMint: AddressInput;
  /** Canonical USDC mint (SPL token-program owned). */
  usdcMint: AddressInput;
  /** Override the program id (defaults to {@link KASSANDRA_PROGRAM_ID}). */
  programId?: Address;
}

export async function initProtocol(args: InitProtocolArgs): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  const protocol = await pda.protocol(programId);
  return new TransactionInstruction({
    programId,
    keys: [
      w(protocol.address),
      w(addr(args.admin), true),
      ro(addr(args.kassMint)),
      ro(addr(args.usdcMint)),
      ro(SYSTEM_PROGRAM_ID),
    ],
    data: withDisc(Ix.InitProtocol),
  });
}

// ---------------------------------------------------------------------------
// CreateOracle (Ix=10) — processor/create_oracle.rs
// Accounts: 0 protocol(w) 1 oracle(w,PDA) 2 stake_vault(w,PDA) 3 creator(w,signer)
//           4 kass_mint(w) 5 usdc_mint(ro) 6 token program(ro) 7 system program(ro)
//           8 creator_kass_token(w) 9 mint_authority(ro,PDA).
// Payload (57): nonce u64 ++ prompt_hash[32] ++ options_count u8 ++ deadline i64
//               ++ twap_window i64.
// ---------------------------------------------------------------------------
export interface CreateOracleArgs {
  /** Oracle nonce — seeds the oracle PDA `[b"oracle", nonce_le8]`. */
  nonce: bigint | number;
  /** Categorical option count (>= 2). */
  optionsCount: number;
  /** Creation-time deadline (unix seconds, i64). */
  deadline: bigint | number;
  /** TWAP window (seconds, i64 > 0). */
  twapWindow: bigint | number;
  /** Creator (signer): pays rent, recorded as creator, fee-burn authority. */
  creator: AddressInput;
  /** Creator's KASS token account the creation fee is burned from. */
  creatorKassToken: AddressInput;
  /** Canonical KASS mint (must equal `protocol.kass_mint`). */
  kassMint: AddressInput;
  /** Canonical USDC mint (must equal `protocol.usdc_mint`). */
  usdcMint: AddressInput;
  programId?: Address;
}

export async function createOracle(args: CreateOracleArgs): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  const protocol = await pda.protocol(programId);
  const oracle = await pda.oracle(BigInt(args.nonce), programId);
  const stakeVault = await pda.stakeVault(oracle.address, programId);
  const mintAuthority = await pda.mintAuthority(programId);

  const data = withDisc(
    Ix.CreateOracle,
    u64LE(args.nonce),
    u8(args.optionsCount),
    i64LE(args.deadline),
    i64LE(args.twapWindow),
  );

  return new TransactionInstruction({
    programId,
    keys: [
      w(protocol.address),
      w(oracle.address),
      w(stakeVault.address),
      w(addr(args.creator), true),
      w(addr(args.kassMint)),
      ro(addr(args.usdcMint)),
      ro(TOKEN_PROGRAM_ID),
      ro(SYSTEM_PROGRAM_ID),
      w(addr(args.creatorKassToken)),
      ro(mintAuthority.address),
    ],
    data,
  });
}

// ---------------------------------------------------------------------------
// WriteOracleMeta (Ix=23) — processor/write_oracle_meta.rs
// Accounts: 0 creator(w,signer) 1 oracle(ro) 2 oracle_meta(w,PDA) 3 system(ro).
// Body (length-prefixed): subject_len u16 ++ subject ++ options_count u8 ++
//   [option_len u16 ++ option]* ++ uri_len u16 ++ uri ++ uri_hash[32].
// ---------------------------------------------------------------------------
export interface WriteOracleMetaArgs {
  /** The oracle whose metadata is being written. */
  oracle: AddressInput;
  /** Creator (signer): must equal the oracle's recorded creator; pays the rent. */
  creator: AddressInput;
  /** The plaintext question (on-chain). */
  subject: string;
  /** The option labels (on-chain); count must equal the oracle's options_count. */
  options: string[];
  /** URL of the extended off-chain metadata JSON (may be empty). */
  uri: string;
  /** 32-byte `sha256` of the canonical off-chain JSON (zeroed if no uri). */
  uriHash: Uint8Array;
  programId?: Address;
}

export async function writeOracleMeta(
  args: WriteOracleMetaArgs,
): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  const meta = await pda.oracleMeta(addr(args.oracle), programId);

  const enc = new TextEncoder();
  const subject = enc.encode(args.subject);
  const parts: Uint8Array[] = [u16LE(subject.length), subject, u8(args.options.length)];
  for (const o of args.options) {
    const b = enc.encode(o);
    parts.push(u16LE(b.length), b);
  }
  const uri = enc.encode(args.uri);
  parts.push(u16LE(uri.length), uri, fixedBytes(args.uriHash, 32));

  return new TransactionInstruction({
    programId,
    keys: [
      w(addr(args.creator), true),
      ro(addr(args.oracle)),
      w(meta.address),
      ro(SYSTEM_PROGRAM_ID),
    ],
    data: withDisc(Ix.WriteOracleMeta, ...parts),
  });
}

// ---------------------------------------------------------------------------
// Propose (Ix=11) — processor/propose.rs
// Accounts: 0 oracle(w) 1 proposer(w,PDA) 2 authority(w,signer) 3 authority_kass(w)
//           4 stake_vault(w,PDA) 5 token program(ro) 6 system program(ro).
// Payload (9): option u8 ++ bond u64.
// ---------------------------------------------------------------------------
export interface ProposeArgs {
  /** The oracle being proposed against. */
  oracle: AddressInput;
  /** Proposer authority (signer): funds rent + bond-transfer authority. */
  authority: AddressInput;
  /** Authority's KASS token account — the bond source. */
  authorityKass: AddressInput;
  /** Categorical option proposed (< oracle.options_count). */
  option: number;
  /** KASS bond escrowed into the stake vault (> 0). */
  bond: bigint | number;
  programId?: Address;
}

export async function propose(args: ProposeArgs): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  const oracle = addr(args.oracle);
  const proposer = await pda.proposer(oracle, addr(args.authority), programId);
  const stakeVault = await pda.stakeVault(oracle, programId);

  const data = withDisc(Ix.Propose, u8(args.option), u64LE(args.bond));

  return new TransactionInstruction({
    programId,
    keys: [
      w(oracle),
      w(proposer.address),
      w(addr(args.authority), true),
      w(addr(args.authorityKass)),
      w(stakeVault.address),
      ro(TOKEN_PROGRAM_ID),
      ro(SYSTEM_PROGRAM_ID),
    ],
    data,
  });
}

// ---------------------------------------------------------------------------
// FinalizeProposals (Ix=12) — processor/finalize_proposals.rs
// Accounts: 0 oracle(w), then the FULL proposer set as a READ-ONLY tail.
// Payload: empty.
// ---------------------------------------------------------------------------
export interface FinalizeProposalsArgs {
  /** The oracle to finalize. */
  oracle: AddressInput;
  /** The FULL proposer-PDA set (exactly `proposer_count`), each read-only. */
  proposers: ReadonlyArray<AddressInput>;
  programId?: Address;
}

export async function finalizeProposals(
  args: FinalizeProposalsArgs,
): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  return new TransactionInstruction({
    programId,
    keys: [w(addr(args.oracle)), ...args.proposers.map((p) => ro(addr(p)))],
    data: withDisc(Ix.FinalizeProposals),
  });
}

// ---------------------------------------------------------------------------
// AdvancePhase (Ix=7) — processor/advance_phase.rs
// Accounts: 0 oracle(w). Payload: empty. (Permissionless: no signer.)
// ---------------------------------------------------------------------------
export interface AdvancePhaseArgs {
  /** The oracle to tick FactProposal -> FactVoting. */
  oracle: AddressInput;
  programId?: Address;
}

export async function advancePhase(args: AdvancePhaseArgs): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  return new TransactionInstruction({
    programId,
    keys: [w(addr(args.oracle))],
    data: withDisc(Ix.AdvancePhase),
  });
}

// ---------------------------------------------------------------------------
// SetGovernance (Ix=13) — processor/set_governance.rs
// Accounts: 0 protocol(w) 1 authority(ro,signer) 2 kass_dao(ro).
// Payload (64): dao_authority[32] ++ kass_dao[32].
//
// Task G1: the handoff VALIDATES the linkage against the threaded `kass_dao`
// account — it must equal the payload `kass_dao`, be owned by the futarchy
// program, and carry the `Dao` Anchor discriminator; and the payload
// `dao_authority` must be the Squads v4 vault PDA derived for that DAO.
// ---------------------------------------------------------------------------
export interface SetGovernanceArgs {
  /** Authority (signer): admin pre-handoff, dao_authority post-handoff. */
  authority: AddressInput;
  /** The Squads v4 multisig vault PDA recorded as `dao_authority` (non-zero). */
  daoAuthority: AddressInput;
  /**
   * The futarchy `Dao` account recorded as `kass_dao` (non-zero). Used BOTH as
   * the payload pubkey and the read-only account the processor validates
   * (owner == futarchy program + `Dao` discriminator).
   */
  kassDao: AddressInput;
  programId?: Address;
}

export async function setGovernance(args: SetGovernanceArgs): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  const protocol = await pda.protocol(programId);

  const data = withDisc(
    Ix.SetGovernance,
    pubkeyBytes(args.daoAuthority),
    pubkeyBytes(args.kassDao),
  );

  return new TransactionInstruction({
    programId,
    keys: [
      w(protocol.address),
      ro(addr(args.authority), true),
      ro(addr(args.kassDao)),
    ],
    data,
  });
}

// ---------------------------------------------------------------------------
// SetConfig (Ix=14) — processor/set_config.rs
// Accounts: 0 protocol(w) 1 dao_authority(ro,signer).
// Payload (200): 25 little-endian 8-byte fields in the FIXED order below.
// ---------------------------------------------------------------------------
/**
 * The 25 governable parameters overwritten wholesale by `set_config`, in the
 * EXACT processor order (`set_config.rs` `u64_at`/`i64_at` indices 0..=24).
 * Fields documented as `i64` are encoded signed; the rest unsigned. All are
 * `bigint` so the full u64 range round-trips.
 */
export interface SetConfigParams {
  emissionNum: bigint;
  emissionDen: bigint;
  totalSupplyCap: bigint;
  /** i64 */
  feeEmaHalflife: bigint;
  feePerEmaUnit: bigint;
  feeEmaIncrement: bigint;
  thresholdNum: bigint;
  thresholdDen: bigint;
  marketThresholdNum: bigint;
  marketThresholdDen: bigint;
  flipSlashNum: bigint;
  flipSlashDen: bigint;
  /** i64 */
  phaseWindow: bigint;
  /** i64 */
  proposalWindow: bigint;
  factVoteSlashNum: bigint;
  factVoteSlashDen: bigint;
  rewardProposerWeight: bigint;
  rewardFactWeight: bigint;
  challengeFailUsdcFeeNum: bigint;
  challengeFailUsdcFeeDen: bigint;
  challengeSuccessKassFeeNum: bigint;
  challengeSuccessKassFeeDen: bigint;
  /** Bootstrapping stake-floor curve: fee-EMA below which the floor is 0. */
  stakeFloorEmaThreshold: bigint;
  /** Bootstrapping stake-floor curve: fee-EMA at which the floor reaches max. */
  stakeFloorEmaCap: bigint;
  /** Bootstrapping stake-floor curve: the max floor (KASS base units); 0 = disabled. */
  stakeFloorMax: bigint;
}

/** Encode {@link SetConfigParams} as the 200-byte `set_config` payload (no disc). */
export function encodeSetConfigParams(p: SetConfigParams): Uint8Array {
  return concatBytes([
    u64LE(p.emissionNum), // 0
    u64LE(p.emissionDen), // 1
    u64LE(p.totalSupplyCap), // 2
    i64LE(p.feeEmaHalflife), // 3 (i64)
    u64LE(p.feePerEmaUnit), // 4
    u64LE(p.feeEmaIncrement), // 5
    u64LE(p.thresholdNum), // 6
    u64LE(p.thresholdDen), // 7
    u64LE(p.marketThresholdNum), // 8
    u64LE(p.marketThresholdDen), // 9
    u64LE(p.flipSlashNum), // 10
    u64LE(p.flipSlashDen), // 11
    i64LE(p.phaseWindow), // 12 (i64)
    i64LE(p.proposalWindow), // 13 (i64)
    u64LE(p.factVoteSlashNum), // 14
    u64LE(p.factVoteSlashDen), // 15
    u64LE(p.rewardProposerWeight), // 16
    u64LE(p.rewardFactWeight), // 17
    u64LE(p.challengeFailUsdcFeeNum), // 18
    u64LE(p.challengeFailUsdcFeeDen), // 19
    u64LE(p.challengeSuccessKassFeeNum), // 20
    u64LE(p.challengeSuccessKassFeeDen), // 21
    u64LE(p.stakeFloorEmaThreshold), // 22
    u64LE(p.stakeFloorEmaCap), // 23
    u64LE(p.stakeFloorMax), // 24
  ]);
}

export interface SetConfigArgs {
  /** DAO authority (signer): must equal `protocol.dao_authority`. */
  authority: AddressInput;
  /** The full governable parameter set (overwritten wholesale). */
  params: SetConfigParams;
  programId?: Address;
}

export async function setConfig(args: SetConfigArgs): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  const protocol = await pda.protocol(programId);

  const data = withDisc(Ix.SetConfig, encodeSetConfigParams(args.params));

  return new TransactionInstruction({
    programId,
    keys: [w(protocol.address), ro(addr(args.authority), true)],
    data,
  });
}

// ---------------------------------------------------------------------------
// ResolveDeadend (Ix=15) — processor/resolve_deadend.rs
// Accounts: 0 protocol(ro) 1 oracle(w) 2 dao_authority(ro,signer).
// Payload (1): option u8.
// ---------------------------------------------------------------------------
export interface ResolveDeadendArgs {
  /** The dead-ended oracle to resolve. */
  oracle: AddressInput;
  /** DAO authority (signer): must equal `protocol.dao_authority`. */
  authority: AddressInput;
  /** The winning categorical option (< oracle.options_count). */
  option: number;
  programId?: Address;
}

export async function resolveDeadend(args: ResolveDeadendArgs): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  const protocol = await pda.protocol(programId);

  return new TransactionInstruction({
    programId,
    keys: [
      ro(protocol.address),
      w(addr(args.oracle)),
      ro(addr(args.authority), true),
    ],
    data: withDisc(Ix.ResolveDeadend, u8(args.option)),
  });
}

// ---------------------------------------------------------------------------
// KassPrice (Ix=16) — processor/kass_price.rs
// Accounts: 0 protocol(ro) 1 kass_dao(ro). Payload: empty. Read-only (return data).
// ---------------------------------------------------------------------------
export interface KassPriceArgs {
  /** The futarchy `Dao` account == `protocol.kass_dao`. */
  kassDao: AddressInput;
  programId?: Address;
}

export async function kassPrice(args: KassPriceArgs): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  const protocol = await pda.protocol(programId);

  return new TransactionInstruction({
    programId,
    keys: [ro(protocol.address), ro(addr(args.kassDao))],
    data: withDisc(Ix.KassPrice),
  });
}

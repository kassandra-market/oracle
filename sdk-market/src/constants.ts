/**
 * Program constants for the kassandra-market AMM prediction-market program — the
 * TypeScript mirror of the Rust source of truth. Every value here is pinned
 * against the program and guarded by `test/parity.test.ts` (a mismatch fails CI
 * = drift guard). Sources:
 *
 *   - `programs/kassandra-market/src/instruction.rs` — {@link Ix} discriminants (0..=10)
 *   - `programs/kassandra-market/src/state.rs`       — {@link AccountType} (0..=3), {@link MarketStatus} (0..=4)
 *   - `programs/kassandra-market/src/error.rs`       — {@link MarketError} (0..=21)
 *   - `programs/kassandra-market/tests/state_layout.rs` — {@link ACCOUNT_SIZES}
 *   - `sdk-rs/src/{metadao,lib}.rs`                  — external program IDs
 */
import { Address } from "@solana/web3.js";

/** kassandra-market program ID (`programs/kassandra-market/src/lib.rs::ID`). */
export const MARKET_PROGRAM_ID = new Address("FEGNHWAB7kc7VC9CCwbvVPsv4Jykz2r2WQ758V4xCT9S");

/** The Solana System program, referenced by account-creating instructions. */
export const SYSTEM_PROGRAM_ID = new Address("11111111111111111111111111111111");

/** The SPL Token program, referenced by escrow/vault token CPIs. */
export const TOKEN_PROGRAM_ID = new Address("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

/** The SPL Associated Token Account program (ATA derivation). */
export const ATA_PROGRAM_ID = new Address("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

/**
 * The BPF Upgradeable Loader — owns every upgradeable program's `ProgramData`
 * account (which stores the program's `upgrade_authority`). `initConfig` requires
 * the caller be that authority, proven via the derived `ProgramData` PDA. Mirror
 * of `sdk-rs/src/pda.rs::BPF_UPGRADEABLE_LOADER_ID`.
 */
export const BPF_UPGRADEABLE_LOADER_ID = new Address("BPFLoaderUpgradeab1e11111111111111111111111");

/**
 * Instruction discriminants — the leading byte of `instruction_data`.
 * Mirror of `Ix` in `instruction.rs`. STABLE PUBLIC CONTRACT: never renumbered;
 * new variants are appended.
 */
export enum Ix {
  InitConfig = 0,
  UpdateConfig = 1,
  CreateMarket = 2,
  Contribute = 3,
  Cancel = 4,
  Refund = 5,
  Activate = 6,
  ClaimLp = 7,
  ResolveMarket = 8,
  CollectFee = 9,
  CloseMarket = 10,
}

/**
 * On-chain account-type discriminator — the FIRST byte of every Pod account
 * (`account_type`, followed by a 7-byte header pad). Mirror of `AccountType` in
 * `state.rs`. `Uninitialized` (0) is what a freshly zeroed account carries.
 */
export enum AccountType {
  Uninitialized = 0,
  Config = 1,
  Market = 2,
  Contribution = 3,
}

/**
 * Market lifecycle, stored on-chain as a `u8` (`Market.status`). Mirror of
 * `MarketStatus` in `state.rs`. Phase 1 uses only `Funding` and `Cancelled`;
 * the rest are used by the Phase-2 MetaDAO composition.
 */
export enum MarketStatus {
  Funding = 0,
  Active = 1,
  Resolved = 2,
  Void = 3,
  Cancelled = 4,
}

/**
 * Program error codes surfaced to clients as `ProgramError::Custom(u32)`.
 * Mirror of `MarketError` in `error.rs` (0..=21). STABLE PUBLIC CONTRACT.
 */
export enum MarketError {
  InvalidAccount = 0,
  Unauthorized = 1,
  AlreadyInitialized = 2,
  InvalidSplit = 3,
  ZeroAmount = 4,
  NotFunding = 5,
  OracleNotTerminal = 6,
  AlreadyFunded = 7,
  AlreadyClaimed = 8,
  NotCancelled = 9,
  OracleResolved = 10,
  NotBinary = 11,
  WrongMint = 12,
  NotFunded = 13,
  PoolNotEmpty = 14,
  NotActive = 15,
  AlreadySettled = 16,
  InvalidFee = 17,
  FeeNotCollected = 18,
  InvalidOutcome = 19,
  ContributionsOpen = 20,
  NotSettled = 21,
}

/** Governance guardrail: max protocol `fee_bps` (10% = 1000 bps). Mirror of `state::MAX_FEE_BPS`. */
export const MAX_FEE_BPS = 1000;

/** Human-readable message per {@link MarketError}. */
export const MARKET_ERROR_MESSAGES: Record<MarketError, string> = {
  [MarketError.InvalidAccount]: "An account passed to the instruction is invalid (wrong owner, address, or contents).",
  [MarketError.Unauthorized]: "The signer is not authorized to perform this action.",
  [MarketError.AlreadyInitialized]: "The target account is already initialized.",
  [MarketError.InvalidSplit]: "Unused (formerly the open_yes_bps range check; the uneven opening prior was abandoned).",
  [MarketError.ZeroAmount]: "A zero amount was supplied where a positive amount is required.",
  [MarketError.NotFunding]: "The market is not in the Funding status this instruction requires.",
  [MarketError.OracleNotTerminal]: "The oracle has not reached a terminal (resolved/void) phase.",
  [MarketError.AlreadyFunded]: "The market has already reached its funding target / been funded.",
  [MarketError.AlreadyClaimed]:
    "Unused (the Contribution is now closed at claim/refund, so its absence — not this flag — is the idempotency guard).",
  [MarketError.NotCancelled]: "The market is not in the Cancelled status this instruction requires.",
  [MarketError.OracleResolved]: "The oracle has already resolved; this action is no longer permitted.",
  [MarketError.NotBinary]: "The oracle is not a binary (two-option) market.",
  [MarketError.WrongMint]: "A supplied mint does not match the expected mint.",
  [MarketError.NotFunded]: "The market has not been funded yet.",
  [MarketError.PoolNotEmpty]: "The AMM pool / vault is not empty.",
  [MarketError.NotActive]: "The market is not in the Active status this instruction requires.",
  [MarketError.AlreadySettled]: "The market has already been settled.",
  [MarketError.InvalidFee]: "The protocol fee (fee_bps) exceeds the governance maximum.",
  [MarketError.FeeNotCollected]:
    "The protocol fee has not been collected yet; run collect_fee before claiming LP.",
  [MarketError.InvalidOutcome]:
    "The requested outcome_index is out of range for the oracle (>= options_count).",
  [MarketError.ContributionsOpen]:
    "The market still has open contributions (some contributor has not yet claimed/refunded); it cannot be closed until every contribution is exited.",
  [MarketError.NotSettled]:
    "The market is not in a terminal (Resolved/Void/Cancelled) status, so it cannot be closed.",
};

/**
 * Decode a `ProgramError::Custom(u32)` code into a {@link MarketError}, or
 * `null` for an unknown code.
 */
export function decodeError(code: number): MarketError | null {
  return code in MarketError && typeof (MarketError as Record<number, unknown>)[code] === "string"
    ? (code as MarketError)
    : null;
}

/**
 * Pinned on-chain ABI sizes (bytes) of the 3 Pod accounts, from
 * `tests/state_layout.rs` (`account_sizes_are_stable`). Each carries an 8-byte
 * header (`account_type: u8` + pad) at offset 0. The decoders read exactly this
 * many bytes; the parity guard asserts these against the program.
 */
export const ACCOUNT_SIZES = {
  Config: 120,
  Market: 400,
  Contribution: 88,
} as const;

/**
 * External program IDs the kassandra-market MetaDAO composition binds to.
 * Verified against `sdk-rs/src/metadao.rs`.
 */
export const EXTERNAL_PROGRAM_IDS = {
  /** MetaDAO conditional-vault. */
  conditionalVault: new Address("VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg"),
  /** MetaDAO AMM v0.4. */
  ammV04: new Address("AMMyu265tkBpRW21iGQxKGLaves3gKm2JcMUqfXNSpqD"),
} as const;

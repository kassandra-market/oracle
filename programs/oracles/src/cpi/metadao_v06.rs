//! MetaDAO **futarchy v0.6** + **Meteora DAMM v2** CPI wire format + account
//! layouts (Task F0 recon).
//!
//! This is the v0.6 governance counterpart of [`super::metadao`] (which pins the
//! dispute core's v0.4 standalone `amm` + `conditional_vault`). v0.6 is a
//! SEPARATE, NEWER stack and this module is purely ADDITIVE — it does not touch
//! the v0.4 wiring.
//!
//! # Resolved program IDs (authoritatively sourced — see `scripts/fetch-metadao-v06.sh`)
//!
//! | program            | id                                            | version / source                |
//! |--------------------|-----------------------------------------------|---------------------------------|
//! | `futarchy`         | `FUTARELBfJfQ8RDGhg1wdhddq1odMAJUePHFuBYfUxKq` | v0.6.0 (replaces `autocrat`)     |
//! | `conditional_vault`| `VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg` | v0.6 line — UNCHANGED from v0.4  |
//! | Meteora DAMM v2    | `cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG` | cp-amm (MeteoraAg/damm-v2 @ main)|
//! | Squads v4          | `SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf` | DAO execution authority host    |
//!
//! Source of truth:
//! * `github.com/metaDAOproject/programs` @ tag **v0.6.0**: `Anchor.toml`
//!   `[programs.localnet]` + the `declare_id!`s in `programs/futarchy/src/lib.rs`
//!   and `programs/conditional_vault/src/lib.rs`, cross-checked against the live
//!   mainnet-beta deployments (slots/sizes in the fetch script header).
//! * `github.com/MeteoraAg/damm-v2` `programs/cp-amm/src/lib.rs` @ main for the
//!   Meteora DAMM v2 (cp-amm) `declare_id!`, cross-confirmed as the mainnet
//!   deployment in MeteoraAg/damm-v2-sdk. MetaDAO's `programs/damm_v2_cpi` shim
//!   (v0.6 tree) `declare_id!`s the same `cpamd…` address.
//! * `github.com/Squads-Protocol/v4` @ rev `6d5235da621a2e9b7379ea358e48760e981053be`
//!   (the exact rev `futarchy/Cargo.toml` depends on) for the multisig/vault PDA
//!   seeds (`state/seeds.rs`) and program id (`declare_id!`).
//!
//! # KEY RECON FINDINGS (these drive F1/F5/F6)
//!
//! 1. **DAO execution authority is a Squads v4 multisig vault, not a futarchy
//!    PDA.** `initialize_dao` CPIs into Squads to create a multisig whose
//!    `create_key` is the `Dao` PDA; a passed proposal carries a `squads_proposal`
//!    and executes through the Squads **vault** PDA. So Kassandra's
//!    `Protocol.dao_authority` (the signer of `set_config`/`resolve_deadend`) is
//!    the [`squads_vault_pda`], derived under [`SQUADS_V4_ID`]. See the PDA
//!    builders below for the exact seeds.
//!
//! 2. **Meteora cp-amm has NO TWAP oracle.** Its `Pool` (zero-copy) stores only
//!    an INSTANTANEOUS `sqrt_price: u128` (Q64.64) plus cumulative *fee*
//!    accumulators — there is no cumulative price observation. The
//!    manipulation-resistant KASS/USDC TWAP the design's `kass_price` (F5) needs
//!    is the futarchy program's **embedded** `FutarchyAmm` spot-pool
//!    `TwapOracle` (`Dao.amm` → see [`futarchy_spot_twap`]), NOT Meteora. The
//!    Meteora `Pool` layout is documented below for completeness (an instantaneous
//!    spot price is still a usable, if manipulable, fallback), but F5 should read
//!    the futarchy TWAP.
//!
//! # Anchor discriminators
//!
//! Each instruction is selected by `sha256("global:<snake_case_name>")[..8]`;
//! each account's first 8 bytes are `sha256("account:<TypeName>")[..8]`. Args
//! follow the discriminator, Borsh-encoded.
//!
//! # `#[event_cpi]`
//!
//! The futarchy instructions are `#[event_cpi]`, appending two trailing accounts
//! (the futarchy `event_authority` PDA `[b"__event_authority"]` + the futarchy
//! program id). See [`super::metadao`] for the same mechanism on the vault.

#![allow(dead_code)]

use crate::error::KassandraError;
use pinocchio::{
    account::AccountView as AccountInfo,
    address::Address as Pubkey,
    cpi::Signer,
    error::ProgramError,
    instruction::{InstructionAccount, InstructionView},
    ProgramResult,
};

// ─────────────────────────────────────────────────────────────────────────────
// Program IDs
// ─────────────────────────────────────────────────────────────────────────────

/// MetaDAO `futarchy` v0.6.0 governance/proposal program (mainnet-beta). Replaces
/// the legacy `autocrat`.
pub const FUTARCHY_ID: Pubkey =
    Pubkey::from_str_const("FUTARELBfJfQ8RDGhg1wdhddq1odMAJUePHFuBYfUxKq");

/// MetaDAO `conditional_vault` (v0.6 line). Byte-for-byte the same deployed
/// program as the v0.4 vault ([`super::metadao::CONDITIONAL_VAULT_ID`]); v0.6
/// reuses it. Its instruction/account discriminators are unchanged.
pub const CONDITIONAL_VAULT_V06_ID: Pubkey =
    Pubkey::from_str_const("VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg");

/// Meteora DAMM v2 (cp-amm) program (mainnet-beta).
pub const METEORA_DAMM_V2_ID: Pubkey =
    Pubkey::from_str_const("cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG");

/// Squads v4 multisig program — hosts the DAO execution-authority vault PDA.
pub const SQUADS_V4_ID: Pubkey =
    Pubkey::from_str_const("SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf");

// ─────────────────────────────────────────────────────────────────────────────
// Anchor instruction discriminators — sha256("global:<name>")[..8]
// ─────────────────────────────────────────────────────────────────────────────
//
// futarchy v0.6 (programs/futarchy/src/lib.rs @ v0.6.0).

/// `futarchy::initialize_dao` — args `InitializeDaoParams` (Borsh; see
/// [`InitializeDaoParams`] doc for the field order).
pub const FUT_INITIALIZE_DAO: [u8; 8] = [0x80, 0xe2, 0x60, 0x5a, 0x27, 0x38, 0x18, 0xc4];
/// `futarchy::initialize_proposal` — no positional args (accounts only).
pub const FUT_INITIALIZE_PROPOSAL: [u8; 8] = [0x32, 0x49, 0x9c, 0x62, 0x81, 0x95, 0x15, 0x9e];
/// `futarchy::launch_proposal`.
pub const FUT_LAUNCH_PROPOSAL: [u8; 8] = [0x10, 0xd3, 0xbd, 0x77, 0xf5, 0x48, 0x00, 0xe5];
/// `futarchy::finalize_proposal` — resolves the conditional question + sets
/// `ProposalState::{Passed,Failed}` from the pass/fail TWAP comparison.
pub const FUT_FINALIZE_PROPOSAL: [u8; 8] = [0x17, 0x44, 0x33, 0xa7, 0x6d, 0xad, 0xbb, 0xa4];
/// `futarchy::update_dao` — args `UpdateDaoParams`.
pub const FUT_UPDATE_DAO: [u8; 8] = [0x83, 0x48, 0x4b, 0x19, 0x70, 0xd2, 0x6d, 0x02];
/// `futarchy::spot_swap` — swaps against the embedded spot AMM (cranks its TWAP).
pub const FUT_SPOT_SWAP: [u8; 8] = [0xa7, 0x61, 0x0c, 0xe7, 0xed, 0x4e, 0xa6, 0xfb];
/// `futarchy::conditional_swap` — swaps against a pass/fail conditional market.
pub const FUT_CONDITIONAL_SWAP: [u8; 8] = [0xc2, 0x88, 0xdc, 0x59, 0xf2, 0xa9, 0x82, 0x9d];

/// `Dao` account discriminator (`sha256("account:Dao")[..8]`).
pub const DAO_ACCOUNT_DISCRIMINATOR: [u8; 8] = [0xa3, 0x09, 0x2f, 0x1f, 0x34, 0x55, 0xc5, 0x31];
/// `Proposal` account discriminator (`sha256("account:Proposal")[..8]`).
pub const PROPOSAL_ACCOUNT_DISCRIMINATOR: [u8; 8] =
    [0x1a, 0x5e, 0xbd, 0xbb, 0x74, 0x88, 0x35, 0x21];

// Meteora DAMM v2 (cp-amm, MeteoraAg/damm-v2 @ main).

/// `cp_amm::initialize_pool`.
pub const METEORA_INITIALIZE_POOL: [u8; 8] = [0x5f, 0xb4, 0x0a, 0xac, 0x54, 0xae, 0xe8, 0x28];
/// `cp_amm::swap`.
pub const METEORA_SWAP: [u8; 8] = [0xf8, 0xc6, 0x9e, 0x91, 0xe1, 0x75, 0x87, 0xc8];
/// `cp_amm::add_liquidity`.
pub const METEORA_ADD_LIQUIDITY: [u8; 8] = [0xb5, 0x9d, 0x59, 0x43, 0x8f, 0xb6, 0x34, 0x48];
/// `Pool` account discriminator (`sha256("account:Pool")[..8]`).
pub const METEORA_POOL_ACCOUNT_DISCRIMINATOR: [u8; 8] =
    [0xf1, 0x9a, 0x6d, 0x04, 0x11, 0xb1, 0x6d, 0xbc];

// Squads v4 (Squads-Protocol/v4 @ rev 6d5235da). Squads v4 is an **Anchor**
// program, so its instruction selectors use the SAME scheme as futarchy/Meteora:
// `sha256("global:<snake_case_name>")[..8]`. (Confirmed against the dumped
// `squads_v4.so`: the [`SQUADS_VAULT_TRANSACTION_EXECUTE`] discriminator below
// dispatches into the program's `VaultTransactionExecute` handler — see the
// F6 dispatch-probe test in `tests/governance_seam.rs`.)
//
// The DAO-execution seam Kassandra cares about is `vault_transaction_execute`:
// a passed futarchy proposal's actions (a `set_config` / `resolve_deadend` CPI
// into Kassandra) are wrapped in a Squads `VaultTransaction` and run by this
// instruction, which `invoke_signed`s each inner instruction with the
// [`squads_vault_pda`] as the signing authority. That vault PDA is exactly what
// Kassandra stores as `Protocol.dao_authority`.

/// `squads_multisig_program::vault_transaction_execute` — runs a created vault
/// transaction, signing inner instructions as the multisig's vault PDA. This is
/// the instruction that produces the `dao_authority` (vault-PDA) signature on
/// Kassandra's `set_config` / `resolve_deadend` in production.
pub const SQUADS_VAULT_TRANSACTION_EXECUTE: [u8; 8] =
    [0xc2, 0x08, 0xa1, 0x57, 0x99, 0xa4, 0x19, 0xab];
/// `squads_multisig_program::vault_transaction_create` — stages the inner
/// instructions (the proposal's actions) into a `VaultTransaction` PDA.
pub const SQUADS_VAULT_TRANSACTION_CREATE: [u8; 8] =
    [0x30, 0xfa, 0x4e, 0xa8, 0xd0, 0xe2, 0xda, 0xd3];
/// `squads_multisig_program::proposal_create`.
pub const SQUADS_PROPOSAL_CREATE: [u8; 8] = [0xdc, 0x3c, 0x49, 0xe0, 0x1e, 0x6c, 0x4f, 0x9f];
/// `squads_multisig_program::multisig_create_v2` — the CPI `initialize_dao`
/// makes to stand up the DAO's multisig (`create_key` == the futarchy `Dao`).
pub const SQUADS_MULTISIG_CREATE_V2: [u8; 8] = [0x32, 0xdd, 0xc7, 0x5d, 0x28, 0xf5, 0x8b, 0xe9];

// ─────────────────────────────────────────────────────────────────────────────
// PDA seeds
// ─────────────────────────────────────────────────────────────────────────────

/// futarchy `Dao` PDA seed prefix.
pub const SEED_DAO: &[u8] = b"dao";
/// futarchy `Proposal` PDA seed prefix.
pub const SEED_PROPOSAL: &[u8] = b"proposal";
/// Anchor `#[event_cpi]` event-authority PDA seed (under the futarchy program).
pub const SEED_EVENT_AUTHORITY: &[u8] = b"__event_authority";

// Squads v4 seeds (Squads-Protocol/v4 @ rev 6d5235da, state/seeds.rs).
/// Squads `SEED_PREFIX`.
pub const SQUADS_SEED_PREFIX: &[u8] = b"multisig";
/// Squads `SEED_MULTISIG`.
pub const SQUADS_SEED_MULTISIG: &[u8] = b"multisig";
/// Squads `SEED_VAULT`.
pub const SQUADS_SEED_VAULT: &[u8] = b"vault";

// ─────────────────────────────────────────────────────────────────────────────
// Account LAYOUTS (verified against metaDAOproject/programs @ v0.6.0 source)
// ─────────────────────────────────────────────────────────────────────────────
//
// ## `Dao` (futarchy, `#[account] #[derive(InitSpace)]`, programs/futarchy/src/
// state/dao.rs). Borsh field order (8-byte Anchor disc first):
//
//   disc[8]
//   amm: FutarchyAmm {
//       state: PoolState   <-- ENUM (variable length!), see below
//       total_liquidity: u128
//       base_mint: Pubkey
//       quote_mint: Pubkey
//       amm_base_vault: Pubkey
//       amm_quote_vault: Pubkey
//   }
//   nonce: u64
//   dao_creator: Pubkey
//   pda_bump: u8
//   squads_multisig: Pubkey
//   squads_multisig_vault: Pubkey   <-- the DAO execution authority (also a PDA)
//   base_mint: Pubkey
//   quote_mint: Pubkey
//   proposal_count: u32
//   pass_threshold_bps: u16
//   seconds_per_proposal: u32
//   twap_initial_observation: u128
//   twap_max_observation_change_per_update: u128
//   twap_start_delay_seconds: u32
//   min_quote_futarchic_liquidity: u64
//   min_base_futarchic_liquidity: u64
//   base_to_stake: u64
//   seq_num: u64
//   initial_spending_limit: Option<InitialSpendingLimit>
//
// CAUTION: `amm.state` is a Borsh enum `PoolState{ Spot{spot:Pool},
// Futarchy{spot,pass,fail:Pool} }`. Its SERIALIZED length depends on the variant
// (Spot = 1 + 1*Pool, Futarchy = 1 + 3*Pool), so EVERY `Dao` field after
// `amm.state` lives at a VARIABLE byte offset. They cannot be read with a fixed
// const; you must Borsh-decode the enum (or skip past it using the live variant
// tag). `Pool` borsh-serializes to 132 bytes (TwapOracle 100 + 4×u64 32). So:
//   - Spot  DAO: fields after state start at 8 + 1 + 132          = 141
//   - Futar DAO: fields after state start at 8 + 1 + 3*132        = 405
// `squads_multisig_vault` is then at +(8 nonce +32 creator +1 bump +32 multisig)
// = +73 from there (Spot: 214, Futarchy: 478). F1 should store the vault key
// directly in Kassandra's `Protocol` at bootstrap rather than re-derive it from
// `Dao` bytes, precisely because of this variable offset.
//
// ## Futarchy spot TWAP (the F5 `kass_price` source). The spot `Pool` is the
// FIRST payload element of BOTH PoolState variants, so its offsets ARE fixed
// regardless of variant:
//
//   byte 8  : PoolState enum tag (0 = Spot, 1 = Futarchy)
//   byte 9  : spot Pool starts == oracle (TwapOracle) starts
//             aggregator                          u128 @  9
//             last_updated_timestamp              i64  @ 25
//             created_at_timestamp                i64  @ 33
//             last_price                          u128 @ 41
//             last_observation                    u128 @ 57
//             max_observation_change_per_update   u128 @ 73
//             initial_observation                 u128 @ 89
//             start_delay_seconds                 u32  @105
//   byte 109: quote_reserves u64, base_reserves u64 @117, … (spot Pool tail)
//
// get_twap() (futarchy source) =
//     aggregator / (last_updated_timestamp - (created_at_timestamp + start_delay_seconds))
// requiring aggregator != 0 and last_updated_timestamp > start. The quotient is a
// price = quote_units_per_base * 1e12 (PRICE_SCALE); UI price further adjusts for
// base/quote decimals. [`futarchy_spot_twap`] mirrors this exactly.

/// PoolState enum-tag byte in a `Dao` account (0 = Spot, 1 = Futarchy).
pub const DAO_POOLSTATE_TAG_OFFSET: usize = 8;
/// Spot `Pool` start (== spot TwapOracle start) inside a `Dao` account.
pub const DAO_SPOT_POOL_OFFSET: usize = 9;
/// `TwapOracle.aggregator: u128` (spot) — byte offset.
pub const DAO_SPOT_AGGREGATOR_OFFSET: usize = 9;
/// `TwapOracle.last_updated_timestamp: i64` (spot) — byte offset.
pub const DAO_SPOT_LAST_UPDATED_TS_OFFSET: usize = 25;
/// `TwapOracle.created_at_timestamp: i64` (spot) — byte offset.
pub const DAO_SPOT_CREATED_AT_TS_OFFSET: usize = 33;
/// `TwapOracle.last_price: u128` (spot) — byte offset.
pub const DAO_SPOT_LAST_PRICE_OFFSET: usize = 41;
/// `TwapOracle.start_delay_seconds: u32` (spot) — byte offset.
pub const DAO_SPOT_START_DELAY_SECONDS_OFFSET: usize = 105;
/// Serialized size of one futarchy `Pool` (TwapOracle 100 + 4×u64 32).
pub const FUTARCHY_POOL_LEN: usize = 132;
/// Smallest `Dao` data length covering the spot TWAP fields.
pub const DAO_SPOT_TWAP_MIN_LEN: usize = DAO_SPOT_POOL_OFFSET + FUTARCHY_POOL_LEN;

// ## `Proposal` (futarchy, programs/futarchy/src/state/proposal.rs). Borsh order:
//
//   disc[8]
//   number: u32                 @  8
//   proposer: Pubkey            @ 12
//   timestamp_enqueued: i64     @ 44
//   state: ProposalState        @ 52   <-- ENUM (variable): Draft{amount_staked:u64}
//                                          | Pending | Passed | Failed. Tag byte +
//                                          (8 bytes ONLY for Draft). All fields
//                                          AFTER `state` are at a variable offset.
//   base_vault: Pubkey
//   quote_vault: Pubkey
//   dao: Pubkey
//   pda_bump: u8
//   question: Pubkey
//   duration_in_seconds: u32
//   squads_proposal: Pubkey
//   pass_base_mint / pass_quote_mint / fail_base_mint / fail_quote_mint: Pubkey
//
// The leading fixed region (number/proposer/timestamp_enqueued + the state TAG)
// is reliable; `state` tag byte @52 (0=Draft,1=Pending,2=Passed,3=Failed) tells
// you the verdict. Fields after `state` need Borsh decoding (Draft adds 8 bytes).

/// `Proposal.number: u32` — byte offset.
pub const PROPOSAL_NUMBER_OFFSET: usize = 8;
/// `Proposal.proposer: Pubkey` — byte offset.
pub const PROPOSAL_PROPOSER_OFFSET: usize = 12;
/// `Proposal.timestamp_enqueued: i64` — byte offset.
pub const PROPOSAL_TS_ENQUEUED_OFFSET: usize = 44;
/// `Proposal.state` enum tag — byte offset (0=Draft,1=Pending,2=Passed,3=Failed).
pub const PROPOSAL_STATE_TAG_OFFSET: usize = 52;

// ## Meteora DAMM v2 `Pool` (cp-amm, `#[account(zero_copy)] #[repr(C)]`,
// MeteoraAg/damm-v2 programs/cp-amm/src/state/pool.rs). Field ORDER (8-byte disc
// first; zero-copy means C layout with explicit padding fields, NOT borsh):
//
//   disc[8]
//   pool_fees: PoolFeesStruct   (base_fee, protocol/referral fee %, dynamic_fee,
//                                init_sqrt_price; nested zero-copy structs)
//   token_a_mint: Pubkey
//   token_b_mint: Pubkey
//   token_a_vault: Pubkey
//   token_b_vault: Pubkey
//   whitelisted_vault: Pubkey
//   padding_0: [u8; 32]
//   liquidity: u128
//   padding_1: u128
//   protocol_a_fee: u64
//   protocol_b_fee: u64
//   padding_2: u128
//   sqrt_min_price: u128
//   sqrt_max_price: u128
//   sqrt_price: u128            <-- the load-bearing INSTANTANEOUS price (Q64.64)
//   activation_point: u64
//   activation_type: u8         (0 = by slot, 1 = by timestamp)
//   pool_status / token_a_flag / token_b_flag / collect_fee_mode / pool_type /
//   fee_version / padding_3: u8 …
//   fee_a_per_liquidity / fee_b_per_liquidity: [u8;32]  (cumulative FEE, U256)
//   permanent_lock_liquidity: u128
//   metrics: PoolMetrics
//   creator: Pubkey
//   token_a_amount / token_b_amount: u64
//   layout_version: u8 + padding …
//   reward_infos: [RewardInfo; NUM_REWARDS]
//
// IMPORTANT (F5): there is NO TWAP / cumulative-price observation in cp-amm —
// `sqrt_price` is the spot price at last touch, and `fee_*_per_liquidity` are FEE
// accumulators, not price. Use [`futarchy_spot_twap`] for the manipulation-
// resistant TWAP. The exact byte offset of `sqrt_price` is NOT hand-pinned here:
// computing it requires the full C-layout/padding of the nested zero-copy
// `PoolFeesStruct`/`BaseFeeStruct`/`DynamicFeeStruct`, which is error-prone by
// hand. F5 (if it ends up reading a Meteora pool at all) MUST pin `sqrt_price`'s
// offset against a LIVE pool account dump and/or the published cp-amm IDL before
// relying on it. Field ORDER above is from source and is authoritative; the
// numeric offset is the deferred unknown.

// ─────────────────────────────────────────────────────────────────────────────
// Little-endian field readers (out-of-bounds -> InvalidAccount)
// ─────────────────────────────────────────────────────────────────────────────

/// Read a 32-byte pubkey out of `data` at byte `off`.
pub fn read_pubkey(data: &[u8], off: usize) -> Result<Pubkey, ProgramError> {
    data.get(off..off + 32)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| KassandraError::InvalidAccount.into())
}

/// Read a little-endian `u32` out of `data` at byte `off`.
pub fn read_u32(data: &[u8], off: usize) -> Result<u32, ProgramError> {
    data.get(off..off + 4)
        .and_then(|s| s.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| KassandraError::InvalidAccount.into())
}

/// Read a little-endian `i64` out of `data` at byte `off`.
pub fn read_i64(data: &[u8], off: usize) -> Result<i64, ProgramError> {
    data.get(off..off + 8)
        .and_then(|s| s.try_into().ok())
        .map(i64::from_le_bytes)
        .ok_or_else(|| KassandraError::InvalidAccount.into())
}

/// Read a little-endian `u128` out of `data` at byte `off`.
pub fn read_u128(data: &[u8], off: usize) -> Result<u128, ProgramError> {
    data.get(off..off + 16)
        .and_then(|s| s.try_into().ok())
        .map(u128::from_le_bytes)
        .ok_or_else(|| KassandraError::InvalidAccount.into())
}

/// Compute the futarchy spot-market TWAP from a raw `Dao` account's bytes,
/// mirroring `Pool::get_twap()` in the v0.6 futarchy source:
///
/// ```text
/// twap = aggregator / (last_updated_timestamp - (created_at_timestamp + start_delay_seconds))
/// ```
///
/// The result is a price scaled by `1e12` (quote units per base unit). This is
/// the F5 `kass_price` primitive: it reads the spot `Pool.oracle` embedded in the
/// `Dao` (fixed offsets, variant-independent — see the layout block above).
/// Returns [`KassandraError::InvalidAccount`] if the buffer is too short, or if
/// the elapsed window is non-positive or the aggregator is zero (i.e. the TWAP
/// has not started / is not yet observable).
pub fn futarchy_spot_twap(dao_data: &[u8]) -> Result<u128, ProgramError> {
    let aggregator = read_u128(dao_data, DAO_SPOT_AGGREGATOR_OFFSET)?;
    let last_updated = read_i64(dao_data, DAO_SPOT_LAST_UPDATED_TS_OFFSET)?;
    let created_at = read_i64(dao_data, DAO_SPOT_CREATED_AT_TS_OFFSET)?;
    let start_delay = read_u32(dao_data, DAO_SPOT_START_DELAY_SECONDS_OFFSET)? as i64;

    let start = created_at
        .checked_add(start_delay)
        .ok_or(KassandraError::InvalidAccount)?;
    let seconds_passed = last_updated
        .checked_sub(start)
        .filter(|&d| d > 0)
        .ok_or(KassandraError::InvalidAccount)?;
    if aggregator == 0 {
        return Err(KassandraError::InvalidAccount.into());
    }
    Ok(aggregator / seconds_passed as u128)
}

// ─────────────────────────────────────────────────────────────────────────────
// Seed-slice assembly (host-runnable; `find_program_address` is SBF-only)
// ─────────────────────────────────────────────────────────────────────────────

/// futarchy `Dao` PDA seeds: `[b"dao", dao_creator, nonce_le[8]]`.
pub fn dao_seeds<'a>(dao_creator: &'a Pubkey, nonce_le: &'a [u8; 8]) -> [&'a [u8]; 3] {
    [SEED_DAO, dao_creator.as_ref(), nonce_le]
}

/// futarchy `Proposal` PDA seeds: `[b"proposal", squads_proposal]`.
pub fn proposal_seeds(squads_proposal: &Pubkey) -> [&[u8]; 2] {
    [SEED_PROPOSAL, squads_proposal.as_ref()]
}

/// Squads multisig PDA seeds: `[b"multisig", b"multisig", create_key]` where
/// `create_key` == the futarchy `Dao` PDA.
pub fn squads_multisig_seeds(dao: &Pubkey) -> [&[u8]; 3] {
    [SQUADS_SEED_PREFIX, SQUADS_SEED_MULTISIG, dao.as_ref()]
}

/// Squads vault (DAO execution authority) PDA seeds:
/// `[b"multisig", multisig, b"vault", vault_index_le[1]]`. The futarchy DAO uses
/// vault index 0.
pub fn squads_vault_seeds<'a>(multisig: &'a Pubkey, vault_index: &'a [u8; 1]) -> [&'a [u8]; 4] {
    [
        SQUADS_SEED_PREFIX,
        multisig.as_ref(),
        SQUADS_SEED_VAULT,
        vault_index,
    ]
}

/// futarchy `#[event_cpi]` event-authority PDA seeds: `[b"__event_authority"]`.
pub fn event_authority_seeds() -> [&'static [u8]; 1] {
    [SEED_EVENT_AUTHORITY]
}

// ─────────────────────────────────────────────────────────────────────────────
// PDA derivation (SBF-only — wrap the seed builders above)
// ─────────────────────────────────────────────────────────────────────────────

/// futarchy `Dao` PDA.
pub fn dao_pda(dao_creator: &Pubkey, nonce: u64) -> (Pubkey, u8) {
    Pubkey::find_program_address(&dao_seeds(dao_creator, &nonce.to_le_bytes()), &FUTARCHY_ID)
}

/// futarchy `Proposal` PDA.
pub fn proposal_pda(squads_proposal: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&proposal_seeds(squads_proposal), &FUTARCHY_ID)
}

/// Squads multisig PDA for a DAO (create_key == `dao`).
pub fn squads_multisig_pda(dao: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&squads_multisig_seeds(dao), &SQUADS_V4_ID)
}

/// Squads **vault** PDA (DAO execution authority) for `multisig` at `vault_index`.
/// This is the key Kassandra stores as `Protocol.dao_authority` and requires as
/// signer on `set_config` / `resolve_deadend`.
pub fn squads_vault_pda(multisig: &Pubkey, vault_index: u8) -> (Pubkey, u8) {
    Pubkey::find_program_address(&squads_vault_seeds(multisig, &[vault_index]), &SQUADS_V4_ID)
}

/// futarchy `#[event_cpi]` event-authority PDA.
pub fn event_authority_pda() -> (Pubkey, u8) {
    Pubkey::find_program_address(&event_authority_seeds(), &FUTARCHY_ID)
}

// ─────────────────────────────────────────────────────────────────────────────
// Args encoders (discriminator ++ Borsh body), no_std / no-alloc
// ─────────────────────────────────────────────────────────────────────────────
//
// STUBBED (documented, not yet wire-validated): `initialize_dao` takes
// `InitializeDaoParams { twap_initial_observation:u128,
// twap_max_observation_change_per_update:u128, twap_start_delay_seconds:u32,
// min_quote_futarchic_liquidity:u64, min_base_futarchic_liquidity:u64,
// base_to_stake:u64, pass_threshold_bps:u16, seconds_per_proposal:u32, nonce:u64,
// initial_spending_limit: Option<InitialSpendingLimit> }` (Borsh). The trailing
// `Option<Vec<Pubkey>>` makes it variable-length; F6 builds it. The fixed-size
// prefix is encoded below for the common `None` spending-limit case.

/// `initialize_proposal` instruction data (no positional args).
pub fn initialize_proposal_data() -> [u8; 8] {
    FUT_INITIALIZE_PROPOSAL
}

/// `finalize_proposal` instruction data (no positional args).
pub fn finalize_proposal_data() -> [u8; 8] {
    FUT_FINALIZE_PROPOSAL
}

/// `initialize_dao` instruction data for the `initial_spending_limit == None`
/// case. Layout: `disc[8] ++ twap_initial_observation:u128 ++
/// twap_max_observation_change_per_update:u128 ++ twap_start_delay_seconds:u32 ++
/// min_quote_futarchic_liquidity:u64 ++ min_base_futarchic_liquidity:u64 ++
/// base_to_stake:u64 ++ pass_threshold_bps:u16 ++ seconds_per_proposal:u32 ++
/// nonce:u64 ++ 0u8 (Option::None tag)` = 8+16+16+4+8+8+8+2+4+8+1 = 83 bytes.
#[allow(clippy::too_many_arguments)]
pub fn initialize_dao_data_no_limit(
    twap_initial_observation: u128,
    twap_max_observation_change_per_update: u128,
    twap_start_delay_seconds: u32,
    min_quote_futarchic_liquidity: u64,
    min_base_futarchic_liquidity: u64,
    base_to_stake: u64,
    pass_threshold_bps: u16,
    seconds_per_proposal: u32,
    nonce: u64,
) -> [u8; 83] {
    let mut out = [0u8; 83];
    let mut o = 0usize;
    let put = |bytes: &[u8], out: &mut [u8; 83], o: &mut usize| {
        out[*o..*o + bytes.len()].copy_from_slice(bytes);
        *o += bytes.len();
    };
    put(&FUT_INITIALIZE_DAO, &mut out, &mut o);
    put(&twap_initial_observation.to_le_bytes(), &mut out, &mut o);
    put(
        &twap_max_observation_change_per_update.to_le_bytes(),
        &mut out,
        &mut o,
    );
    put(&twap_start_delay_seconds.to_le_bytes(), &mut out, &mut o);
    put(
        &min_quote_futarchic_liquidity.to_le_bytes(),
        &mut out,
        &mut o,
    );
    put(
        &min_base_futarchic_liquidity.to_le_bytes(),
        &mut out,
        &mut o,
    );
    put(&base_to_stake.to_le_bytes(), &mut out, &mut o);
    put(&pass_threshold_bps.to_le_bytes(), &mut out, &mut o);
    put(&seconds_per_proposal.to_le_bytes(), &mut out, &mut o);
    put(&nonce.to_le_bytes(), &mut out, &mut o);
    // initial_spending_limit: Option::None
    put(&[0u8], &mut out, &mut o);
    debug_assert_eq!(o, 83);
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Thin invoke wrappers
// ─────────────────────────────────────────────────────────────────────────────

/// Invoke an instruction on the `futarchy` v0.6 program with PDA signers.
pub fn invoke_futarchy_signed<A: AsRef<AccountInfo>>(
    data: &[u8],
    metas: &[InstructionAccount],
    infos: &[A],
    signers: &[Signer],
) -> ProgramResult {
    let ix = InstructionView {
        program_id: &FUTARCHY_ID,
        data,
        accounts: metas,
    };
    pinocchio::cpi::invoke_signed_with_slice(&ix, infos, signers)
}

/// Invoke an instruction on the Meteora DAMM v2 (cp-amm) program with PDA signers.
pub fn invoke_meteora_signed<A: AsRef<AccountInfo>>(
    data: &[u8],
    metas: &[InstructionAccount],
    infos: &[A],
    signers: &[Signer],
) -> ProgramResult {
    let ix = InstructionView {
        program_id: &METEORA_DAMM_V2_ID,
        data,
        accounts: metas,
    };
    pinocchio::cpi::invoke_signed_with_slice(&ix, infos, signers)
}

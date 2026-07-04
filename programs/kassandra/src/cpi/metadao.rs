//! MetaDAO `conditional_vault` + `amm` CPI wire format.
//!
//! # Resolved program IDs (authoritatively sourced — see `scripts/fetch-metadao.sh`)
//!
//! | program            | id                                            | version              |
//! |--------------------|-----------------------------------------------|----------------------|
//! | `conditional_vault`| `VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg` | v0.4.0               |
//! | `amm`              | `AMMyu265tkBpRW21iGQxKGLaves3gKm2JcMUqfXNSpqD` | v0.4.2 (delayed-twap)|
//!
//! Source of truth: `github.com/metaDAOproject/programs` — the `declare_id!`s in
//! `programs/conditional_vault/src/lib.rs` and `programs/amm/src/lib.rs`,
//! cross-checked against `Anchor.toml` and the live mainnet-beta deployments. The
//! DEPLOYED `amm` binary (`AMMyu…`, dumped at mainnet slot 326427490) is the
//! **delayed-twap v0.4.1/v0.4.2** build (tags `delayed-twap-v0.4.1`,
//! `proposal-duration-v0.4.2`), NOT the base `v0.4` tag — it added
//! `TwapOracle.start_delay_slots` + `CreateAmmArgs.twap_start_delay_slots` (see
//! the `Amm` layout block below). MetaDAO governance v0.5+ moved AMM liquidity to
//! Meteora DAMM v2 (`programs/damm_v2_cpi`), so `AMMyu…` is the last first-party
//! MetaDAO AMM and the one whose built-in TWAP oracle matches our design.
//!
//! # Anchor discriminators
//!
//! Each instruction is selected by `sha256("global:<snake_case_name>")[..8]`.
//! Anchor args follow the discriminator, Borsh-encoded. For the structs we use
//! here the Borsh encoding is just the fields concatenated in declaration order
//! (fixed-size arrays / scalars, no length prefixes), so we hand-roll it to
//! avoid pulling `borsh` into the on-chain program.
//!
//! # `#[event_cpi]`
//!
//! Every `conditional_vault` (and `amm`) instruction is annotated
//! `#[event_cpi]`, which appends **two trailing accounts** to the declared
//! account list: the `event_authority` PDA (seeds `[b"__event_authority"]`,
//! derived under the *target* program) and the target program itself. Anchor's
//! remaining-account loops (e.g. the conditional-token mints) run *after* those
//! two accounts. Account orderings below already include them.

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

/// MetaDAO `conditional_vault` v0.4.0 (mainnet-beta).
pub const CONDITIONAL_VAULT_ID: Pubkey =
    Pubkey::from_str_const("VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg");

/// MetaDAO `amm` v0.4 (mainnet-beta).
pub const AMM_ID: Pubkey = Pubkey::from_str_const("AMMyu265tkBpRW21iGQxKGLaves3gKm2JcMUqfXNSpqD");

// ─────────────────────────────────────────────────────────────────────────────
// Anchor instruction discriminators — sha256("global:<name>")[..8]
// ─────────────────────────────────────────────────────────────────────────────

/// `conditional_vault::initialize_question`
pub const INITIALIZE_QUESTION: [u8; 8] = [0xf5, 0x97, 0x6a, 0xbc, 0x58, 0x2c, 0x41, 0xd4];
/// `conditional_vault::resolve_question`
pub const RESOLVE_QUESTION: [u8; 8] = [0x34, 0x20, 0xe0, 0xb3, 0xb4, 0x08, 0x00, 0xf6];
/// `conditional_vault::initialize_conditional_vault`
pub const INITIALIZE_CONDITIONAL_VAULT: [u8; 8] = [0x25, 0x58, 0xfa, 0xd4, 0x36, 0xda, 0xe3, 0xaf];
/// `conditional_vault::split_tokens`
pub const SPLIT_TOKENS: [u8; 8] = [0x4f, 0xc3, 0x74, 0x00, 0x8c, 0xb0, 0x49, 0xb3];
/// `conditional_vault::merge_tokens`
pub const MERGE_TOKENS: [u8; 8] = [0xe2, 0x59, 0xfb, 0x79, 0xe1, 0x82, 0xb4, 0x0e];
/// `conditional_vault::redeem_tokens`
pub const REDEEM_TOKENS: [u8; 8] = [0xf6, 0x62, 0x86, 0x29, 0x98, 0x21, 0x78, 0x45];

/// `amm::create_amm` — args (delayed-twap v0.4.1+) = `CreateAmmArgs {
/// twap_initial_observation: u128, twap_max_observation_change_per_update: u128,
/// twap_start_delay_slots: u64 }` (Borsh, 40 bytes). The base v0.4 build had only
/// the two u128s; the DEPLOYED mainnet binary requires the trailing u64.
pub const CREATE_AMM: [u8; 8] = [0xf2, 0x5b, 0x15, 0xaa, 0x05, 0x44, 0x7d, 0x40];
/// `amm::add_liquidity` — args = `AddLiquidityArgs { quote_amount: u64,
/// max_base_amount: u64, min_lp_tokens: u64 }`.
pub const ADD_LIQUIDITY: [u8; 8] = [0xb5, 0x9d, 0x59, 0x43, 0x8f, 0xb6, 0x34, 0x48];
/// `amm::remove_liquidity`.
pub const REMOVE_LIQUIDITY: [u8; 8] = [0x50, 0x55, 0xd1, 0x48, 0x18, 0xce, 0xb1, 0x6c];
/// `amm::swap` — args = `SwapArgs { swap_type: u8 (0=Buy,1=Sell), input_amount:
/// u64, output_amount_min: u64 }`.
pub const SWAP: [u8; 8] = [0xf8, 0xc6, 0x9e, 0x91, 0xe1, 0x75, 0x87, 0xc8];
/// `amm::crank_that_twap` — folds the current price into the TWAP observation
/// (only once per `ONE_MINUTE_IN_SLOTS == 150` slots). No args; accounts =
/// `[amm(w), event_authority, amm_program]`.
pub const CRANK_THAT_TWAP: [u8; 8] = [0xdc, 0x64, 0x19, 0xf9, 0x00, 0x5c, 0xc3, 0xc1];

// ─────────────────────────────────────────────────────────────────────────────
// PDA seeds (from the conditional_vault source)
// ─────────────────────────────────────────────────────────────────────────────

/// `Question` PDA seed prefix.
pub const SEED_QUESTION: &[u8] = b"question";
/// `ConditionalVault` PDA seed prefix.
pub const SEED_CONDITIONAL_VAULT: &[u8] = b"conditional_vault";
/// Conditional-token mint PDA seed prefix.
pub const SEED_CONDITIONAL_TOKEN: &[u8] = b"conditional_token";
/// Anchor `#[event_cpi]` event-authority PDA seed.
pub const SEED_EVENT_AUTHORITY: &[u8] = b"__event_authority";

// ─────────────────────────────────────────────────────────────────────────────
// Account layout byte offsets (single source of truth — verified against the
// deployed v0.4.0 source `metaDAOproject/programs`, declare_id! == VLTX1…)
// ─────────────────────────────────────────────────────────────────────────────
//
// Both `Question` and `ConditionalVault` carry the 8-byte Anchor account
// discriminator first, so every field offset below is `8 + <borsh offset>`.
// Task 10 (`open_challenge`) and Task 11 (`settle_challenge`) read these.

/// `Question.oracle: Pubkey` — byte offset (after the 8-byte Anchor disc).
pub const QUESTION_ORACLE_OFFSET: usize = 40;
/// `Question.payout_numerators: Vec<u32>` length-prefix offset. At
/// `initialize_question` the Vec is `vec![0; num_outcomes]`, so this u32 LE
/// length equals `num_outcomes`.
pub const QUESTION_NUM_OUTCOMES_LEN_OFFSET: usize = 72;
/// `ConditionalVault.question: Pubkey` — byte offset.
pub const VAULT_QUESTION_OFFSET: usize = 8;
/// `ConditionalVault.underlying_token_mint: Pubkey` — byte offset.
pub const VAULT_UNDERLYING_MINT_OFFSET: usize = 40;
/// `ConditionalVault.underlying_token_account: Pubkey` — byte offset.
pub const VAULT_UNDERLYING_ACCOUNT_OFFSET: usize = 72;

// ─────────────────────────────────────────────────────────────────────────────
// `amm` v0.4.x `Amm` account layout (verified against the source
// `metaDAOproject/programs`, `programs/amm/src/state/amm.rs`, `declare_id! ==
// AMMyu…`). The DEPLOYED mainnet binary is the "delayed-twap" v0.4.1/v0.4.2
// build (tags `delayed-twap-v0.4.1` / `proposal-duration-v0.4.2`), which added a
// `TwapOracle.start_delay_slots: u64` field AFTER `initial_observation` (and a
// `CreateAmmArgs.twap_start_delay_slots`). That new field sits *after* every
// field settle_challenge reads, so the offsets below are identical to the base
// v0.4 layout; only `seq_num` shifted (227 → unread).
// ─────────────────────────────────────────────────────────────────────────────
//
// The `Amm` account is an Anchor `#[account]` (8-byte disc first) Borsh-encoded
// (sequential, little-endian, NO alignment padding). Field order:
//
//   disc[8] | bump:u8 @8 | created_at_slot:u64 @9 | lp_mint:Pubkey @17
//   | base_mint:Pubkey @49 | quote_mint:Pubkey @81 | base_mint_decimals:u8 @113
//   | quote_mint_decimals:u8 @114 | base_amount:u64 @115 | quote_amount:u64 @123
//   | oracle: TwapOracle @131 { last_updated_slot:u64 @131, last_price:u128 @139,
//       last_observation:u128 @155, aggregator:u128 @171,
//       max_observation_change_per_update:u128 @187, initial_observation:u128 @203,
//       start_delay_slots:u64 @219 }   | seq_num:u64 @227
//
// `get_twap()` in the v0.4.2 source computes
//   `aggregator / (last_updated_slot - (created_at_slot + start_delay_slots))`
// — a slot-weighted average of the quote/base price (scaled by PRICE_SCALE =
// 1e12). settle_challenge reads exactly those four fields and mirrors that math.

/// Anchor account discriminator for `Amm` (`sha256("account:Amm")[..8]`). The
/// first 8 bytes of every `Amm` account; checked in settle as defense-in-depth
/// on top of the conditional mint-pair binding.
pub const AMM_ACCOUNT_DISCRIMINATOR: [u8; 8] = [0x8f, 0xf5, 0xc8, 0x11, 0x4a, 0xd6, 0xc4, 0x87];

/// `Amm.created_at_slot: u64` — byte offset.
pub const AMM_CREATED_AT_SLOT_OFFSET: usize = 9;
/// `Amm.base_mint: Pubkey` — byte offset.
pub const AMM_BASE_MINT_OFFSET: usize = 49;
/// `Amm.quote_mint: Pubkey` — byte offset.
pub const AMM_QUOTE_MINT_OFFSET: usize = 81;
/// `Amm.oracle.last_updated_slot: u64` — byte offset.
pub const AMM_LAST_UPDATED_SLOT_OFFSET: usize = 131;
/// `Amm.oracle.aggregator: u128` — byte offset.
pub const AMM_AGGREGATOR_OFFSET: usize = 171;
/// `Amm.oracle.start_delay_slots: u64` — byte offset (v0.4.1+ delayed-twap).
pub const AMM_START_DELAY_SLOTS_OFFSET: usize = 219;
/// Smallest `Amm` account data length that covers every field settle reads
/// (`start_delay_slots` end). The real account is larger (`8 +
/// size_of::<Amm>()`).
pub const AMM_MIN_LEN: usize = AMM_START_DELAY_SLOTS_OFFSET + 8;

// ─────────────────────────────────────────────────────────────────────────────
// Little-endian field readers (single source of truth, co-located with the
// offset consts). Shared by `open_challenge` and `settle_challenge` so the two
// processors decode MetaDAO account fields the same way; out-of-bounds reads map
// to `KassandraError::InvalidAccount`.
// ─────────────────────────────────────────────────────────────────────────────

/// Verify `amm` is a bound MetaDAO v0.4 `Amm` account for a specific conditional
/// pair: owned by the AMM program, long enough, carrying the `Amm` Anchor
/// discriminator, and whose recorded base/quote mints are EXACTLY
/// `expected_base`/`expected_quote` (this market's conditional (KASS, USDC) mint
/// pair for one outcome).
///
/// Shared by BOTH `open_challenge` and `settle_challenge`. Binding at open is
/// load-bearing: a `Market` recorded with an AMM that can't bind here could
/// never settle (`settle_challenge` pins to the RECORDED address), so
/// `open_challenge_count` would never return to 0, `finalize_oracle` would be
/// blocked forever, and every stake in the oracle would be permanently locked.
pub fn assert_amm_bound(
    amm: &AccountInfo,
    expected_base: &Pubkey,
    expected_quote: &Pubkey,
) -> ProgramResult {
    if !amm.owned_by(&AMM_ID) {
        return Err(KassandraError::InvalidAccount.into());
    }
    let data = amm.try_borrow()?;
    if data.len() < AMM_MIN_LEN || data[..8] != AMM_ACCOUNT_DISCRIMINATOR {
        return Err(KassandraError::InvalidAccount.into());
    }
    let base_mint = read_pubkey(&data, AMM_BASE_MINT_OFFSET)?;
    let quote_mint = read_pubkey(&data, AMM_QUOTE_MINT_OFFSET)?;
    if &base_mint != expected_base || &quote_mint != expected_quote {
        return Err(KassandraError::InvalidAccount.into());
    }
    Ok(())
}

/// Read a 32-byte pubkey out of `data` at byte `off`, or `InvalidAccount`.
pub fn read_pubkey(data: &[u8], off: usize) -> Result<Pubkey, ProgramError> {
    data.get(off..off + 32)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| KassandraError::InvalidAccount.into())
}

/// Read a little-endian `u32` out of `data` at byte `off`, or `InvalidAccount`.
pub fn read_u32(data: &[u8], off: usize) -> Result<u32, ProgramError> {
    data.get(off..off + 4)
        .and_then(|s| s.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| KassandraError::InvalidAccount.into())
}

/// Read a little-endian `u64` out of `data` at byte `off`, or `InvalidAccount`.
pub fn read_u64(data: &[u8], off: usize) -> Result<u64, ProgramError> {
    data.get(off..off + 8)
        .and_then(|s| s.try_into().ok())
        .map(u64::from_le_bytes)
        .ok_or_else(|| KassandraError::InvalidAccount.into())
}

/// Read a little-endian `u128` out of `data` at byte `off`, or `InvalidAccount`.
pub fn read_u128(data: &[u8], off: usize) -> Result<u128, ProgramError> {
    data.get(off..off + 16)
        .and_then(|s| s.try_into().ok())
        .map(u128::from_le_bytes)
        .ok_or_else(|| KassandraError::InvalidAccount.into())
}

// ─────────────────────────────────────────────────────────────────────────────
// Seed-slice assembly (host-runnable)
// ─────────────────────────────────────────────────────────────────────────────
//
// `find_program_address` is an SBF-only syscall (it panics off-target), so the
// `*_pda` wrappers below cannot run host-side. The seed ORDER is the part most
// likely to drift, so it is factored into these tiny, host-runnable builders
// that the wrappers reuse. Tests feed the same builders into the host's
// `solana_sdk` PDA derivation (and then into the REAL program), proving the
// seed order matches the deployed binary without needing the syscall. The
// single-byte seeds (`num_outcomes`, mint `index`) are passed as `&[u8; 1]` so
// the caller owns their storage and the returned slices borrow it.

/// `Question` PDA seeds: `[b"question", question_id, oracle, [num_outcomes]]`.
pub fn question_seeds<'a>(
    question_id: &'a [u8; 32],
    oracle: &'a Pubkey,
    num_outcomes: &'a [u8; 1],
) -> [&'a [u8]; 4] {
    [SEED_QUESTION, question_id, oracle.as_ref(), num_outcomes]
}

/// `ConditionalVault` PDA seeds: `[b"conditional_vault", question, underlying_mint]`.
pub fn vault_seeds<'a>(question: &'a Pubkey, underlying_mint: &'a Pubkey) -> [&'a [u8]; 3] {
    [
        SEED_CONDITIONAL_VAULT,
        question.as_ref(),
        underlying_mint.as_ref(),
    ]
}

/// Conditional-token mint PDA seeds: `[b"conditional_token", vault, [index]]`.
pub fn conditional_token_mint_seeds<'a>(vault: &'a Pubkey, index: &'a [u8; 1]) -> [&'a [u8]; 3] {
    [SEED_CONDITIONAL_TOKEN, vault.as_ref(), index]
}

/// `#[event_cpi]` event-authority PDA seeds: `[b"__event_authority"]`.
pub fn event_authority_seeds() -> [&'static [u8]; 1] {
    [SEED_EVENT_AUTHORITY]
}

// ─────────────────────────────────────────────────────────────────────────────
// PDA derivation (SBF-only — wrap the seed builders above)
// ─────────────────────────────────────────────────────────────────────────────

/// `Question` PDA: seeds `[b"question", question_id, oracle, [num_outcomes]]`.
pub fn question_pda(question_id: &[u8; 32], oracle: &Pubkey, num_outcomes: u8) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &question_seeds(question_id, oracle, &[num_outcomes]),
        &CONDITIONAL_VAULT_ID,
    )
}

/// `ConditionalVault` PDA: seeds `[b"conditional_vault", question, underlying_mint]`.
pub fn vault_pda(question: &Pubkey, underlying_mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &vault_seeds(question, underlying_mint),
        &CONDITIONAL_VAULT_ID,
    )
}

/// Conditional-token mint PDA for outcome `index`:
/// seeds `[b"conditional_token", vault, [index]]`.
pub fn conditional_token_mint_pda(vault: &Pubkey, index: u8) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &conditional_token_mint_seeds(vault, &[index]),
        &CONDITIONAL_VAULT_ID,
    )
}

/// `#[event_cpi]` event-authority PDA for `program_id`.
///
/// Parameterized by program id because each `#[event_cpi]` program (the
/// conditional_vault AND the amm) derives its own event authority under its own
/// program id. Pass [`CONDITIONAL_VAULT_ID`] for vault CPIs, [`AMM_ID`] for AMM
/// CPIs.
pub fn event_authority_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&event_authority_seeds(), program_id)
}

// ─────────────────────────────────────────────────────────────────────────────
// Args encoders (discriminator ++ Borsh body), no_std / no-alloc
// ─────────────────────────────────────────────────────────────────────────────

/// `initialize_question` instruction data.
///
/// Layout: `disc[8] ++ question_id[32] ++ oracle[32] ++ num_outcomes[1]`.
pub fn initialize_question_data(
    question_id: &[u8; 32],
    oracle: &Pubkey,
    num_outcomes: u8,
) -> [u8; 73] {
    let mut out = [0u8; 73];
    out[0..8].copy_from_slice(&INITIALIZE_QUESTION);
    out[8..40].copy_from_slice(question_id);
    out[40..72].copy_from_slice(oracle.as_ref());
    out[72] = num_outcomes;
    out
}

/// `initialize_conditional_vault` instruction data (no args).
pub fn initialize_conditional_vault_data() -> [u8; 8] {
    INITIALIZE_CONDITIONAL_VAULT
}

/// `split_tokens` instruction data. Layout: `disc[8] ++ amount[8 LE]`.
pub fn split_tokens_data(amount: u64) -> [u8; 16] {
    interact_data(&SPLIT_TOKENS, amount)
}

/// `merge_tokens` instruction data. Layout: `disc[8] ++ amount[8 LE]`.
pub fn merge_tokens_data(amount: u64) -> [u8; 16] {
    interact_data(&MERGE_TOKENS, amount)
}

/// `redeem_tokens` instruction data — NO args (just the discriminator). Validated
/// against the deployed v0.4 `conditional_vault` source: `handle_redeem_tokens`
/// takes no instruction args; it burns the holder's FULL balance of every
/// outcome's conditional token and transfers
/// `Σ_i balance_i × payout_numerators[i] / payout_denominator` underlying out of
/// the vault to the holder. For a binary pass-wins `[1,0]`: pass-balance redeems
/// 1:1, fail-balance → 0 (both burned); fail-wins `[0,1]` is symmetric. Uses the
/// SAME `InteractWithVault` account struct as `split_tokens` (see the account
/// ordering note below).
pub fn redeem_tokens_data() -> [u8; 8] {
    REDEEM_TOKENS
}

fn interact_data(disc: &[u8; 8], amount: u64) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[0..8].copy_from_slice(disc);
    out[8..16].copy_from_slice(&amount.to_le_bytes());
    out
}

/// `resolve_question` instruction data for a BINARY (2-outcome) question.
///
/// Layout: `disc[8] ++ len:u32 LE (== 2) ++ payout_numerators[0]:u32 LE ++
/// payout_numerators[1]:u32 LE`. The arg is Anchor `ResolveQuestionArgs {
/// payout_numerators: Vec<u32> }`, and a Borsh `Vec<u32>` is a **4-byte LE
/// length prefix THEN the u32 elements** — NOT a flat concatenation. No-alloc:
/// the whole thing is a fixed 20-byte buffer.
///
/// `[1, 0]` resolves PASS-side (outcome 0 pays); `[0, 1]` resolves FAIL-side
/// (outcome 1 pays). The conditional_vault requires `len == num_outcomes` and a
/// non-zero payout denominator (sum of numerators), so exactly one of the two
/// must be `1`.
pub fn resolve_question_data_binary(numerators: [u32; 2]) -> [u8; 20] {
    let mut out = [0u8; 20];
    out[0..8].copy_from_slice(&RESOLVE_QUESTION);
    out[8..12].copy_from_slice(&2u32.to_le_bytes());
    out[12..16].copy_from_slice(&numerators[0].to_le_bytes());
    out[16..20].copy_from_slice(&numerators[1].to_le_bytes());
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Account orderings (for the program-side CPI, Task 10)
// ─────────────────────────────────────────────────────────────────────────────
//
// `initialize_question` — accounts:
//   0 question (w, PDA, init)   1 payer (signer, w)   2 system_program
//   3 event_authority           4 conditional_vault program id
//
// `initialize_conditional_vault` — accounts:
//   0 vault (w, PDA, init)      1 question              2 underlying_token_mint
//   3 vault_underlying_ata (w, init_if_needed)          4 payer (signer, w)
//   5 token_program             6 associated_token_program   7 system_program
//   8 event_authority           9 conditional_vault program id
//   …remaining: conditional_token_mint[0..num_outcomes] (w, PDA, created here)
//
// `split_tokens` / `merge_tokens` / `redeem_tokens` — accounts (InteractWithVault):
//   0 question                  1 vault (w)             2 vault_underlying_ata (w)
//   3 authority (signer)        4 user_underlying_ata (w)   5 token_program
//   6 event_authority           7 conditional_vault program id
//   …remaining: conditional_token_mint[0..n] (w)
//              then user_conditional_token_account[0..n] (w, owner == authority)
//
// All THREE share the identical `InteractWithVault` account struct (verified
// against the deployed v0.4 `conditional_vault` source `common.rs`); only the
// handler differs. `user_underlying_token_account` is constrained
// `token::authority = authority` + `token::mint = vault.underlying_token_mint`,
// so on a program-signed `redeem_tokens` the redeemed underlying lands in an
// account owned by the signing authority (our oracle PDA — i.e. `stake_vault`),
// and the `user_conditional_token_account[i]` must be owned by that same
// authority. `redeem_tokens` additionally requires `question.is_resolved()`.
//
// For split, the vault mints `amount` of EACH outcome's conditional token to the
// user and pulls `amount` underlying into the vault ATA. Binary (pass/fail)
// markets use num_outcomes == 2; outcome index → conditional_token_mint index.

// ─────────────────────────────────────────────────────────────────────────────
// Thin invoke wrappers
// ─────────────────────────────────────────────────────────────────────────────

/// Invoke an instruction on the `conditional_vault` program.
///
/// `metas` must be in the order documented above (including the two trailing
/// `#[event_cpi]` accounts and any remaining accounts); `infos` must be the
/// matching `AccountInfo`s in the same order. `data` is a discriminator-prefixed
/// payload from the encoders above. Pass PDA `signers` when our program must
/// authorize a split/merge of vault-held KASS.
pub fn invoke_conditional_vault_signed<A: AsRef<AccountInfo>>(
    data: &[u8],
    metas: &[InstructionAccount],
    infos: &[A],
    signers: &[Signer],
) -> ProgramResult {
    let ix = InstructionView {
        program_id: &CONDITIONAL_VAULT_ID,
        data,
        accounts: metas,
    };
    pinocchio::cpi::invoke_signed_with_slice(&ix, infos, signers)
}

/// Invoke an instruction on the `amm` program (Task 10/11 wiring).
pub fn invoke_amm_signed<A: AsRef<AccountInfo>>(
    data: &[u8],
    metas: &[InstructionAccount],
    infos: &[A],
    signers: &[Signer],
) -> ProgramResult {
    let ix = InstructionView {
        program_id: &AMM_ID,
        data,
        accounts: metas,
    };
    pinocchio::cpi::invoke_signed_with_slice(&ix, infos, signers)
}

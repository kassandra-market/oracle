//! MetaDAO `conditional_vault` + `amm` CPI wire format.
//!
//! # Resolved program IDs (authoritatively sourced — see `scripts/fetch-metadao.sh`)
//!
//! | program            | id                                            | version |
//! |--------------------|-----------------------------------------------|---------|
//! | `conditional_vault`| `VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg` | v0.4.0  |
//! | `amm`              | `AMMyu265tkBpRW21iGQxKGLaves3gKm2JcMUqfXNSpqD` | v0.4    |
//!
//! Source of truth: `github.com/metaDAOproject/programs` — the `declare_id!`s in
//! `programs/conditional_vault/src/lib.rs` (`main`) and `programs/amm/src/lib.rs`
//! (tag `v0.4`), cross-checked against `Anchor.toml` and the live mainnet-beta
//! deployments. MetaDAO governance v0.5+ moved AMM liquidity to Meteora DAMM v2
//! (`programs/damm_v2_cpi`), so `AMMyu…` is the last first-party MetaDAO AMM and
//! the one whose built-in TWAP oracle matches our decision-market design.
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

use pinocchio::{
    account_info::AccountInfo,
    instruction::{AccountMeta, Instruction, Signer},
    pubkey::{find_program_address, Pubkey},
    ProgramResult,
};
use pinocchio_pubkey::pubkey;

// ─────────────────────────────────────────────────────────────────────────────
// Program IDs
// ─────────────────────────────────────────────────────────────────────────────

/// MetaDAO `conditional_vault` v0.4.0 (mainnet-beta).
pub const CONDITIONAL_VAULT_ID: Pubkey = pubkey!("VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg");

/// MetaDAO `amm` v0.4 (mainnet-beta).
pub const AMM_ID: Pubkey = pubkey!("AMMyu265tkBpRW21iGQxKGLaves3gKm2JcMUqfXNSpqD");

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

/// `amm::create_amm` — STUB (Task 10): args = `CreateAmmArgs { twap_initial_observation: u128, twap_max_observation_change_per_update: u128 }`.
pub const CREATE_AMM: [u8; 8] = [0xf2, 0x5b, 0x15, 0xaa, 0x05, 0x44, 0x7d, 0x40];
/// `amm::add_liquidity` — STUB (Task 10).
pub const ADD_LIQUIDITY: [u8; 8] = [0xb5, 0x9d, 0x59, 0x43, 0x8f, 0xb6, 0x34, 0x48];
/// `amm::remove_liquidity` — STUB (Task 10).
pub const REMOVE_LIQUIDITY: [u8; 8] = [0x50, 0x55, 0xd1, 0x48, 0x18, 0xce, 0xb1, 0x6c];
/// `amm::swap` — STUB (Task 10): args = `SwapArgs { swap_type, input_amount: u64, output_amount_min: u64 }`.
pub const SWAP: [u8; 8] = [0xf8, 0xc6, 0x9e, 0x91, 0xe1, 0x75, 0x87, 0xc8];
/// `amm::crank_that_twap` — STUB (Task 11): refresh the TWAP observation before reading it.
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
    [SEED_QUESTION, question_id, oracle, num_outcomes]
}

/// `ConditionalVault` PDA seeds: `[b"conditional_vault", question, underlying_mint]`.
pub fn vault_seeds<'a>(question: &'a Pubkey, underlying_mint: &'a Pubkey) -> [&'a [u8]; 3] {
    [SEED_CONDITIONAL_VAULT, question, underlying_mint]
}

/// Conditional-token mint PDA seeds: `[b"conditional_token", vault, [index]]`.
pub fn conditional_token_mint_seeds<'a>(vault: &'a Pubkey, index: &'a [u8; 1]) -> [&'a [u8]; 3] {
    [SEED_CONDITIONAL_TOKEN, vault, index]
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
    find_program_address(
        &question_seeds(question_id, oracle, &[num_outcomes]),
        &CONDITIONAL_VAULT_ID,
    )
}

/// `ConditionalVault` PDA: seeds `[b"conditional_vault", question, underlying_mint]`.
pub fn vault_pda(question: &Pubkey, underlying_mint: &Pubkey) -> (Pubkey, u8) {
    find_program_address(
        &vault_seeds(question, underlying_mint),
        &CONDITIONAL_VAULT_ID,
    )
}

/// Conditional-token mint PDA for outcome `index`:
/// seeds `[b"conditional_token", vault, [index]]`.
pub fn conditional_token_mint_pda(vault: &Pubkey, index: u8) -> (Pubkey, u8) {
    find_program_address(
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
    find_program_address(&event_authority_seeds(), program_id)
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
    out[40..72].copy_from_slice(oracle);
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

fn interact_data(disc: &[u8; 8], amount: u64) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[0..8].copy_from_slice(disc);
    out[8..16].copy_from_slice(&amount.to_le_bytes());
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
// `split_tokens` / `merge_tokens` — accounts (InteractWithVault):
//   0 question                  1 vault (w)             2 vault_underlying_ata (w)
//   3 authority (signer)        4 user_underlying_ata (w)   5 token_program
//   6 event_authority           7 conditional_vault program id
//   …remaining: conditional_token_mint[0..n] (w)
//              then user_conditional_token_account[0..n] (w, owner == authority)
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
pub fn invoke_conditional_vault_signed(
    data: &[u8],
    metas: &[AccountMeta],
    infos: &[&AccountInfo],
    signers: &[Signer],
) -> ProgramResult {
    let ix = Instruction {
        program_id: &CONDITIONAL_VAULT_ID,
        data,
        accounts: metas,
    };
    pinocchio::cpi::slice_invoke_signed(&ix, infos, signers)
}

/// Invoke an instruction on the `amm` program (Task 10/11 wiring).
pub fn invoke_amm_signed(
    data: &[u8],
    metas: &[AccountMeta],
    infos: &[&AccountInfo],
    signers: &[Signer],
) -> ProgramResult {
    let ix = Instruction {
        program_id: &AMM_ID,
        data,
        accounts: metas,
    };
    pinocchio::cpi::slice_invoke_signed(&ix, infos, signers)
}

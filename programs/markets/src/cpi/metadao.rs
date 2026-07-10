//! MetaDAO `conditional_vault` + `amm` CPI wire format — program side.
//!
//! `activate` invokes exactly TWO MetaDAO CPIs, both Market-PDA-signed:
//! `split_tokens` (conditional_vault) and `add_liquidity` (amm). Everything else
//! (`initialize_question` / `initialize_conditional_vault` / `create_amm`) is
//! composed CLIENT-side (see `kassandra-markets-sdk::metadao`); the program only
//! VERIFIES those accounts via the offset readers below.
//!
//! Discriminators, offsets, and program IDs are ported + re-verified against
//! `../kassandra/programs/oracles/src/cpi/metadao.rs`. `tests/parity.rs`
//! asserts they equal the sdks/oracles/rust copies byte-for-byte.

#![allow(dead_code)]

use pinocchio::{
    account::AccountView,
    address::Address,
    cpi::{invoke_signed, Signer},
    error::ProgramError,
    instruction::{InstructionAccount, InstructionView},
    ProgramResult,
};

use crate::error::MarketError;

// ─────────────────────────────────────────────────────────────────────────────
// Program IDs
// ─────────────────────────────────────────────────────────────────────────────

/// MetaDAO `conditional_vault` v0.4.0 (mainnet-beta).
pub const CONDITIONAL_VAULT_ID: Address =
    Address::from_str_const("VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg");
/// MetaDAO `amm` v0.4.2 delayed-twap (mainnet-beta).
pub const AMM_ID: Address = Address::from_str_const("AMMyu265tkBpRW21iGQxKGLaves3gKm2JcMUqfXNSpqD");
/// SPL Associated-Token-Account program (classic SPL Token).
pub const ASSOCIATED_TOKEN_PROGRAM_ID: Address =
    Address::from_str_const("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

// ─────────────────────────────────────────────────────────────────────────────
// Anchor instruction discriminators — sha256("global:<name>")[..8]
// ─────────────────────────────────────────────────────────────────────────────

/// `conditional_vault::split_tokens`
pub const SPLIT_TOKENS_DISC: [u8; 8] = [0x4f, 0xc3, 0x74, 0x00, 0x8c, 0xb0, 0x49, 0xb3];
/// `conditional_vault::resolve_question`
pub const RESOLVE_QUESTION_DISC: [u8; 8] = [0x34, 0x20, 0xe0, 0xb3, 0xb4, 0x08, 0x00, 0xf6];
/// `conditional_vault::redeem_tokens` (`sha256("global:redeem_tokens")[..8]`) —
/// mirrors `sdks/oracles/rust::metadao::REDEEM_TOKENS_DISC` / the TS SDK `DISC.redeemTokens`.
pub const REDEEM_TOKENS_DISC: [u8; 8] = [0xf6, 0x62, 0x86, 0x29, 0x98, 0x21, 0x78, 0x45];
/// `amm::add_liquidity`
pub const ADD_LIQUIDITY_DISC: [u8; 8] = [0xb5, 0x9d, 0x59, 0x43, 0x8f, 0xb6, 0x34, 0x48];
/// `amm::remove_liquidity` (`sha256("global:remove_liquidity")[..8]`). Not present
/// in the sibling SDKs (they never compose it); the value is the canonical Anchor
/// discriminator, verified in `tests/cpi_wire.rs`.
pub const REMOVE_LIQUIDITY_DISC: [u8; 8] = [0x50, 0x55, 0xd1, 0x48, 0x18, 0xce, 0xb1, 0x6c];

/// `Amm` account discriminator (`sha256("account:Amm")[..8]`) — checked as
/// defense-in-depth when `activate` verifies the composed pool.
pub const AMM_ACCOUNT_DISCRIMINATOR: [u8; 8] = [0x8f, 0xf5, 0xc8, 0x11, 0x4a, 0xd6, 0xc4, 0x87];

// ─────────────────────────────────────────────────────────────────────────────
// PDA seeds
// ─────────────────────────────────────────────────────────────────────────────

/// `Question` PDA seed prefix.
pub const SEED_QUESTION: &[u8] = b"question";
/// `ConditionalVault` PDA seed prefix.
pub const SEED_CONDITIONAL_VAULT: &[u8] = b"conditional_vault";
/// Conditional-token mint PDA seed prefix.
pub const SEED_CONDITIONAL_TOKEN: &[u8] = b"conditional_token";
/// Anchor `#[event_cpi]` event-authority PDA seed.
pub const SEED_EVENT_AUTHORITY: &[u8] = b"__event_authority";
/// `Amm` PDA seed prefix.
pub const SEED_AMM: &[u8] = b"amm__";
/// AMM LP-mint PDA seed prefix.
pub const SEED_AMM_LP_MINT: &[u8] = b"amm_lp_mint";

// ─────────────────────────────────────────────────────────────────────────────
// Account layout byte offsets (all fields are AFTER the 8-byte Anchor disc)
// ─────────────────────────────────────────────────────────────────────────────

/// `Question.oracle: Pubkey`.
pub const QUESTION_ORACLE_OFFSET: usize = 40;
/// `Question.payout_numerators: Vec<u32>` length-prefix (== num_outcomes at init).
pub const QUESTION_NUM_OUTCOMES_LEN_OFFSET: usize = 72;
/// `Question.payout_numerators[0]: u32` (cYES numerator; 0 until resolved).
pub const QUESTION_NUM0_OFFSET: usize = 76;
/// `Question.payout_numerators[1]: u32` (cNO numerator; 0 until resolved).
pub const QUESTION_NUM1_OFFSET: usize = 80;
/// `Question.payout_denominator: u32` (0 until resolved; `>0` ⇔ resolved).
pub const QUESTION_DENOMINATOR_OFFSET: usize = 84;
/// `ConditionalVault.question: Pubkey`.
pub const VAULT_QUESTION_OFFSET: usize = 8;
/// `ConditionalVault.underlying_token_mint: Pubkey`.
pub const VAULT_UNDERLYING_MINT_OFFSET: usize = 40;
/// `ConditionalVault.underlying_token_account: Pubkey`.
pub const VAULT_UNDERLYING_ACCOUNT_OFFSET: usize = 72;
/// `Amm.base_mint: Pubkey`.
pub const AMM_BASE_MINT_OFFSET: usize = 49;
/// `Amm.quote_mint: Pubkey`.
pub const AMM_QUOTE_MINT_OFFSET: usize = 81;
/// `Amm.base_amount: u64` — base-side reserve (0 on a freshly created pool).
pub const AMM_BASE_AMOUNT_OFFSET: usize = 115;
/// `Amm.quote_amount: u64` — quote-side reserve (0 on a freshly created pool).
pub const AMM_QUOTE_AMOUNT_OFFSET: usize = 123;

/// SPL `Mint.supply: u64` byte offset (`mint_authority: COption<Pubkey>` = 4+32).
/// Used to read the AMM LP mint's total supply for the accrued-fee math.
pub const MINT_SUPPLY_OFFSET: usize = 36;

// ─────────────────────────────────────────────────────────────────────────────
// Little-endian bounded field readers (out-of-bounds → InvalidAccount)
// ─────────────────────────────────────────────────────────────────────────────

/// Read a 32-byte pubkey out of `data` at byte `off`, or `InvalidAccount`.
pub fn read_pubkey(data: &[u8], off: usize) -> Result<Address, ProgramError> {
    data.get(off..off + 32)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| MarketError::InvalidAccount.into())
}

/// Read a little-endian `u32` out of `data` at byte `off`, or `InvalidAccount`.
pub fn read_u32(data: &[u8], off: usize) -> Result<u32, ProgramError> {
    data.get(off..off + 4)
        .and_then(|s| s.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| MarketError::InvalidAccount.into())
}

/// Read a little-endian `u64` out of `data` at byte `off`, or `InvalidAccount`.
pub fn read_u64(data: &[u8], off: usize) -> Result<u64, ProgramError> {
    data.get(off..off + 8)
        .and_then(|s| s.try_into().ok())
        .map(u64::from_le_bytes)
        .ok_or_else(|| MarketError::InvalidAccount.into())
}

// ─────────────────────────────────────────────────────────────────────────────
// Arg encoders (discriminator ++ Borsh body), no_std / no-alloc
// ─────────────────────────────────────────────────────────────────────────────

/// `split_tokens` instruction data. Layout: `disc[8] ++ amount[8 LE]`.
pub fn split_tokens_data(amount: u64) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[0..8].copy_from_slice(&SPLIT_TOKENS_DISC);
    out[8..16].copy_from_slice(&amount.to_le_bytes());
    out
}

/// `add_liquidity` instruction data. Layout:
/// `disc[8] ++ quote_amount[8 LE] ++ max_base_amount[8 LE] ++ min_lp_tokens[8 LE]`.
pub fn add_liquidity_data(quote_amount: u64, max_base_amount: u64, min_lp_tokens: u64) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[0..8].copy_from_slice(&ADD_LIQUIDITY_DISC);
    out[8..16].copy_from_slice(&quote_amount.to_le_bytes());
    out[16..24].copy_from_slice(&max_base_amount.to_le_bytes());
    out[24..32].copy_from_slice(&min_lp_tokens.to_le_bytes());
    out
}

/// `remove_liquidity` instruction data. Layout:
/// `disc[8] ++ lp_tokens_to_burn[8 LE] ++ min_base_amount[8 LE] ++ min_quote_amount[8 LE]`.
///
/// `collect_fee` always passes `min_base == min_quote == 0`, so any ambiguity in
/// the relative order of the two slippage mins is immaterial (both zero); only
/// `lp_tokens_to_burn` (the first arg) is load-bearing.
pub fn remove_liquidity_data(
    lp_tokens_to_burn: u64,
    min_base_amount: u64,
    min_quote_amount: u64,
) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[0..8].copy_from_slice(&REMOVE_LIQUIDITY_DISC);
    out[8..16].copy_from_slice(&lp_tokens_to_burn.to_le_bytes());
    out[16..24].copy_from_slice(&min_base_amount.to_le_bytes());
    out[24..32].copy_from_slice(&min_quote_amount.to_le_bytes());
    out
}

/// `redeem_tokens` instruction data — discriminator only (no args). Requires the
/// bound `Question` to be resolved; burns the holder's FULL conditional balances
/// and pays `Σ balance_i · numerator_i / denominator` underlying out.
pub fn redeem_tokens_data() -> [u8; 8] {
    REDEEM_TOKENS_DISC
}

/// `resolve_question` instruction data for a BINARY (2-outcome) question.
///
/// Layout: `disc[8] ++ len:u32 LE (== 2) ++ payout_numerators[0]:u32 LE ++
/// payout_numerators[1]:u32 LE` (20 bytes). The arg is Anchor
/// `ResolveQuestionArgs { payout_numerators: Vec<u32> }`; a Borsh `Vec<u32>` is a
/// 4-byte LE length prefix THEN the elements. `[1,0]` = outcome 0 pays, `[0,1]` =
/// outcome 1 pays, `[1,1]` = void (denominator 2, each leg pays half).
pub fn resolve_question_data_binary(numerators: [u32; 2]) -> [u8; 20] {
    let mut out = [0u8; 20];
    out[0..8].copy_from_slice(&RESOLVE_QUESTION_DISC);
    out[8..12].copy_from_slice(&2u32.to_le_bytes());
    out[12..16].copy_from_slice(&numerators[0].to_le_bytes());
    out[16..20].copy_from_slice(&numerators[1].to_le_bytes());
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Seed builders + PDA derivation (SBF-only syscall)
// ─────────────────────────────────────────────────────────────────────────────

/// `Question` PDA seeds: `[b"question", question_id, oracle, [num_outcomes]]`.
pub fn question_seeds<'a>(
    question_id: &'a [u8; 32],
    oracle: &'a Address,
    num_outcomes: &'a [u8; 1],
) -> [&'a [u8]; 4] {
    [SEED_QUESTION, question_id, oracle.as_ref(), num_outcomes]
}

/// `ConditionalVault` PDA seeds: `[b"conditional_vault", question, underlying_mint]`.
pub fn vault_seeds<'a>(question: &'a Address, underlying_mint: &'a Address) -> [&'a [u8]; 3] {
    [
        SEED_CONDITIONAL_VAULT,
        question.as_ref(),
        underlying_mint.as_ref(),
    ]
}

/// Conditional-token mint PDA seeds: `[b"conditional_token", vault, [index]]`.
pub fn conditional_token_mint_seeds<'a>(vault: &'a Address, index: &'a [u8; 1]) -> [&'a [u8]; 3] {
    [SEED_CONDITIONAL_TOKEN, vault.as_ref(), index]
}

/// `#[event_cpi]` event-authority PDA seeds: `[b"__event_authority"]`.
pub fn event_authority_seeds() -> [&'static [u8]; 1] {
    [SEED_EVENT_AUTHORITY]
}

/// `Amm` PDA seeds: `[b"amm__", base_mint, quote_mint]`.
pub fn amm_seeds<'a>(base_mint: &'a Address, quote_mint: &'a Address) -> [&'a [u8]; 3] {
    [SEED_AMM, base_mint.as_ref(), quote_mint.as_ref()]
}

/// AMM LP-mint PDA seeds: `[b"amm_lp_mint", amm]`.
pub fn amm_lp_mint_seeds(amm: &Address) -> [&[u8]; 2] {
    [SEED_AMM_LP_MINT, amm.as_ref()]
}

/// `Question` PDA.
pub fn question_pda(question_id: &[u8; 32], oracle: &Address, num_outcomes: u8) -> (Address, u8) {
    Address::find_program_address(
        &question_seeds(question_id, oracle, &[num_outcomes]),
        &CONDITIONAL_VAULT_ID,
    )
}

/// `ConditionalVault` PDA.
pub fn vault_pda(question: &Address, underlying_mint: &Address) -> (Address, u8) {
    Address::find_program_address(
        &vault_seeds(question, underlying_mint),
        &CONDITIONAL_VAULT_ID,
    )
}

/// Conditional-token mint PDA for outcome `index`.
pub fn conditional_token_mint_pda(vault: &Address, index: u8) -> (Address, u8) {
    Address::find_program_address(
        &conditional_token_mint_seeds(vault, &[index]),
        &CONDITIONAL_VAULT_ID,
    )
}

/// `#[event_cpi]` event-authority PDA for `program_id` (vault OR amm).
pub fn event_authority_pda(program_id: &Address) -> (Address, u8) {
    Address::find_program_address(&event_authority_seeds(), program_id)
}

/// `Amm` PDA.
pub fn amm_pda(base_mint: &Address, quote_mint: &Address) -> (Address, u8) {
    Address::find_program_address(&amm_seeds(base_mint, quote_mint), &AMM_ID)
}

/// AMM LP-mint PDA.
pub fn amm_lp_mint_pda(amm: &Address) -> (Address, u8) {
    Address::find_program_address(&amm_lp_mint_seeds(amm), &AMM_ID)
}

/// Classic SPL associated-token-account address for `owner`/`mint` — the AMM's
/// per-mint vault ATA is `ata(amm, conditional_mint)`.
pub fn associated_token_address(owner: &Address, mint: &Address) -> (Address, u8) {
    Address::find_program_address(
        &[owner.as_ref(), pinocchio_token::ID.as_ref(), mint.as_ref()],
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Thin invoke wrappers over the const-generic `cpi::invoke_signed`
// ─────────────────────────────────────────────────────────────────────────────

/// Invoke a Market-PDA-signed instruction on the `conditional_vault` program
/// (`split_tokens` at `activate`, `resolve_question` at `resolve_market`).
/// `metas`/`infos` must be in the documented order (`N` is the fixed account
/// count of the composed CPI).
pub fn invoke_conditional_vault_signed<const N: usize>(
    data: &[u8],
    metas: &[InstructionAccount; N],
    infos: &[&AccountView; N],
    signers: &[Signer],
) -> ProgramResult {
    let ix = InstructionView {
        program_id: &CONDITIONAL_VAULT_ID,
        data,
        accounts: metas,
    };
    invoke_signed(&ix, infos, signers)
}

/// Invoke a Market-PDA-signed instruction on the `amm` program (`add_liquidity`).
pub fn invoke_amm_signed<const N: usize>(
    data: &[u8],
    metas: &[InstructionAccount; N],
    infos: &[&AccountView; N],
    signers: &[Signer],
) -> ProgramResult {
    let ix = InstructionView {
        program_id: &AMM_ID,
        data,
        accounts: metas,
    };
    invoke_signed(&ix, infos, signers)
}

//! MetaDAO `conditional_vault` (v0.4.0) + `amm` (v0.4.2) CPI wire format —
//! client-side composition. This is the SINGLE SOURCE OF TRUTH for the
//! discriminators, PDA seeds, and account orders the keeper/test harness uses to
//! compose the MetaDAO market BEFORE calling `kassandra-market::activate`. The
//! program crate re-declares only the subset it invokes (`split_tokens`,
//! `add_liquidity`) and `tests/parity.rs` asserts they agree byte-for-byte.
//!
//! Ported + re-verified against `../kassandra/programs/kassandra/src/cpi/metadao.rs`
//! and the real account orders realized in `../kassandra/programs/kassandra/tests/
//! challenge_e2e.rs` (`build_pool`, `setup_market`).

use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey,
    pubkey::Pubkey,
    system_program,
};

// ─────────────────────────────────────────────────────────────────────────────
// Program IDs
// ─────────────────────────────────────────────────────────────────────────────

/// MetaDAO `conditional_vault` v0.4.0 (mainnet-beta).
pub const CONDITIONAL_VAULT_ID: Pubkey = pubkey!("VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg");
/// MetaDAO `amm` v0.4.2 delayed-twap (mainnet-beta).
pub const AMM_ID: Pubkey = pubkey!("AMMyu265tkBpRW21iGQxKGLaves3gKm2JcMUqfXNSpqD");

/// SPL Token program.
pub const TOKEN_PROGRAM_ID: Pubkey = pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
/// SPL Associated-Token-Account program.
pub const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey =
    pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

// ─────────────────────────────────────────────────────────────────────────────
// Anchor instruction discriminators — sha256("global:<name>")[..8]
// ─────────────────────────────────────────────────────────────────────────────

/// `conditional_vault::initialize_question`
pub const INITIALIZE_QUESTION_DISC: [u8; 8] = [0xf5, 0x97, 0x6a, 0xbc, 0x58, 0x2c, 0x41, 0xd4];
/// `conditional_vault::initialize_conditional_vault`
pub const INITIALIZE_CONDITIONAL_VAULT_DISC: [u8; 8] =
    [0x25, 0x58, 0xfa, 0xd4, 0x36, 0xda, 0xe3, 0xaf];
/// `conditional_vault::split_tokens`
pub const SPLIT_TOKENS_DISC: [u8; 8] = [0x4f, 0xc3, 0x74, 0x00, 0x8c, 0xb0, 0x49, 0xb3];
/// `conditional_vault::merge_tokens`
pub const MERGE_TOKENS_DISC: [u8; 8] = [0xe2, 0x59, 0xfb, 0x79, 0xe1, 0x82, 0xb4, 0x0e];
/// `conditional_vault::redeem_tokens`
pub const REDEEM_TOKENS_DISC: [u8; 8] = [0xf6, 0x62, 0x86, 0x29, 0x98, 0x21, 0x78, 0x45];
/// `conditional_vault::resolve_question`
pub const RESOLVE_QUESTION_DISC: [u8; 8] = [0x34, 0x20, 0xe0, 0xb3, 0xb4, 0x08, 0x00, 0xf6];
/// `amm::create_amm`
pub const CREATE_AMM_DISC: [u8; 8] = [0xf2, 0x5b, 0x15, 0xaa, 0x05, 0x44, 0x7d, 0x40];
/// `amm::add_liquidity`
pub const ADD_LIQUIDITY_DISC: [u8; 8] = [0xb5, 0x9d, 0x59, 0x43, 0x8f, 0xb6, 0x34, 0x48];
/// `amm::swap`
pub const SWAP_DISC: [u8; 8] = [0xf8, 0xc6, 0x9e, 0x91, 0xe1, 0x75, 0x87, 0xc8];

/// `amm::SwapType` Borsh tag: `Buy` (quote→base) or `Sell` (base→quote).
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SwapType {
    Buy = 0,
    Sell = 1,
}

/// `Amm` account discriminator (`sha256("account:Amm")[..8]`).
pub const AMM_ACCOUNT_DISCRIMINATOR: [u8; 8] = [0x8f, 0xf5, 0xc8, 0x11, 0x4a, 0xd6, 0xc4, 0x87];

// ─────────────────────────────────────────────────────────────────────────────
// PDA seeds
// ─────────────────────────────────────────────────────────────────────────────

const SEED_QUESTION: &[u8] = b"question";
const SEED_CONDITIONAL_VAULT: &[u8] = b"conditional_vault";
const SEED_CONDITIONAL_TOKEN: &[u8] = b"conditional_token";
const SEED_EVENT_AUTHORITY: &[u8] = b"__event_authority";
const SEED_AMM: &[u8] = b"amm__";
const SEED_AMM_LP_MINT: &[u8] = b"amm_lp_mint";

// ─────────────────────────────────────────────────────────────────────────────
// PDA derivers
// ─────────────────────────────────────────────────────────────────────────────

/// `Question` PDA: seeds `[b"question", question_id, oracle_authority, [num_outcomes]]`.
pub fn question(
    question_id: &[u8; 32],
    oracle_authority: &Pubkey,
    num_outcomes: u8,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            SEED_QUESTION,
            question_id,
            oracle_authority.as_ref(),
            &[num_outcomes],
        ],
        &CONDITIONAL_VAULT_ID,
    )
}

/// `ConditionalVault` PDA: seeds `[b"conditional_vault", question, underlying_mint]`.
pub fn vault(question: &Pubkey, underlying_mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            SEED_CONDITIONAL_VAULT,
            question.as_ref(),
            underlying_mint.as_ref(),
        ],
        &CONDITIONAL_VAULT_ID,
    )
}

/// Conditional-token mint PDA for `index`: seeds `[b"conditional_token", vault, [index]]`.
pub fn conditional_token_mint(vault: &Pubkey, index: u8) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[SEED_CONDITIONAL_TOKEN, vault.as_ref(), &[index]],
        &CONDITIONAL_VAULT_ID,
    )
}

/// `#[event_cpi]` event-authority PDA under `program_id`: seeds `[b"__event_authority"]`.
pub fn event_authority(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[SEED_EVENT_AUTHORITY], program_id)
}

/// `Amm` PDA: seeds `[b"amm__", base_mint, quote_mint]`.
pub fn amm(base_mint: &Pubkey, quote_mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[SEED_AMM, base_mint.as_ref(), quote_mint.as_ref()],
        &AMM_ID,
    )
}

/// AMM LP-mint PDA: seeds `[b"amm_lp_mint", amm]`.
pub fn amm_lp_mint(amm: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[SEED_AMM_LP_MINT, amm.as_ref()], &AMM_ID)
}

/// Associated token account for `owner`/`mint` (classic SPL Token program).
pub fn ata(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[owner.as_ref(), TOKEN_PROGRAM_ID.as_ref(), mint.as_ref()],
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    )
    .0
}

// ─────────────────────────────────────────────────────────────────────────────
// Arg encoders (discriminator ++ Borsh body)
// ─────────────────────────────────────────────────────────────────────────────

/// `initialize_question` data: `disc[8] ++ question_id[32] ++ oracle[32] ++ num_outcomes[1]`.
pub fn initialize_question_data(
    question_id: &[u8; 32],
    oracle_authority: &Pubkey,
    num_outcomes: u8,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(73);
    out.extend_from_slice(&INITIALIZE_QUESTION_DISC);
    out.extend_from_slice(question_id);
    out.extend_from_slice(oracle_authority.as_ref());
    out.push(num_outcomes);
    out
}

/// `initialize_conditional_vault` data — discriminator only.
pub fn initialize_conditional_vault_data() -> Vec<u8> {
    INITIALIZE_CONDITIONAL_VAULT_DISC.to_vec()
}

/// `split_tokens` data: `disc[8] ++ amount[8 LE]`.
pub fn split_tokens_data(amount: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(16);
    out.extend_from_slice(&SPLIT_TOKENS_DISC);
    out.extend_from_slice(&amount.to_le_bytes());
    out
}

/// `merge_tokens` data: `disc[8] ++ amount[8 LE]`.
pub fn merge_tokens_data(amount: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(16);
    out.extend_from_slice(&MERGE_TOKENS_DISC);
    out.extend_from_slice(&amount.to_le_bytes());
    out
}

/// `redeem_tokens` data — discriminator only (no args).
pub fn redeem_tokens_data() -> Vec<u8> {
    REDEEM_TOKENS_DISC.to_vec()
}

/// `create_amm` data: `disc[8] ++ twap_initial_observation[u128] ++
/// twap_max_observation_change_per_update[u128] ++ twap_start_delay_slots[u64]`.
pub fn create_amm_data(
    twap_initial_observation: u128,
    twap_max_change: u128,
    twap_start_delay: u64,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(40);
    out.extend_from_slice(&CREATE_AMM_DISC);
    out.extend_from_slice(&twap_initial_observation.to_le_bytes());
    out.extend_from_slice(&twap_max_change.to_le_bytes());
    out.extend_from_slice(&twap_start_delay.to_le_bytes());
    out
}

/// `add_liquidity` data: `disc[8] ++ quote_amount[u64] ++ max_base_amount[u64] ++
/// min_lp_tokens[u64]`.
pub fn add_liquidity_data(quote_amount: u64, max_base_amount: u64, min_lp_tokens: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    out.extend_from_slice(&ADD_LIQUIDITY_DISC);
    out.extend_from_slice(&quote_amount.to_le_bytes());
    out.extend_from_slice(&max_base_amount.to_le_bytes());
    out.extend_from_slice(&min_lp_tokens.to_le_bytes());
    out
}

/// `swap` data: `disc[8] ++ swap_type[u8] ++ input_amount[u64] ++
/// output_amount_min[u64]`.
pub fn swap_data(swap_type: SwapType, input_amount: u64, output_amount_min: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(25);
    out.extend_from_slice(&SWAP_DISC);
    out.push(swap_type as u8);
    out.extend_from_slice(&input_amount.to_le_bytes());
    out.extend_from_slice(&output_amount_min.to_le_bytes());
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Client instruction builders (compose the MetaDAO market before `activate`)
//
// Account orders mirror `challenge_e2e.rs` EXACTLY (`setup_market`, `build_pool`).
// ─────────────────────────────────────────────────────────────────────────────

/// `initialize_question` — 5 accounts (incl. the two `#[event_cpi]` trailers).
pub fn initialize_question(
    payer: &Pubkey,
    oracle_authority: &Pubkey,
    question_id: &[u8; 32],
    num_outcomes: u8,
) -> Instruction {
    let (question, _) = question(question_id, oracle_authority, num_outcomes);
    let (event_auth, _) = event_authority(&CONDITIONAL_VAULT_ID);
    Instruction {
        program_id: CONDITIONAL_VAULT_ID,
        accounts: vec![
            AccountMeta::new(question, false),
            AccountMeta::new(*payer, true),
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(event_auth, false),
            AccountMeta::new_readonly(CONDITIONAL_VAULT_ID, false),
        ],
        data: initialize_question_data(question_id, oracle_authority, num_outcomes),
    }
}

/// `initialize_conditional_vault` — 10 fixed accounts + `num_outcomes` trailing
/// conditional-token mints (w, PDA). Binary markets pass `num_outcomes == 2`.
pub fn initialize_conditional_vault(
    payer: &Pubkey,
    question: &Pubkey,
    underlying_mint: &Pubkey,
    num_outcomes: u8,
) -> Instruction {
    let (vault, _) = vault(question, underlying_mint);
    let vault_underlying_ata = ata(&vault, underlying_mint);
    let (event_auth, _) = event_authority(&CONDITIONAL_VAULT_ID);
    let mut accounts = vec![
        AccountMeta::new(vault, false),
        AccountMeta::new_readonly(*question, false),
        AccountMeta::new_readonly(*underlying_mint, false),
        AccountMeta::new(vault_underlying_ata, false),
        AccountMeta::new(*payer, true),
        AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        AccountMeta::new_readonly(ASSOCIATED_TOKEN_PROGRAM_ID, false),
        AccountMeta::new_readonly(system_program::ID, false),
        AccountMeta::new_readonly(event_auth, false),
        AccountMeta::new_readonly(CONDITIONAL_VAULT_ID, false),
    ];
    for index in 0..num_outcomes {
        let (mint, _) = conditional_token_mint(&vault, index);
        accounts.push(AccountMeta::new(mint, false));
    }
    Instruction {
        program_id: CONDITIONAL_VAULT_ID,
        accounts,
        data: initialize_conditional_vault_data(),
    }
}

/// `create_amm` — 12 accounts. `base_mint`/`quote_mint` are the conditional
/// mints (cYES = base, cNO = quote for our single-pool probability market).
pub fn create_amm(
    payer: &Pubkey,
    base_mint: &Pubkey,
    quote_mint: &Pubkey,
    twap_initial_observation: u128,
    twap_max_change: u128,
    twap_start_delay: u64,
) -> Instruction {
    let (amm, _) = amm(base_mint, quote_mint);
    let (lp_mint, _) = amm_lp_mint(&amm);
    let vault_ata_base = ata(&amm, base_mint);
    let vault_ata_quote = ata(&amm, quote_mint);
    let (event_auth, _) = event_authority(&AMM_ID);
    Instruction {
        program_id: AMM_ID,
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(amm, false),
            AccountMeta::new(lp_mint, false),
            AccountMeta::new_readonly(*base_mint, false),
            AccountMeta::new_readonly(*quote_mint, false),
            AccountMeta::new(vault_ata_base, false),
            AccountMeta::new(vault_ata_quote, false),
            AccountMeta::new_readonly(ASSOCIATED_TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(event_auth, false),
            AccountMeta::new_readonly(AMM_ID, false),
        ],
        data: create_amm_data(twap_initial_observation, twap_max_change, twap_start_delay),
    }
}

/// `add_liquidity` — 11 accounts. `authority` owns `user_lp`/`user_base`/
/// `user_quote`; in `activate` the authority is the Market PDA.
#[allow(clippy::too_many_arguments)]
pub fn add_liquidity(
    authority: &Pubkey,
    base_mint: &Pubkey,
    quote_mint: &Pubkey,
    user_lp: &Pubkey,
    user_base: &Pubkey,
    user_quote: &Pubkey,
    quote_amount: u64,
    max_base_amount: u64,
    min_lp_tokens: u64,
) -> Instruction {
    let (amm, _) = amm(base_mint, quote_mint);
    let (lp_mint, _) = amm_lp_mint(&amm);
    let vault_ata_base = ata(&amm, base_mint);
    let vault_ata_quote = ata(&amm, quote_mint);
    let (event_auth, _) = event_authority(&AMM_ID);
    Instruction {
        program_id: AMM_ID,
        accounts: vec![
            AccountMeta::new(*authority, true),
            AccountMeta::new(amm, false),
            AccountMeta::new(lp_mint, false),
            AccountMeta::new(*user_lp, false),
            AccountMeta::new(*user_base, false),
            AccountMeta::new(*user_quote, false),
            AccountMeta::new(vault_ata_base, false),
            AccountMeta::new(vault_ata_quote, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(event_auth, false),
            AccountMeta::new_readonly(AMM_ID, false),
        ],
        data: add_liquidity_data(quote_amount, max_base_amount, min_lp_tokens),
    }
}

/// The shared `InteractWithVault` account list for `split_tokens` / `merge_tokens`
/// / `redeem_tokens` on a BINARY vault. Order (verified against the deployed v0.4
/// `conditional_vault` `common.rs`):
///   0 question(ro) 1 vault(w) 2 vault_underlying_ata(w) 3 authority(signer)
///   4 user_underlying_ata(w) 5 token_program 6 event_authority 7 cv_program
///   8 yes_mint(w) 9 no_mint(w) 10 user_yes_acct(w) 11 user_no_acct(w)
/// `user_underlying_ata` is `token::authority = authority`; the two
/// `user_conditional_token_account`s must also be owned by `authority`.
#[allow(clippy::too_many_arguments)]
fn interact_with_vault_accounts(
    question: &Pubkey,
    vault: &Pubkey,
    vault_underlying_ata: &Pubkey,
    authority: &Pubkey,
    user_underlying_ata: &Pubkey,
    yes_mint: &Pubkey,
    no_mint: &Pubkey,
    user_yes_acct: &Pubkey,
    user_no_acct: &Pubkey,
) -> Vec<AccountMeta> {
    let (event_auth, _) = event_authority(&CONDITIONAL_VAULT_ID);
    vec![
        AccountMeta::new_readonly(*question, false),
        AccountMeta::new(*vault, false),
        AccountMeta::new(*vault_underlying_ata, false),
        // `authority` is a READONLY signer here (matches the program's own
        // `split_metas`/`redeem_metas` CPIs in processor/{activate,collect_fee}.rs
        // and the TS SDK's `ro(authority, true)`). The signer proves ownership of
        // the user token accounts; the account itself is never mutated.
        AccountMeta::new_readonly(*authority, true),
        AccountMeta::new(*user_underlying_ata, false),
        AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        AccountMeta::new_readonly(event_auth, false),
        AccountMeta::new_readonly(CONDITIONAL_VAULT_ID, false),
        AccountMeta::new(*yes_mint, false),
        AccountMeta::new(*no_mint, false),
        AccountMeta::new(*user_yes_acct, false),
        AccountMeta::new(*user_no_acct, false),
    ]
}

/// `split_tokens` — client builder (binary vault). `authority` signs, pays
/// `amount` underlying out of `user_underlying_ata` into the vault, and receives
/// `amount` of BOTH conditional tokens in `user_yes_acct`/`user_no_acct`.
#[allow(clippy::too_many_arguments)]
pub fn split_tokens(
    authority: &Pubkey,
    question: &Pubkey,
    vault: &Pubkey,
    vault_underlying_ata: &Pubkey,
    user_underlying_ata: &Pubkey,
    yes_mint: &Pubkey,
    no_mint: &Pubkey,
    user_yes_acct: &Pubkey,
    user_no_acct: &Pubkey,
    amount: u64,
) -> Instruction {
    Instruction {
        program_id: CONDITIONAL_VAULT_ID,
        accounts: interact_with_vault_accounts(
            question,
            vault,
            vault_underlying_ata,
            authority,
            user_underlying_ata,
            yes_mint,
            no_mint,
            user_yes_acct,
            user_no_acct,
        ),
        data: split_tokens_data(amount),
    }
}

/// `merge_tokens` — client builder (binary vault); inverse of `split`. Burns
/// `amount` of BOTH conditional tokens and returns `amount` underlying.
#[allow(clippy::too_many_arguments)]
pub fn merge_tokens(
    authority: &Pubkey,
    question: &Pubkey,
    vault: &Pubkey,
    vault_underlying_ata: &Pubkey,
    user_underlying_ata: &Pubkey,
    yes_mint: &Pubkey,
    no_mint: &Pubkey,
    user_yes_acct: &Pubkey,
    user_no_acct: &Pubkey,
    amount: u64,
) -> Instruction {
    Instruction {
        program_id: CONDITIONAL_VAULT_ID,
        accounts: interact_with_vault_accounts(
            question,
            vault,
            vault_underlying_ata,
            authority,
            user_underlying_ata,
            yes_mint,
            no_mint,
            user_yes_acct,
            user_no_acct,
        ),
        data: merge_tokens_data(amount),
    }
}

/// `swap` — 9 accounts. `authority` signs and owns `user_base`/`user_quote`.
/// `Buy` spends `user_quote` (cNO) for `user_base` (cYES); `Sell` the reverse.
/// The pool's swap fee accrues to the reserves, growing the LP position's value.
#[allow(clippy::too_many_arguments)]
pub fn swap(
    authority: &Pubkey,
    base_mint: &Pubkey,
    quote_mint: &Pubkey,
    user_base: &Pubkey,
    user_quote: &Pubkey,
    swap_type: SwapType,
    input_amount: u64,
    output_amount_min: u64,
) -> Instruction {
    let (amm, _) = amm(base_mint, quote_mint);
    let vault_base = ata(&amm, base_mint);
    let vault_quote = ata(&amm, quote_mint);
    let (event_auth, _) = event_authority(&AMM_ID);
    Instruction {
        program_id: AMM_ID,
        accounts: vec![
            AccountMeta::new(*authority, true),
            AccountMeta::new(amm, false),
            AccountMeta::new(*user_base, false),
            AccountMeta::new(*user_quote, false),
            AccountMeta::new(vault_base, false),
            AccountMeta::new(vault_quote, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(event_auth, false),
            AccountMeta::new_readonly(AMM_ID, false),
        ],
        data: swap_data(swap_type, input_amount, output_amount_min),
    }
}

/// `redeem_tokens` — client builder (binary vault). Requires the Question to be
/// resolved. Burns the holder's FULL balance of each conditional token and pays
/// `Σ balance_i × numerator_i / denominator` underlying into `user_underlying_ata`.
#[allow(clippy::too_many_arguments)]
pub fn redeem_tokens(
    authority: &Pubkey,
    question: &Pubkey,
    vault: &Pubkey,
    vault_underlying_ata: &Pubkey,
    user_underlying_ata: &Pubkey,
    yes_mint: &Pubkey,
    no_mint: &Pubkey,
    user_yes_acct: &Pubkey,
    user_no_acct: &Pubkey,
) -> Instruction {
    Instruction {
        program_id: CONDITIONAL_VAULT_ID,
        accounts: interact_with_vault_accounts(
            question,
            vault,
            vault_underlying_ata,
            authority,
            user_underlying_ata,
            yes_mint,
            no_mint,
            user_yes_acct,
            user_no_acct,
        ),
        data: redeem_tokens_data(),
    }
}

//! Shared LiteSVM test harness for the Kassandra-market program.
//!
//! Every integration test starts with `mod common; use common::*;` and builds a
//! [`TestCtx`], which deploys the compiled `.so` into a fresh [`LiteSVM`], funds
//! a payer, and exposes convenience builders for the program's instructions.
//!
//! The `.so` is `include_bytes!`'d at compile time, so `just build`
//! (`cargo build-sbf`) MUST run **before** `cargo test` — otherwise the embedded
//! bytes are stale (or missing).
//!
//! The `TestCtx` inherent methods are split across sibling modules (`setup`,
//! `fabricate`, `config`, `market_ix`, `metadao_ops`); this file keeps the shared
//! types and the free helper functions those modules build on.

#![allow(dead_code)]

// The `TestCtx` inherent-method groups. Each submodule only adds `impl TestCtx`
// blocks (no nameable items), so declaring the modules is enough — the methods
// are in scope wherever `TestCtx` is, no `pub use` re-export needed.
mod config;
mod fabricate;
mod market_ix;
mod metadao_ops;
mod setup;

use litesvm::{types::TransactionResult, LiteSVM};
use solana_sdk::{
    account::Account,
    instruction::InstructionError,
    pubkey::Pubkey,
    signature::Keypair,
    transaction::TransactionError,
};

/// Build a raw `Oracle` account body (`ORACLE_LEN` bytes) matching the sibling
/// Kassandra layout the market gate reads: tag byte 0, plus `options_count`,
/// `phase`, and `resolved_option` stamped at their exact offsets. Mirrors
/// `kassandra_markets_program::kass_oracle::KassOracle::read` so the harness and
/// the on-chain gate agree byte-for-byte.
fn kass_oracle_bytes(options_count: u8, phase: u8, resolved_option: u8) -> Vec<u8> {
    use kassandra_markets_program::kass_oracle as k;
    let mut data = vec![0u8; k::ORACLE_LEN];
    data[0] = k::ORACLE_ACCOUNT_TYPE;
    data[k::OPTIONS_COUNT_OFFSET] = options_count;
    data[k::PHASE_OFFSET] = phase;
    data[k::RESOLVED_OPTION_OFFSET] = resolved_option;
    data
}

/// The Kassandra program id that must own a fabricated oracle account.
fn kass_oracle_owner() -> Pubkey {
    Pubkey::new_from_array(kassandra_markets_program::kass_oracle::KASSANDRA_PROGRAM_ID.to_bytes())
}

/// LiteSVM-backed test context: a funded payer plus the deployed program.
pub struct TestCtx {
    pub svm: LiteSVM,
    pub payer: Keypair,
    pub program_id: Pubkey,
}

/// Fabricate the BPF-Upgradeable-Loader `ProgramData` account of `program_id`
/// with `authority` as the stored `upgrade_authority`, at the canonical PDA
/// `find_program_address([program_id], BPF_UPGRADEABLE_LOADER_ID)`.
///
/// Builds the 45-byte `UpgradeableLoaderState::ProgramData` metadata the program
/// reads: `u32 LE variant == 3 @0`, `u64 LE slot @4`, `Option::Some tag == 1 @12`,
/// then the 32-byte authority `@13..45`. The account is loader-owned + rent-exempt.
fn set_program_data(svm: &mut LiteSVM, program_id: &Pubkey, authority: &Pubkey) {
    let (program_data, _) = kassandra_markets_sdk::pda::program_data(program_id);
    let mut data = vec![0u8; 45];
    data[0..4].copy_from_slice(&3u32.to_le_bytes()); // ProgramData variant
                                                     // bytes [4..12] = slot (0)
    data[12] = 1; // Option::Some
    data[13..45].copy_from_slice(&authority.to_bytes());
    svm.set_account(
        program_data,
        Account {
            lamports: 1_000_000_000,
            data,
            owner: kassandra_markets_sdk::pda::BPF_UPGRADEABLE_LOADER_ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
}

impl Default for TestCtx {
    fn default() -> Self {
        Self::new()
    }
}

/// Derived addresses of a client-composed MetaDAO market (the precondition for
/// `activate`), returned by [`TestCtx::compose_metadao_market`].
pub struct MetaDaoRefs {
    pub question: Pubkey,
    pub vault: Pubkey,
    pub vault_underlying_ata: Pubkey,
    pub yes_mint: Pubkey,
    pub no_mint: Pubkey,
    pub amm: Pubkey,
    pub lp_mint: Pubkey,
    pub amm_vault_base: Pubkey,
    pub amm_vault_quote: Pubkey,
}

/// Decode a LiteSVM transaction error into its `Custom(u32)` code, if any.
pub fn custom_code(res: &TransactionResult) -> Option<u32> {
    match res {
        Err(meta) => match &meta.err {
            TransactionError::InstructionError(_, InstructionError::Custom(code)) => Some(*code),
            _ => None,
        },
        Ok(_) => None,
    }
}

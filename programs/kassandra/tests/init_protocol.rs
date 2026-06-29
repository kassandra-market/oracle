//! Tests for `init_protocol` (Task H0): the one-time Protocol singleton.

mod common;
use common::*;

use kassandra_program::error::KassandraError;
use kassandra_program::state::AccountType;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signer;

/// Decode a LiteSVM transaction error into its `Custom(u32)` code, if any.
fn custom_code(res: &litesvm::types::TransactionResult) -> Option<u32> {
    use solana_sdk::instruction::InstructionError;
    use solana_sdk::transaction::TransactionError;
    match res {
        Err(meta) => match &meta.err {
            TransactionError::InstructionError(_, InstructionError::Custom(code)) => Some(*code),
            _ => None,
        },
        Ok(_) => None,
    }
}

#[test]
fn init_once_records_admin_and_mints() {
    let mut ctx = TestCtx::new();
    let (protocol_pda, res) = ctx.init_protocol();
    assert!(res.is_ok(), "init_protocol should succeed: {res:?}");

    let p = ctx.protocol(protocol_pda);
    assert_eq!(p.account_type, AccountType::Protocol.as_u8());
    assert_eq!(p.admin, ctx.payer.pubkey().to_bytes());
    assert_eq!(p.kass_mint, ctx.kass_mint.to_bytes());
    assert_eq!(p.usdc_mint, ctx.usdc_mint.to_bytes());
    assert_eq!(p.fee_ema, 0);
    assert_eq!(p.last_creation_unix, 0);
}

#[test]
fn second_init_fails_already_initialized() {
    let mut ctx = TestCtx::new();
    let (_pda, res) = ctx.init_protocol();
    assert!(res.is_ok(), "first init should succeed: {res:?}");

    let (_pda2, res2) = ctx.init_protocol();
    assert_eq!(
        custom_code(&res2),
        Some(KassandraError::AlreadyInitialized as u32),
        "second init must fail AlreadyInitialized: {res2:?}"
    );
}

#[test]
fn prefunded_pda_is_adopted() {
    let mut ctx = TestCtx::new();
    let (protocol_pda, _) = TestCtx::protocol_pda(&ctx.program_id);

    // Attacker grief: drop 1 lamport on the deterministic singleton PDA before
    // anyone bootstraps. A plain CreateAccount would now fail forever.
    ctx.svm.airdrop(&protocol_pda, 1).unwrap();

    let (pda, res) = ctx.init_protocol();
    assert_eq!(pda, protocol_pda);
    assert!(
        res.is_ok(),
        "init must adopt a pre-funded PDA, not brick: {res:?}"
    );

    let p = ctx.protocol(protocol_pda);
    assert_eq!(p.account_type, AccountType::Protocol.as_u8());
    assert_eq!(p.admin, ctx.payer.pubkey().to_bytes());
    assert_eq!(p.kass_mint, ctx.kass_mint.to_bytes());
    assert_eq!(p.usdc_mint, ctx.usdc_mint.to_bytes());
    assert_eq!(p.fee_ema, 0);

    // Double-init still fails (now via the account_type tag, not lamports).
    let (_pda2, res2) = ctx.init_protocol();
    assert_eq!(
        custom_code(&res2),
        Some(KassandraError::AlreadyInitialized as u32),
        "second init must still fail AlreadyInitialized: {res2:?}"
    );
}

#[test]
fn wrong_protocol_pda_fails() {
    let mut ctx = TestCtx::new();
    // A non-canonical address for the protocol account.
    let bogus = Pubkey::new_unique();
    let ix = ctx.init_protocol_ix(bogus);
    let res = ctx.send(ix, &[]);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidAccount as u32),
        "wrong protocol PDA must fail InvalidAccount: {res:?}"
    );
}

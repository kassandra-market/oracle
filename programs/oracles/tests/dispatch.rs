//! Instruction-dispatch tests: these submit real transactions to the deployed
//! program in LiteSVM and assert on the surfaced `TransactionError`.

mod common;
use common::*;

use kassandra_oracles_program::{error::KassandraError, instruction::Ix};
use solana_instruction::{AccountMeta, Instruction};
use solana_instruction_error::InstructionError;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction_error::TransactionError;

/// Build a single instruction to the program with the given data and a single
/// (signer, writable) payer account so the transaction is well-formed.
fn ix_with_data(ctx: &TestCtx, data: Vec<u8>) -> Instruction {
    Instruction {
        program_id: Pubkey::new_from_array(kassandra_oracles_program::ID.to_bytes()),
        accounts: vec![AccountMeta::new(ctx.payer.pubkey(), true)],
        data,
    }
}

#[test]
fn empty_instruction_data_is_invalid() {
    let mut ctx = TestCtx::new();
    let ix = ix_with_data(&ctx, vec![]);
    let err = ctx.send(ix, &[]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(0, InstructionError::InvalidInstructionData),
    );
}

#[test]
fn unknown_discriminant_is_invalid() {
    let mut ctx = TestCtx::new();
    let ix = ix_with_data(&ctx, vec![0xFE]);
    let err = ctx.send(ix, &[]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(0, InstructionError::InvalidInstructionData),
    );
}

#[test]
fn known_discriminant_is_routed_to_processor() {
    // Every `Ix` variant is now implemented (no `NotImplemented` arm remains),
    // so a known discriminant is ROUTED to its processor rather than stubbed.
    // FinalizeOracle (the last one wired up) with a single non-program-owned
    // account reaches `load_oracle`, which rejects the bad owner with
    // `InvalidAccount` — proving dispatch routed it instead of returning
    // `NotImplemented`.
    let mut ctx = TestCtx::new();
    let ix = ix_with_data(&ctx, vec![Ix::FinalizeOracle as u8]);
    let err = ctx.send(ix, &[]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::InvalidAccount as u32),
        ),
    );
}

//! Instruction-dispatch tests: these submit real transactions to the deployed
//! program in LiteSVM and assert on the surfaced `TransactionError`.

mod common;
use common::*;

use kassandra_program::{error::KassandraError, instruction::Ix};
use solana_sdk::{
    instruction::{AccountMeta, Instruction, InstructionError},
    pubkey::Pubkey,
    signature::Signer,
    transaction::TransactionError,
};

/// Build a single instruction to the program with the given data and a single
/// (signer, writable) payer account so the transaction is well-formed.
fn ix_with_data(ctx: &TestCtx, data: Vec<u8>) -> Instruction {
    Instruction {
        program_id: Pubkey::new_from_array(kassandra_program::ID),
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
fn known_discriminant_returns_not_implemented() {
    let mut ctx = TestCtx::new();
    // A known discriminant with an arbitrary trailing payload byte.
    let ix = ix_with_data(&ctx, vec![Ix::SubmitFact as u8, 0xAB]);
    let err = ctx.send(ix, &[]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::NotImplemented as u32),
        ),
    );
}

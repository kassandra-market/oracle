//! `advance_phase` integration tests.
//!
//! Drives the real deployed program in LiteSVM against a seeded disputed oracle
//! (in `Phase::FactProposal`). Locks in:
//!
//! * Permissionless `FactProposal -> FactVoting` freeze once the proposal
//!   window has elapsed.
//! * Account order: `[oracle (writable)]`, no signer.
//! * Payload: `disc=7` only (empty).

mod common;
use common::*;

use kassandra_oracles_program::{error::KassandraError, state::Phase};
use solana_instruction_error::InstructionError;
use solana_pubkey::Pubkey;
use solana_transaction_error::TransactionError;

/// PHASE_WINDOW mirrored from the program config (`src/config.rs`).
const PHASE_WINDOW: i64 = 3600;

/// Seed an oracle in `FactProposal`.
fn seed() -> (TestCtx, Pubkey) {
    let mut ctx = TestCtx::new();
    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: 1_000,
        },
        ProposerSpec {
            option: 1,
            bond: 2_000,
        },
    ]);
    (ctx, oracle)
}

#[test]
fn advance_phase_before_window_end_fails() {
    let (mut ctx, oracle) = seed();

    // Still inside the proposal window: not yet advanceable.
    let ix = advance_phase_ix(&ctx, oracle);
    let err = ctx.send(ix, &[]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::WindowNotElapsed as u32),
        ),
    );

    // Phase unchanged.
    assert_eq!(ctx.oracle(oracle).phase, Phase::FactProposal as u8);
}

#[test]
fn advance_phase_after_window_end_succeeds() {
    let (mut ctx, oracle) = seed();

    // Cross phase_ends_at.
    ctx.warp(WINDOW + 1);
    let now_after_warp = ctx.now();

    let ix = advance_phase_ix(&ctx, oracle);
    ctx.send(ix, &[]).expect("advance_phase should succeed");

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::FactVoting as u8);
    // A fresh voting window opens at now + PHASE_WINDOW.
    assert_eq!(o.phase_ends_at, now_after_warp + PHASE_WINDOW);
}

#[test]
fn advance_phase_wrong_phase_fails() {
    let (mut ctx, oracle) = seed();

    // Only FactProposal -> FactVoting lives here; any other phase is rejected.
    ctx.set_phase(oracle, Phase::AiClaim);
    // Even with the window elapsed, the wrong starting phase must fail.
    ctx.warp(WINDOW + 1);

    let ix = advance_phase_ix(&ctx, oracle);
    let err = ctx.send(ix, &[]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::WrongPhase as u32),
        ),
    );
}

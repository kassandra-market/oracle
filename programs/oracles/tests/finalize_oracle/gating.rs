//! `finalize_oracle` — gating / validation failures.

use super::*;
use kassandra_oracles_program::{error::KassandraError, state::CLAIM_OPTION_NONE};
use solana_instruction_error::InstructionError;
use solana_transaction_error::TransactionError;

#[test]
fn outstanding_challenge_fails() {
    let (mut ctx, oracle, pdas) = seed_challenge(
        &[
            ProposerSpec {
                option: 0,
                bond: 1_000,
            },
            ProposerSpec {
                option: 1,
                bond: 1_000,
            },
        ],
        &[0, 0],
    );
    // One challenge market still open.
    ctx.set_open_challenge_count(oracle, 1);
    ctx.warp(WINDOW + 1);

    let err = ctx
        .send(finalize_oracle_ix(&ctx, oracle, &pdas), &[])
        .unwrap_err()
        .err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::ChallengesOutstanding as u32),
        ),
    );
}

#[test]
fn window_still_open_fails() {
    let (mut ctx, oracle, pdas) = seed_challenge(
        &[
            ProposerSpec {
                option: 0,
                bond: 1_000,
            },
            ProposerSpec {
                option: 0,
                bond: 1_000,
            },
        ],
        &[0, 0],
    );
    // No warp: the challenge window is still open.
    let err = ctx
        .send(finalize_oracle_ix(&ctx, oracle, &pdas), &[])
        .unwrap_err()
        .err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::WindowNotElapsed as u32),
        ),
    );
}

#[test]
fn wrong_phase_fails() {
    // Leave the oracle in FactProposal (seed default), never enter Challenge.
    let mut ctx = TestCtx::new();
    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: 1_000,
        },
        ProposerSpec {
            option: 1,
            bond: 1_000,
        },
    ]);
    let pdas: Vec<Pubkey> = ctx.proposers(oracle).iter().map(|p| p.pda).collect();
    ctx.warp(WINDOW + 1);

    let err = ctx
        .send(finalize_oracle_ix(&ctx, oracle, &pdas), &[])
        .unwrap_err()
        .err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::WrongPhase as u32),
        ),
    );
}

#[test]
fn subset_of_proposers_fails() {
    // Three proposers seeded, but only two passed -> tail.len() != proposer_count.
    let (mut ctx, oracle, pdas) = seed_challenge(
        &[
            ProposerSpec {
                option: 1,
                bond: 1_000,
            },
            ProposerSpec {
                option: 1,
                bond: 1_000,
            },
            ProposerSpec {
                option: 2,
                bond: 1_000,
            },
        ],
        &[1, 1, 2],
    );
    ctx.warp(WINDOW + 1);

    let err = ctx
        .send(finalize_oracle_ix(&ctx, oracle, &pdas[..2]), &[])
        .unwrap_err()
        .err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::InvalidAccount as u32),
        ),
    );
}

#[test]
fn surviving_count_mismatch_fails() {
    // Full set passed, but surviving_count is inconsistent with the (none
    // disqualified) proposer set.
    let (mut ctx, oracle, pdas) = seed_challenge(
        &[
            ProposerSpec {
                option: 0,
                bond: 1_000,
            },
            ProposerSpec {
                option: 1,
                bond: 1_000,
            },
        ],
        &[0, 1],
    );
    // Claim only one survivor though both are non-disqualified.
    ctx.set_surviving_count(oracle, 1);
    ctx.warp(WINDOW + 1);

    let err = ctx
        .send(finalize_oracle_ix(&ctx, oracle, &pdas), &[])
        .unwrap_err()
        .err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::InvalidAccount as u32),
        ),
    );
}

#[test]
fn surviving_no_show_is_rejected() {
    // A non-disqualified proposer left at the no-show sentinel is an invariant
    // violation -> InvalidAccount (never counted as a vote for 0xFF).
    let (mut ctx, oracle, pdas) = seed_challenge(
        &[
            ProposerSpec {
                option: 0,
                bond: 1_000,
            },
            ProposerSpec {
                option: 1,
                bond: 1_000,
            },
        ],
        &[0, 1],
    );
    ctx.set_proposer_claim_option(pdas[1], CLAIM_OPTION_NONE);
    ctx.warp(WINDOW + 1);

    let err = ctx
        .send(finalize_oracle_ix(&ctx, oracle, &pdas), &[])
        .unwrap_err()
        .err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::InvalidAccount as u32),
        ),
    );
}

#[test]
fn second_finalize_fails() {
    let (mut ctx, oracle, pdas) = seed_challenge(
        &[
            ProposerSpec {
                option: 1,
                bond: 1_000,
            },
            ProposerSpec {
                option: 1,
                bond: 1_000,
            },
        ],
        &[1, 1],
    );
    ctx.warp(WINDOW + 1);
    ctx.send(finalize_oracle_ix(&ctx, oracle, &pdas), &[])
        .expect("first finalize should succeed");
    assert_eq!(ctx.oracle(oracle).phase, Phase::Resolved as u8);

    // Second call: phase is no longer Challenge.
    let err = ctx
        .send(finalize_oracle_ix(&ctx, oracle, &pdas), &[])
        .unwrap_err()
        .err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::WrongPhase as u32),
        ),
    );
}

#[test]
fn proposer_from_other_oracle_fails() {
    // Pass oracle A but a proposer belonging to oracle B.
    let mut ctx = TestCtx::new();
    let oracle_a = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: 1_000,
        },
        ProposerSpec {
            option: 1,
            bond: 1_000,
        },
    ]);
    let oracle_b = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: 1_000,
        },
        ProposerSpec {
            option: 1,
            bond: 1_000,
        },
    ]);
    let a0 = ctx.proposers(oracle_a)[0].pda;
    let foreign = ctx.proposers(oracle_b)[0].pda;
    ctx.set_proposer_claim_option(a0, 0);
    ctx.set_phase(oracle_a, Phase::Challenge);
    ctx.warp(WINDOW + 1);

    let err = ctx
        .send(finalize_oracle_ix(&ctx, oracle_a, &[a0, foreign]), &[])
        .unwrap_err()
        .err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::InvalidAccount as u32),
        ),
    );
}

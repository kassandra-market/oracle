//! `finalize_oracle` integration tests.
//!
//! These seed a disputed oracle directly into [`Phase::Challenge`] with chosen
//! surviving/disqualified proposers and `claim_option`s, warp past the challenge
//! window, and call `finalize_oracle`. They lock in:
//!
//! * Gating: Challenge phase, after the window, `open_challenge_count == 0`.
//! * The terminal decision: Winner → Resolved (+ `resolved_option`), Tie /
//!   NoSurvivors → InvalidDeadend.
//! * The full-set + `surviving_count` consistency guards.
//! * Idempotency: a second finalize fails (phase is terminal, no longer Challenge).
//! * No token CPI — the stake vault never moves.

mod common;
use common::*;

use kassandra_oracles_program::{
    error::KassandraError,
    state::{Phase, CLAIM_OPTION_NONE},
};
use solana_instruction::Instruction;
use solana_instruction_error::InstructionError;
use solana_pubkey::Pubkey;
use solana_transaction_error::TransactionError;

// ----- instruction builder --------------------------------------------------

/// Delegates to the shared harness builder (S3 account order: oracle, kass_mint,
/// stake_vault, token program, then the read-only proposer tail; payload =
/// oracle nonce). The proposers are READ-ONLY (finalize only reads
/// claim_option / disqualified).
fn finalize_oracle_ix(ctx: &TestCtx, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
    ctx.finalize_oracle_ix(oracle, tail)
}

// ----- fixture --------------------------------------------------------------

/// Seed a disputed oracle from `specs`, set each proposer's `claim_option` to
/// the matching entry in `claims`, force the oracle into [`Phase::Challenge`]
/// (window still open), and return `(ctx, oracle, proposer_pdas)`.
fn seed_challenge(specs: &[ProposerSpec], claims: &[u8]) -> (TestCtx, Pubkey, Vec<Pubkey>) {
    assert_eq!(specs.len(), claims.len());
    let mut ctx = TestCtx::new();
    let oracle = ctx.seed_disputed_oracle(specs);
    let pdas: Vec<Pubkey> = ctx.proposers(oracle).iter().map(|p| p.pda).collect();
    for (pda, &opt) in pdas.iter().zip(claims) {
        ctx.set_proposer_claim_option(*pda, opt);
    }
    ctx.set_phase(oracle, Phase::Challenge);
    (ctx, oracle, pdas)
}

// ----- tests ----------------------------------------------------------------

#[test]
fn clear_winner_resolves() {
    // Three survivors voting [1, 1, 2] -> option 1 wins.
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
    let vault = ctx.seeded(oracle).stake_vault;
    let vault_before = ctx.token_balance(vault);

    ctx.warp(WINDOW + 1);
    ctx.send(finalize_oracle_ix(&ctx, oracle, &pdas), &[])
        .expect("finalize should succeed");

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::Resolved as u8);
    assert_eq!(o.resolved_option, 1);
    // No token CPI: the vault is untouched.
    assert_eq!(ctx.token_balance(vault), vault_before);
    // S1: total_correct_proposer_stake == Σ bond of survivors voting option 1
    // (the two 1_000-bond proposers; the option-2 proposer is excluded).
    assert_eq!(o.total_correct_proposer_stake, 2_000);
    // reward_pool == bond_pool (0 here; S3 folds emission in).
    assert_eq!(o.reward_pool, o.bond_pool);
    assert_eq!(o.reward_pool, 0);
}

#[test]
fn tie_is_invalid_deadend() {
    // Two survivors voting [0, 1] -> tie -> dead-end.
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
    ctx.warp(WINDOW + 1);
    ctx.send(finalize_oracle_ix(&ctx, oracle, &pdas), &[])
        .expect("finalize should succeed");

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::InvalidDeadend as u8);
    assert_eq!(o.resolved_option, CLAIM_OPTION_NONE);
    // S1: a dead-end distributes no rewards — both stamps stay 0.
    assert_eq!(o.total_correct_proposer_stake, 0);
    assert_eq!(o.reward_pool, 0);
}

#[test]
fn resolved_stamps_correct_total_and_reward_pool() {
    // S1: total_correct_proposer_stake == Σ bond of SURVIVORS voting the resolved
    // option (excludes a wrong-but-survived proposer); reward_pool == bond_pool
    // (non-zero here via a prior flip-slash already in the pool).
    let (mut ctx, oracle, pdas) = seed_challenge(
        &[
            ProposerSpec {
                option: 1,
                bond: 1_000,
            },
            ProposerSpec {
                option: 1,
                bond: 3_000,
            },
            ProposerSpec {
                option: 2,
                bond: 5_000,
            },
        ],
        &[1, 1, 2],
    );
    // A prior flip-slash on the option-2 proposer puts 700 into bond_pool (it
    // stays surviving + non-disqualified, so finalize_oracle still counts it as a
    // survivor voting option 2 — excluded from the correct-proposer total).
    ctx.set_proposer_prior_slash(oracle, pdas[2], 700);

    ctx.warp(WINDOW + 1);
    ctx.send(finalize_oracle_ix(&ctx, oracle, &pdas), &[])
        .expect("finalize should succeed");

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::Resolved as u8);
    assert_eq!(o.resolved_option, 1);
    // Σ bond over survivors voting option 1: 1_000 + 3_000 = 4_000.
    assert_eq!(o.total_correct_proposer_stake, 4_000);
    // reward_pool finalized to bond_pool (the 700 prior slash).
    assert_eq!(o.bond_pool, 700);
    assert_eq!(o.reward_pool, 700);
}

#[test]
fn all_disqualified_is_invalid_deadend() {
    // Both proposers disqualified, surviving_count forced to 0 -> NoSurvivors.
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
    for pda in &pdas {
        ctx.set_proposer_disqualified(*pda);
    }
    ctx.set_surviving_count(oracle, 0);

    ctx.warp(WINDOW + 1);
    ctx.send(finalize_oracle_ix(&ctx, oracle, &pdas), &[])
        .expect("finalize should succeed");

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::InvalidDeadend as u8);
    // Dead-end: resolved_option is stamped with the loud sentinel, not 0.
    assert_eq!(o.resolved_option, CLAIM_OPTION_NONE);
}

#[test]
fn one_survivor_among_disqualified_resolves() {
    // p0 survives (votes 2); p1 disqualified. surviving_count = 1 -> Winner(2).
    let (mut ctx, oracle, pdas) = seed_challenge(
        &[
            ProposerSpec {
                option: 2,
                bond: 1_000,
            },
            ProposerSpec {
                option: 1,
                bond: 1_000,
            },
        ],
        &[2, 1],
    );
    ctx.set_proposer_disqualified(pdas[1]);
    ctx.set_surviving_count(oracle, 1);

    ctx.warp(WINDOW + 1);
    ctx.send(finalize_oracle_ix(&ctx, oracle, &pdas), &[])
        .expect("finalize should succeed");

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::Resolved as u8);
    assert_eq!(o.resolved_option, 2);
}

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

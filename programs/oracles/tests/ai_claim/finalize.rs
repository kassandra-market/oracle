use super::*;

use kassandra_oracles_program::{
    config::{FLIP_SLASH_DEN, FLIP_SLASH_NUM},
    error::KassandraError,
};
use solana_instruction_error::InstructionError;
use solana_transaction_error::TransactionError;

#[test]
fn finalize_no_show_full_slash() {
    // Proposer 0 submits a valid claim; proposer 1 is a no-show.
    let (mut ctx, oracle) = seed_ai(&[
        ProposerSpec {
            option: 0,
            bond: 1_000,
        },
        ProposerSpec {
            option: 1,
            bond: 3_000,
        },
    ]);
    submit_for(&mut ctx, oracle, 0, 0).expect("submit should succeed");
    let p0 = ctx.proposers(oracle)[0].pda;
    let p1 = ctx.proposers(oracle)[1].pda;
    let bond1 = ctx.proposers(oracle)[1].bond;

    let surviving_before = ctx.oracle(oracle).surviving_count;
    ctx.warp(WINDOW + 1);
    ctx.send(finalize_ai_claims_ix(&ctx, oracle, &[p0, p1]), &[])
        .expect("finalize should succeed");

    // No-show fully slashed + disqualified.
    let np1 = ctx.proposer(p1);
    assert_eq!(np1.slashed, 1);
    assert_eq!(np1.disqualified, 1);
    assert_eq!(np1.ai_finalized, 1);
    // slashed_amount == full bond, and equals this proposer's whole bond_pool
    // contribution (the honest submitter contributes 0).
    assert_eq!(np1.slashed_amount, bond1);

    // Honest submitter survives untouched (and contributes nothing).
    let np0 = ctx.proposer(p0);
    assert_eq!(np0.slashed, 0);
    assert_eq!(np0.disqualified, 0);
    assert_eq!(np0.ai_finalized, 1);
    assert_eq!(np0.slashed_amount, 0);

    let o = ctx.oracle(oracle);
    assert_eq!(o.surviving_count, surviving_before - 1);
    assert_eq!(o.bond_pool, bond1);
    // bond_pool delta == Σ slashed_amount.
    assert_eq!(o.bond_pool, np0.slashed_amount + np1.slashed_amount);
    // Whole set finalized -> Challenge.
    assert_eq!(o.phase, Phase::Challenge as u8);
}

#[test]
fn finalize_flipped_partial_slash_remains_surviving() {
    // Proposer 0 flips (original 0, claims 1); proposer 1 submits honestly.
    let (mut ctx, oracle) = seed_ai(&[
        ProposerSpec {
            option: 0,
            bond: 2_000,
        },
        ProposerSpec {
            option: 1,
            bond: 1_000,
        },
    ]);
    submit_for(&mut ctx, oracle, 0, 1).expect("flip submit should succeed");
    submit_for(&mut ctx, oracle, 1, 1).expect("submit should succeed");
    let p0 = ctx.proposers(oracle)[0].pda;
    let p1 = ctx.proposers(oracle)[1].pda;
    let bond0 = ctx.proposers(oracle)[0].bond;

    let surviving_before = ctx.oracle(oracle).surviving_count;
    ctx.warp(WINDOW + 1);
    ctx.send(finalize_ai_claims_ix(&ctx, oracle, &[p0, p1]), &[])
        .expect("finalize should succeed");

    // Flipped: partial slash, still surviving, NOT disqualified.
    let np0 = ctx.proposer(p0);
    assert_eq!(np0.slashed, 1);
    assert_eq!(np0.disqualified, 0);
    assert_eq!(np0.flipped, 1);
    assert_eq!(np0.ai_finalized, 1);

    let expected_slash = bond0 * FLIP_SLASH_NUM / FLIP_SLASH_DEN;
    // slashed_amount on the flipper equals bond/2 == the bond_pool delta.
    assert_eq!(np0.slashed_amount, expected_slash);
    let np1 = ctx.proposer(p1);
    assert_eq!(np1.slashed_amount, 0);
    let o = ctx.oracle(oracle);
    assert_eq!(o.bond_pool, expected_slash);
    // bond_pool delta == Σ slashed_amount.
    assert_eq!(o.bond_pool, np0.slashed_amount + np1.slashed_amount);
    // Flipper remains surviving; honest submitter too -> no decrement.
    assert_eq!(o.surviving_count, surviving_before);
    assert_eq!(o.phase, Phase::Challenge as u8);
}

#[test]
fn finalize_incremental_then_completes() {
    // Three honest submitters; finalize across two calls.
    let (mut ctx, oracle) = seed_ai(&[
        ProposerSpec {
            option: 0,
            bond: 1_000,
        },
        ProposerSpec {
            option: 1,
            bond: 1_000,
        },
        ProposerSpec {
            option: 0,
            bond: 1_000,
        },
    ]);
    submit_for(&mut ctx, oracle, 0, 0).expect("submit 0");
    submit_for(&mut ctx, oracle, 1, 1).expect("submit 1");
    submit_for(&mut ctx, oracle, 2, 0).expect("submit 2");
    let p0 = ctx.proposers(oracle)[0].pda;
    let p1 = ctx.proposers(oracle)[1].pda;
    let p2 = ctx.proposers(oracle)[2].pda;

    ctx.warp(WINDOW + 1);

    // Call 1: only p0. Phase stays AiClaim.
    ctx.send(finalize_ai_claims_ix(&ctx, oracle, &[p0]), &[])
        .expect("first chunk should succeed");
    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::AiClaim as u8);
    assert_eq!(o.ai_finalized_count, 1);

    // Call 2: p1 + p2. Now ai_finalized_count == proposer_count -> Challenge.
    ctx.send(finalize_ai_claims_ix(&ctx, oracle, &[p1, p2]), &[])
        .expect("second chunk should succeed");
    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::Challenge as u8);
    assert_eq!(o.ai_finalized_count, 3);
    // No slashing: all honest, all still surviving.
    assert_eq!(o.surviving_count, 3);
    assert_eq!(o.bond_pool, 0);
}

#[test]
fn finalize_already_processed_fails() {
    let (mut ctx, oracle) = seed_ai(&[
        ProposerSpec {
            option: 0,
            bond: 1_000,
        },
        ProposerSpec {
            option: 1,
            bond: 1_000,
        },
    ]);
    submit_for(&mut ctx, oracle, 0, 0).expect("submit 0");
    submit_for(&mut ctx, oracle, 1, 1).expect("submit 1");
    let p0 = ctx.proposers(oracle)[0].pda;
    ctx.warp(WINDOW + 1);

    // Finalize p0 once (oracle stays AiClaim: p1 still pending).
    ctx.send(finalize_ai_claims_ix(&ctx, oracle, &[p0]), &[])
        .expect("first finalize should succeed");
    assert_eq!(ctx.oracle(oracle).phase, Phase::AiClaim as u8);

    // Re-finalizing the same proposer must abort.
    let err = ctx
        .send(finalize_ai_claims_ix(&ctx, oracle, &[p0]), &[])
        .unwrap_err()
        .err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::AlreadySettled as u32),
        ),
    );
}

#[test]
fn finalize_window_still_open_fails() {
    let (mut ctx, oracle) = seed_ai(&[
        ProposerSpec {
            option: 0,
            bond: 1_000,
        },
        ProposerSpec {
            option: 1,
            bond: 1_000,
        },
    ]);
    let p0 = ctx.proposers(oracle)[0].pda;
    // No warp: window still open.
    let err = ctx
        .send(finalize_ai_claims_ix(&ctx, oracle, &[p0]), &[])
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
fn finalize_proposer_from_other_oracle_fails() {
    // Two independent oracles; pass oracle A but a proposer from oracle B.
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
    ctx.set_phase(oracle_a, Phase::AiClaim);
    let foreign = ctx.proposers(oracle_b)[0].pda;
    ctx.warp(WINDOW + 1);

    let err = ctx
        .send(finalize_ai_claims_ix(&ctx, oracle_a, &[foreign]), &[])
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
fn finalize_same_proposer_twice_in_one_call_fails() {
    let (mut ctx, oracle) = seed_ai(&[
        ProposerSpec {
            option: 0,
            bond: 1_000,
        },
        ProposerSpec {
            option: 1,
            bond: 1_000,
        },
    ]);
    let p0 = ctx.proposers(oracle)[0].pda;
    ctx.warp(WINDOW + 1);

    let err = ctx
        .send(finalize_ai_claims_ix(&ctx, oracle, &[p0, p0]), &[])
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
fn finalize_empty_tail_fails() {
    let (mut ctx, oracle) = seed_ai(&[
        ProposerSpec {
            option: 0,
            bond: 1_000,
        },
        ProposerSpec {
            option: 1,
            bond: 1_000,
        },
    ]);
    ctx.warp(WINDOW + 1);

    let err = ctx
        .send(finalize_ai_claims_ix(&ctx, oracle, &[]), &[])
        .unwrap_err()
        .err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::IncompleteFactSet as u32),
        ),
    );
}

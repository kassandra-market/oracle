use super::*;

use kassandra_oracles_program::{
    config::{THRESHOLD_DEN, THRESHOLD_NUM},
    error::KassandraError,
    state::{Phase, VOTE_APPROVE},
};
use solana_instruction_error::InstructionError;
use solana_transaction_error::TransactionError;

#[test]
fn finalize_empty_tail_fails() {
    let (mut ctx, oracle, vault) = seed();
    let _fact = submit_one(&mut ctx, oracle, vault, 1);
    advance_to_voting(&mut ctx, oracle);
    ctx.warp(WINDOW + 1);

    // No facts passed at all -> nothing to settle.
    let err = ctx
        .send(finalize_facts_ix(&ctx, oracle, &[]), &[])
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

#[test]
fn finalize_same_fact_twice_in_one_call_fails() {
    let (mut ctx, oracle, vault) = seed();
    let fact = submit_one(&mut ctx, oracle, vault, 1);
    advance_to_voting(&mut ctx, oracle);
    cast_vote(&mut ctx, oracle, vault, fact, VOTE_APPROVE, 2_500);
    ctx.warp(WINDOW + 1);

    // Same fact passed twice -> distinctness violation.
    let err = ctx
        .send(finalize_facts_ix(&ctx, oracle, &[fact, fact]), &[])
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
fn finalize_already_settled_fact_fails() {
    // Two facts so settling one leaves the oracle in FactVoting (phase only
    // advances once the whole set is settled) — that keeps the AlreadySettled
    // path reachable on a re-finalize of the same fact.
    let (mut ctx, oracle, vault) = seed();
    let fact_a = submit_one(&mut ctx, oracle, vault, 1);
    let _fact_b = submit_one(&mut ctx, oracle, vault, 2);
    advance_to_voting(&mut ctx, oracle);
    cast_vote(&mut ctx, oracle, vault, fact_a, VOTE_APPROVE, 2_500);
    ctx.warp(WINDOW + 1);

    ctx.send(finalize_facts_ix(&ctx, oracle, &[fact_a]), &[])
        .expect("first finalize should succeed");
    assert_eq!(ctx.oracle(oracle).phase, Phase::FactVoting as u8);

    // Re-finalizing the same (now settled) fact must abort.
    let err = ctx
        .send(finalize_facts_ix(&ctx, oracle, &[fact_a]), &[])
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
fn finalize_zero_dispute_bond_fails() {
    let (mut ctx, oracle, vault) = seed();
    let fact = submit_one(&mut ctx, oracle, vault, 1);
    advance_to_voting(&mut ctx, oracle);
    cast_vote(&mut ctx, oracle, vault, fact, VOTE_APPROVE, 2_500);
    ctx.warp(WINDOW + 1);

    // Corrupt the denominator: threshold is now undefined.
    ctx.set_dispute_bond_total(oracle, 0);

    let err = ctx
        .send(finalize_facts_ix(&ctx, oracle, &[fact]), &[])
        .unwrap_err()
        .err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::NoDisputeBond as u32),
        ),
    );
}

#[test]
fn finalize_wrong_phase_fails() {
    let (mut ctx, oracle, vault) = seed();
    let fact = submit_one(&mut ctx, oracle, vault, 1);
    // Still in FactProposal — never advanced.
    ctx.warp(WINDOW + 1);

    let err = ctx
        .send(finalize_facts_ix(&ctx, oracle, &[fact]), &[])
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
fn finalize_window_still_open_fails() {
    let (mut ctx, oracle, vault) = seed();
    let fact = submit_one(&mut ctx, oracle, vault, 1);
    advance_to_voting(&mut ctx, oracle);
    // Do NOT warp past the voting window.

    let err = ctx
        .send(finalize_facts_ix(&ctx, oracle, &[fact]), &[])
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

/// Sanity check that the test threshold constants match the program's.
#[test]
fn threshold_constants_match_expectation() {
    assert_eq!(THRESHOLD_NUM, 2);
    assert_eq!(THRESHOLD_DEN, 3);
}

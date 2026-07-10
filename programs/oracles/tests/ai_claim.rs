//! `submit_ai_claim` + `finalize_ai_claims` integration tests.
//!
//! These compose the real deployed instructions in LiteSVM, starting from a
//! seeded disputed oracle warped/forced into the [`Phase::AiClaim`] window.
//! They lock in:
//!
//! * AiClaim PDA seeds `[b"claim", oracle, proposer]` and the submit payload.
//! * Submit gating (phase / window / authority / option range / one-per-proposer).
//! * `claim_option` / `flipped` recording.
//! * Incremental finalize: FULL slash for no-shows, PARTIAL (1/2) for flippers,
//!   no slash for honest submitters; phase advances to Challenge only once the
//!   whole proposer set is ai-finalized; `bond_pool` is a counter (no token CPI).

mod common;
use common::*;

use kassandra_oracles_program::{
    config::{FLIP_SLASH_DEN, FLIP_SLASH_NUM},
    error::KassandraError,
    instruction::Ix,
    state::{Phase, CLAIM_OPTION_NONE},
};
use solana_instruction_error::InstructionError;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction_error::TransactionError;

// ----- instruction builders -------------------------------------------------

/// Derive the AiClaim PDA: seeds `[b"claim", oracle, proposer]`.
fn claim_pda(program_id: &Pubkey, oracle: &Pubkey, proposer: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"claim", oracle.as_ref(), proposer.as_ref()], program_id)
}

fn submit_payload(option: u8) -> Vec<u8> {
    let mut data = Vec::with_capacity(1 + 32 + 32 + 32 + 1);
    data.push(Ix::SubmitAiClaim as u8);
    data.extend_from_slice(&[0xAA; 32]); // model_id
    data.extend_from_slice(&[0xBB; 32]); // params_hash
    data.extend_from_slice(&[0xCC; 32]); // io_hash
    data.push(option);
    data
}

// ----- fixture --------------------------------------------------------------

/// Seed a disputed oracle from the given specs and force it into AiClaim with
/// the window still open (seed sets `phase_ends_at = now + WINDOW`).
fn seed_ai(specs: &[ProposerSpec]) -> (TestCtx, Pubkey) {
    let mut ctx = TestCtx::new();
    let oracle = ctx.seed_disputed_oracle(specs);
    ctx.set_phase(oracle, Phase::AiClaim);
    (ctx, oracle)
}

/// Submit a claim of `option` for the seeded proposer at `idx`, returning the
/// transaction result so callers can assert success/failure.
#[allow(clippy::result_large_err)]
fn submit_for(
    ctx: &mut TestCtx,
    oracle: Pubkey,
    idx: usize,
    option: u8,
) -> litesvm::types::TransactionResult {
    let authority = ctx.proposers(oracle)[idx].authority.insecure_clone();
    let proposer_pda = ctx.proposers(oracle)[idx].pda;
    ctx.svm.airdrop(&authority.pubkey(), 1_000_000_000).unwrap();
    let (claim, _) = claim_pda(&ctx.program_id, &oracle, &proposer_pda);
    let ix = submit_ai_claim_ix(
        ctx,
        oracle,
        proposer_pda,
        claim,
        authority.pubkey(),
        submit_payload(option),
    );
    ctx.send(ix, &[&authority])
}

// ----- submit tests ---------------------------------------------------------

#[test]
fn submit_claim_matching_original() {
    // Two proposers (options 0 and 1). Proposer 0 claims its original option 0.
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
    let proposer_pda = ctx.proposers(oracle)[0].pda;
    submit_for(&mut ctx, oracle, 0, 0).expect("submit should succeed");

    let (claim, _) = claim_pda(&ctx.program_id, &oracle, &proposer_pda);
    let c = ctx.ai_claim(claim);
    assert_eq!(c.option, 0);
    assert_eq!(c.oracle, oracle.to_bytes().into());
    assert_eq!(c.proposer, proposer_pda.to_bytes().into());
    assert_eq!(c.challenged, 0);
    assert_eq!(c.model_id, [0xAA; 32]);
    assert_eq!(c.params_hash, [0xBB; 32]);
    assert_eq!(c.io_hash, [0xCC; 32]);

    let p = ctx.proposer(proposer_pda);
    assert_eq!(p.claim_option, 0);
    assert_eq!(p.flipped, 0);
}

#[test]
fn submit_flipped_claim_marks_flipped() {
    // Proposer 0's original option is 0; it claims 1 instead -> flipped.
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
    let proposer_pda = ctx.proposers(oracle)[0].pda;
    submit_for(&mut ctx, oracle, 0, 1).expect("submit should succeed");

    let p = ctx.proposer(proposer_pda);
    assert_eq!(p.claim_option, 1);
    assert_eq!(p.flipped, 1);
}

#[test]
fn submit_wrong_authority_fails() {
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
    let proposer_pda = ctx.proposers(oracle)[0].pda;
    let (claim, _) = claim_pda(&ctx.program_id, &oracle, &proposer_pda);

    // A signer who is NOT the proposer's authority.
    let attacker = Keypair::new();
    ctx.svm.airdrop(&attacker.pubkey(), 1_000_000_000).unwrap();
    let ix = submit_ai_claim_ix(
        &ctx,
        oracle,
        proposer_pda,
        claim,
        attacker.pubkey(),
        submit_payload(0),
    );
    let err = ctx.send(ix, &[&attacker]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::Unauthorized as u32),
        ),
    );
}

#[test]
fn submit_duplicate_claim_fails() {
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
    submit_for(&mut ctx, oracle, 0, 0).expect("first submit should succeed");

    let err = submit_for(&mut ctx, oracle, 0, 0).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::DuplicateClaim as u32),
        ),
    );
}

#[test]
fn submit_wrong_phase_fails() {
    // Leave the oracle in FactProposal (seed default), never force AiClaim.
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
    let err = submit_for(&mut ctx, oracle, 0, 0).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::WrongPhase as u32),
        ),
    );
}

#[test]
fn submit_after_window_fails() {
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
    let err = submit_for(&mut ctx, oracle, 0, 0).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::WindowClosed as u32),
        ),
    );
}

#[test]
fn submit_option_out_of_range_fails() {
    // options_count == 2 (max option 1, min 2). Option 2 is out of range.
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
    let err = submit_for(&mut ctx, oracle, 0, 2).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::InvalidOption as u32),
        ),
    );
}

// ----- finalize tests -------------------------------------------------------

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

/// Sanity: a freshly seeded proposer has the no-show sentinel.
#[test]
fn seeded_proposer_is_no_show_by_default() {
    let (ctx, oracle) = seed_ai(&[
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
    assert_eq!(ctx.proposer(p0).claim_option, CLAIM_OPTION_NONE);
}

#[test]
fn submit_by_disqualified_proposer_fails() {
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
    ctx.set_proposer_disqualified(p0);

    let err = submit_for(&mut ctx, oracle, 0, 0).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::Unauthorized as u32),
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

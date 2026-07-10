use super::*;

use kassandra_oracles_program::{error::KassandraError, state::CLAIM_OPTION_NONE};
use solana_instruction_error::InstructionError;
use solana_keypair::Keypair;
use solana_transaction_error::TransactionError;

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

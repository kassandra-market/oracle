//! `vote_fact` integration tests.
//!
//! These compose the real deployed instructions in LiteSVM:
//! seed -> submit_fact -> warp -> advance_phase -> fund voter -> vote_fact.
//! They lock in:
//!
//! * FactVote PDA seeds `[b"vote", fact, voter]`.
//! * Payload `disc=1 ++ kind u8 ++ stake u64 LE`.
//! * Account order: oracle, fact, fact_vote, voter, voter-KASS, stake-vault,
//!   token-program, system-program.
//! * One vote per voter per fact; non-exclusive across facts.

mod common;
use common::*;

use kassandra_oracles_program::{
    error::KassandraError,
    state::{VOTE_APPROVE, VOTE_DUPLICATE},
};
use solana_instruction_error::InstructionError;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction_error::TransactionError;

// ----- instruction builders -------------------------------------------------

// ----- fixture --------------------------------------------------------------

struct Setup {
    ctx: TestCtx,
    oracle: Pubkey,
    vault: Pubkey,
    facts: Vec<Pubkey>,
}

/// Seed an oracle, submit `num_facts` facts (in FactProposal), and optionally
/// advance to FactVoting. Returns the fact PDAs in order.
fn setup(num_facts: usize, advance: bool) -> Setup {
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
    let vault = ctx.seeded(oracle).stake_vault;

    // A submitter that bankrolls all the fact stakes.
    let submitter = Keypair::new();
    ctx.svm.airdrop(&submitter.pubkey(), 1_000_000_000).unwrap();
    let submitter_kass = ctx.fund_kass(&submitter, 1_000_000);

    let mut facts = Vec::with_capacity(num_facts);
    for i in 0..num_facts {
        let content_hash = [i as u8 + 1; 32];
        let (fact, _) = TestCtx::fact_pda(&ctx.program_id, &oracle, &content_hash);
        let ix = submit_fact_ix(
            &ctx,
            oracle,
            fact,
            submitter.pubkey(),
            submitter_kass,
            vault,
            submit_fact_payload(&content_hash, 100, b"ipfs://fact"),
        );
        ctx.send(ix, &[&submitter])
            .expect("submit_fact should succeed");
        facts.push(fact);
    }

    if advance {
        ctx.warp(WINDOW + 1);
        let ix = advance_phase_ix(&ctx, oracle);
        ctx.send(ix, &[]).expect("advance_phase should succeed");
    }

    Setup {
        ctx,
        oracle,
        vault,
        facts,
    }
}

/// Create + fund a fresh voter (lamports for rent + KASS for stake).
fn fund_voter(ctx: &mut TestCtx, kass: u64) -> (Keypair, Pubkey) {
    let voter = Keypair::new();
    ctx.svm.airdrop(&voter.pubkey(), 1_000_000_000).unwrap();
    let voter_kass = ctx.fund_kass(&voter, kass);
    (voter, voter_kass)
}

// ----- tests ----------------------------------------------------------------

#[test]
fn vote_fact_approve_tallies_and_moves_stake() {
    let Setup {
        mut ctx,
        oracle,
        vault,
        facts,
    } = setup(1, true);
    let fact = facts[0];

    let (voter, voter_kass) = fund_voter(&mut ctx, 1_000);
    let (fact_vote, _) = TestCtx::vote_pda(&ctx.program_id, &fact, &voter.pubkey());

    let vault_before = ctx.token_balance(vault);
    let total_before = ctx.oracle(oracle).total_oracle_stake;

    let stake = 500u64;
    let ix = vote_fact_ix(
        &ctx,
        oracle,
        fact,
        fact_vote,
        voter.pubkey(),
        voter_kass,
        vault,
        vote_payload(VOTE_APPROVE, stake),
    );
    ctx.send(ix, &[&voter])
        .expect("approve vote should succeed");

    let f = ctx.fact(fact);
    assert_eq!(f.approve_stake, stake);
    assert_eq!(f.duplicate_stake, 0);

    assert_eq!(ctx.token_balance(vault), vault_before + stake);
    assert_eq!(ctx.oracle(oracle).total_oracle_stake, total_before + stake);

    let v = ctx.fact_vote(fact_vote);
    assert_eq!(v.kind, VOTE_APPROVE);
    assert_eq!(v.stake, stake);
    assert_eq!(v.fact, fact.to_bytes().into());
    assert_eq!(v.voter, voter.pubkey().to_bytes().into());
}

#[test]
fn vote_fact_duplicate_tallies_duplicate_stake() {
    let Setup {
        mut ctx,
        oracle,
        vault,
        facts,
    } = setup(1, true);
    let fact = facts[0];

    let (voter, voter_kass) = fund_voter(&mut ctx, 1_000);
    let (fact_vote, _) = TestCtx::vote_pda(&ctx.program_id, &fact, &voter.pubkey());

    let stake = 700u64;
    let ix = vote_fact_ix(
        &ctx,
        oracle,
        fact,
        fact_vote,
        voter.pubkey(),
        voter_kass,
        vault,
        vote_payload(VOTE_DUPLICATE, stake),
    );
    ctx.send(ix, &[&voter])
        .expect("duplicate vote should succeed");

    let f = ctx.fact(fact);
    assert_eq!(f.duplicate_stake, stake);
    assert_eq!(f.approve_stake, 0);

    let v = ctx.fact_vote(fact_vote);
    assert_eq!(v.kind, VOTE_DUPLICATE);
}

#[test]
fn vote_fact_double_vote_same_fact_fails() {
    let Setup {
        mut ctx,
        oracle,
        vault,
        facts,
    } = setup(1, true);
    let fact = facts[0];

    let (voter, voter_kass) = fund_voter(&mut ctx, 1_000);
    let (fact_vote, _) = TestCtx::vote_pda(&ctx.program_id, &fact, &voter.pubkey());

    let ix1 = vote_fact_ix(
        &ctx,
        oracle,
        fact,
        fact_vote,
        voter.pubkey(),
        voter_kass,
        vault,
        vote_payload(VOTE_APPROVE, 100),
    );
    ctx.send(ix1, &[&voter]).expect("first vote should succeed");

    let ix2 = vote_fact_ix(
        &ctx,
        oracle,
        fact,
        fact_vote,
        voter.pubkey(),
        voter_kass,
        vault,
        vote_payload(VOTE_APPROVE, 100),
    );
    let err = ctx.send(ix2, &[&voter]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::DuplicateVote as u32),
        ),
    );
}

#[test]
fn vote_fact_non_exclusive_across_facts() {
    let Setup {
        mut ctx,
        oracle,
        vault,
        facts,
    } = setup(2, true);
    let fact_a = facts[0];
    let fact_b = facts[1];

    // One voter, votes the full stake on BOTH facts.
    let (voter, voter_kass) = fund_voter(&mut ctx, 10_000);
    let stake = 400u64;

    for fact in [fact_a, fact_b] {
        let (fact_vote, _) = TestCtx::vote_pda(&ctx.program_id, &fact, &voter.pubkey());
        let ix = vote_fact_ix(
            &ctx,
            oracle,
            fact,
            fact_vote,
            voter.pubkey(),
            voter_kass,
            vault,
            vote_payload(VOTE_APPROVE, stake),
        );
        ctx.send(ix, &[&voter]).expect("vote should succeed");
    }

    // Full stake counts on each fact (not split).
    assert_eq!(ctx.fact(fact_a).approve_stake, stake);
    assert_eq!(ctx.fact(fact_b).approve_stake, stake);
}

#[test]
fn vote_fact_wrong_phase_fails() {
    // No advance: oracle is still in FactProposal.
    let Setup {
        mut ctx,
        oracle,
        vault,
        facts,
    } = setup(1, false);
    let fact = facts[0];

    let (voter, voter_kass) = fund_voter(&mut ctx, 1_000);
    let (fact_vote, _) = TestCtx::vote_pda(&ctx.program_id, &fact, &voter.pubkey());

    let ix = vote_fact_ix(
        &ctx,
        oracle,
        fact,
        fact_vote,
        voter.pubkey(),
        voter_kass,
        vault,
        vote_payload(VOTE_APPROVE, 100),
    );
    let err = ctx.send(ix, &[&voter]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::WrongPhase as u32),
        ),
    );
}

#[test]
fn vote_fact_zero_stake_ok_when_floor_zero() {
    // Bootstrapping: a 0-stake vote is accepted while the oracle's floor is 0.
    let Setup {
        mut ctx,
        oracle,
        vault,
        facts,
    } = setup(1, true);
    let fact = facts[0];
    assert_eq!(
        ctx.oracle(oracle).min_stake,
        0,
        "genesis oracle floor must be 0"
    );

    let (voter, voter_kass) = fund_voter(&mut ctx, 1_000);
    let (fact_vote, _) = TestCtx::vote_pda(&ctx.program_id, &fact, &voter.pubkey());

    let ix = vote_fact_ix(
        &ctx,
        oracle,
        fact,
        fact_vote,
        voter.pubkey(),
        voter_kass,
        vault,
        vote_payload(VOTE_APPROVE, 0),
    );
    assert!(
        ctx.send(ix, &[&voter]).is_ok(),
        "0-stake vote must succeed when floor is 0"
    );
}

#[test]
fn vote_fact_below_floor_fails() {
    // Once activity raises the floor, a vote stake below it is rejected.
    let Setup {
        mut ctx,
        oracle,
        vault,
        facts,
    } = setup(1, true);
    let fact = facts[0];
    ctx.set_oracle_min_stake(oracle, 1_000);

    let (voter, voter_kass) = fund_voter(&mut ctx, 1_000);
    let (fact_vote, _) = TestCtx::vote_pda(&ctx.program_id, &fact, &voter.pubkey());

    let ix = vote_fact_ix(
        &ctx,
        oracle,
        fact,
        fact_vote,
        voter.pubkey(),
        voter_kass,
        vault,
        vote_payload(VOTE_APPROVE, 999),
    );
    let err = ctx.send(ix, &[&voter]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::BelowMinStake as u32),
        ),
    );
}

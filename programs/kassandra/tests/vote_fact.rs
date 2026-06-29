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

use kassandra_program::{
    error::KassandraError,
    instruction::Ix,
    state::{VOTE_APPROVE, VOTE_DUPLICATE},
};
use solana_sdk::{
    instruction::{AccountMeta, Instruction, InstructionError},
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    system_program,
    transaction::TransactionError,
};
use spl_token::ID as TOKEN_PROGRAM_ID;

// ----- instruction builders -------------------------------------------------

fn submit_fact_payload(content_hash: &[u8; 32], stake: u64, uri: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(1 + 32 + 8 + 2 + uri.len());
    data.push(Ix::SubmitFact as u8);
    data.extend_from_slice(content_hash);
    data.extend_from_slice(&stake.to_le_bytes());
    data.extend_from_slice(&(uri.len() as u16).to_le_bytes());
    data.extend_from_slice(uri);
    data
}

#[allow(clippy::too_many_arguments)]
fn submit_fact_ix(
    ctx: &TestCtx,
    oracle: Pubkey,
    fact: Pubkey,
    submitter: Pubkey,
    submitter_kass: Pubkey,
    vault: Pubkey,
    data: Vec<u8>,
) -> Instruction {
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(fact, false),
            AccountMeta::new(submitter, true),
            AccountMeta::new(submitter_kass, false),
            AccountMeta::new(vault, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data,
    }
}

fn advance_phase_ix(ctx: &TestCtx, oracle: Pubkey) -> Instruction {
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![AccountMeta::new(oracle, false)],
        data: vec![Ix::AdvancePhase as u8],
    }
}

fn vote_payload(kind: u8, stake: u64) -> Vec<u8> {
    let mut data = Vec::with_capacity(1 + 1 + 8);
    data.push(Ix::VoteFact as u8);
    data.push(kind);
    data.extend_from_slice(&stake.to_le_bytes());
    data
}

#[allow(clippy::too_many_arguments)]
fn vote_fact_ix(
    ctx: &TestCtx,
    oracle: Pubkey,
    fact: Pubkey,
    fact_vote: Pubkey,
    voter: Pubkey,
    voter_kass: Pubkey,
    vault: Pubkey,
    data: Vec<u8>,
) -> Instruction {
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(fact, false),
            AccountMeta::new(fact_vote, false),
            AccountMeta::new(voter, true),
            AccountMeta::new(voter_kass, false),
            AccountMeta::new(vault, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data,
    }
}

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
    assert_eq!(v.fact, fact.to_bytes());
    assert_eq!(v.voter, voter.pubkey().to_bytes());
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
fn vote_fact_zero_stake_fails() {
    let Setup {
        mut ctx,
        oracle,
        vault,
        facts,
    } = setup(1, true);
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
        vote_payload(VOTE_APPROVE, 0),
    );
    let err = ctx.send(ix, &[&voter]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::ZeroStake as u32),
        ),
    );
}

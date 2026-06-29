//! `finalize_facts` integration tests.
//!
//! These compose the real deployed instructions in LiteSVM:
//! seed -> submit_fact(s) -> warp -> advance_phase -> fund voters -> vote_fact
//! -> warp -> finalize_facts. They lock in:
//!
//! * Gating: FactVoting phase, after the voting window has elapsed.
//! * The resolved fact-quorum rule (agreed / duplicate-dominant / rejected)
//!   using the fixed `dispute_bond_total` denominator and 2/3 supermajority.
//! * `bond_pool` is a counter only — no token CPI, the vault never moves.
//! * The no-facts dead-end slashes every proposer.
//! * Distinctness + exact-count enforcement (no partial finalization).

mod common;
use common::*;

use kassandra_program::{
    config::{THRESHOLD_DEN, THRESHOLD_NUM},
    error::KassandraError,
    instruction::Ix,
    state::{Phase, VOTE_APPROVE, VOTE_DUPLICATE},
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

/// Build a `finalize_facts` instruction: oracle (writable) + a tail of the
/// given accounts (all writable). No signer is required.
fn finalize_facts_ix(ctx: &TestCtx, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
    let mut accounts = Vec::with_capacity(1 + tail.len());
    accounts.push(AccountMeta::new(oracle, false));
    for k in tail {
        accounts.push(AccountMeta::new(*k, false));
    }
    Instruction {
        program_id: ctx.program_id,
        accounts,
        data: vec![Ix::FinalizeFacts as u8],
    }
}

// ----- fixture --------------------------------------------------------------

/// Seed a two-proposer oracle with bonds [1_000, 2_000], so
/// `dispute_bond_total == 3_000` and the agreed threshold (2/3) is
/// `approve_stake >= 2_000`.
fn seed() -> (TestCtx, Pubkey, Pubkey) {
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
    (ctx, oracle, vault)
}

/// Submit one fact (stake 100) and return its PDA. Oracle must be in
/// FactProposal.
fn submit_one(ctx: &mut TestCtx, oracle: Pubkey, vault: Pubkey, tag: u8) -> Pubkey {
    let submitter = Keypair::new();
    ctx.svm.airdrop(&submitter.pubkey(), 1_000_000_000).unwrap();
    let submitter_kass = ctx.fund_kass(&submitter, 1_000_000);
    let content_hash = [tag; 32];
    let (fact, _) = TestCtx::fact_pda(&ctx.program_id, &oracle, &content_hash);
    let ix = submit_fact_ix(
        ctx,
        oracle,
        fact,
        submitter.pubkey(),
        submitter_kass,
        vault,
        submit_fact_payload(&content_hash, 100, b"ipfs://fact"),
    );
    ctx.send(ix, &[&submitter]).expect("submit_fact should succeed");
    fact
}

/// Advance an oracle from FactProposal into FactVoting (warps past the proposal
/// window, then ticks).
fn advance_to_voting(ctx: &mut TestCtx, oracle: Pubkey) {
    ctx.warp(WINDOW + 1);
    let ix = advance_phase_ix(ctx, oracle);
    ctx.send(ix, &[]).expect("advance_phase should succeed");
}

/// Cast a vote of `kind`/`stake` on `fact` from a fresh, funded voter.
fn cast_vote(ctx: &mut TestCtx, oracle: Pubkey, vault: Pubkey, fact: Pubkey, kind: u8, stake: u64) {
    let voter = Keypair::new();
    ctx.svm.airdrop(&voter.pubkey(), 1_000_000_000).unwrap();
    let voter_kass = ctx.fund_kass(&voter, stake);
    let (fact_vote, _) = TestCtx::vote_pda(&ctx.program_id, &fact, &voter.pubkey());
    let ix = vote_fact_ix(
        ctx,
        oracle,
        fact,
        fact_vote,
        voter.pubkey(),
        voter_kass,
        vault,
        vote_payload(kind, stake),
    );
    ctx.send(ix, &[&voter]).expect("vote_fact should succeed");
}

// ----- tests ----------------------------------------------------------------

#[test]
fn finalize_agreed_fact() {
    let (mut ctx, oracle, vault) = seed();
    let fact = submit_one(&mut ctx, oracle, vault, 1);
    advance_to_voting(&mut ctx, oracle);

    // 2_500 >= 2_000 threshold and beats duplicate (0).
    cast_vote(&mut ctx, oracle, vault, fact, VOTE_APPROVE, 2_500);

    let bond_pool_before = ctx.oracle(oracle).bond_pool;
    ctx.warp(WINDOW + 1);
    ctx.send(finalize_facts_ix(&ctx, oracle, &[fact]), &[])
        .expect("finalize_facts should succeed");

    let f = ctx.fact(fact);
    assert_eq!(f.agreed, 1);
    assert_eq!(f.duplicate, 0);
    assert_eq!(f.settled, 1);

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::AiClaim as u8);
    assert_eq!(o.bond_pool, bond_pool_before, "agreed fact must not slash");
}

#[test]
fn finalize_rejected_fact_slashes_into_bond_pool() {
    let (mut ctx, oracle, vault) = seed();
    let fact = submit_one(&mut ctx, oracle, vault, 1);
    advance_to_voting(&mut ctx, oracle);

    // 500 < 2_000 threshold, and not duplicate-dominant (duplicate == 0).
    cast_vote(&mut ctx, oracle, vault, fact, VOTE_APPROVE, 500);

    let stake = ctx.fact(fact).stake;
    let bond_pool_before = ctx.oracle(oracle).bond_pool;
    let vault_before = ctx.token_balance(vault);

    ctx.warp(WINDOW + 1);
    ctx.send(finalize_facts_ix(&ctx, oracle, &[fact]), &[])
        .expect("finalize_facts should succeed");

    let f = ctx.fact(fact);
    assert_eq!(f.agreed, 0);
    assert_eq!(f.duplicate, 0);
    assert_eq!(f.settled, 1);

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::AiClaim as u8);
    assert_eq!(o.bond_pool, bond_pool_before + stake);
    // No token CPI: the vault is untouched (bond_pool is a counter only).
    assert_eq!(ctx.token_balance(vault), vault_before);
}

#[test]
fn finalize_duplicate_dominant_fact_not_slashed() {
    let (mut ctx, oracle, vault) = seed();
    let fact = submit_one(&mut ctx, oracle, vault, 1);
    advance_to_voting(&mut ctx, oracle);

    // duplicate (600) > approve (100) -> duplicate-dominant.
    cast_vote(&mut ctx, oracle, vault, fact, VOTE_APPROVE, 100);
    cast_vote(&mut ctx, oracle, vault, fact, VOTE_DUPLICATE, 600);

    let bond_pool_before = ctx.oracle(oracle).bond_pool;
    ctx.warp(WINDOW + 1);
    ctx.send(finalize_facts_ix(&ctx, oracle, &[fact]), &[])
        .expect("finalize_facts should succeed");

    let f = ctx.fact(fact);
    assert_eq!(f.duplicate, 1);
    assert_eq!(f.agreed, 0);
    assert_eq!(f.settled, 1);

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::AiClaim as u8);
    assert_eq!(
        o.bond_pool, bond_pool_before,
        "duplicate-dominant fact must not slash"
    );
}

#[test]
fn finalize_no_facts_deadend_slashes_all_proposers() {
    let (mut ctx, oracle, _vault) = seed();
    // No facts submitted; advance straight to FactVoting.
    advance_to_voting(&mut ctx, oracle);

    let proposer_pdas: Vec<Pubkey> = ctx.proposers(oracle).iter().map(|p| p.pda).collect();
    let total_bonds: u64 = ctx.proposers(oracle).iter().map(|p| p.bond).sum();

    ctx.warp(WINDOW + 1);
    ctx.send(finalize_facts_ix(&ctx, oracle, &proposer_pdas), &[])
        .expect("finalize_facts (no-facts) should succeed");

    for pda in &proposer_pdas {
        let p = ctx.proposer(*pda);
        assert_eq!(p.slashed, 1);
        assert_eq!(p.disqualified, 1);
    }

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::InvalidDeadend as u8);
    assert_eq!(o.surviving_count, 0);
    assert_eq!(o.bond_pool, total_bonds);
}

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
fn finalize_subset_keeps_phase_then_completes() {
    // 3 facts: A agreed, B rejected, C duplicate-dominant. Finalize in two
    // calls ([A, B] then [C]); phase stays FactVoting until all are settled.
    let (mut ctx, oracle, vault) = seed();
    let fact_a = submit_one(&mut ctx, oracle, vault, 1);
    let fact_b = submit_one(&mut ctx, oracle, vault, 2);
    let fact_c = submit_one(&mut ctx, oracle, vault, 3);
    advance_to_voting(&mut ctx, oracle);

    cast_vote(&mut ctx, oracle, vault, fact_a, VOTE_APPROVE, 2_500); // agreed
    cast_vote(&mut ctx, oracle, vault, fact_b, VOTE_APPROVE, 500); // rejected
    cast_vote(&mut ctx, oracle, vault, fact_c, VOTE_APPROVE, 100); // duplicate-dominant
    cast_vote(&mut ctx, oracle, vault, fact_c, VOTE_DUPLICATE, 600);

    ctx.warp(WINDOW + 1);

    // Call 1: settle A and B. Phase must stay FactVoting (C still pending).
    ctx.send(finalize_facts_ix(&ctx, oracle, &[fact_a, fact_b]), &[])
        .expect("first chunk should succeed");

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::FactVoting as u8, "phase must not advance yet");
    assert_eq!(o.settled_count, 2);
    assert_eq!(ctx.fact(fact_a).agreed, 1);
    assert_eq!(ctx.fact(fact_b).settled, 1);
    assert_eq!(ctx.fact(fact_b).agreed, 0);
    assert_eq!(ctx.fact(fact_c).settled, 0);

    // Call 2: settle C. Now settled_count == fact_count -> AiClaim.
    ctx.send(finalize_facts_ix(&ctx, oracle, &[fact_c]), &[])
        .expect("second chunk should succeed");

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::AiClaim as u8);
    assert_eq!(o.settled_count, 3);
    assert_eq!(ctx.fact(fact_c).duplicate, 1);
    // bond_pool == sum of rejected facts' stakes (only B, stake 100).
    assert_eq!(o.bond_pool, ctx.fact(fact_b).stake);
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
fn finalize_equal_stakes_is_rejected() {
    // approve == duplicate: NOT duplicate-dominant (needs strictly greater)
    // and NOT agreed (needs approve > duplicate) -> rejected + slashed.
    let (mut ctx, oracle, vault) = seed();
    let fact = submit_one(&mut ctx, oracle, vault, 1);
    advance_to_voting(&mut ctx, oracle);

    cast_vote(&mut ctx, oracle, vault, fact, VOTE_APPROVE, 500);
    cast_vote(&mut ctx, oracle, vault, fact, VOTE_DUPLICATE, 500);

    let stake = ctx.fact(fact).stake;
    ctx.warp(WINDOW + 1);
    ctx.send(finalize_facts_ix(&ctx, oracle, &[fact]), &[])
        .expect("finalize_facts should succeed");

    let f = ctx.fact(fact);
    assert_eq!(f.agreed, 0);
    assert_eq!(f.duplicate, 0);
    assert_eq!(f.settled, 1);
    assert_eq!(ctx.oracle(oracle).bond_pool, stake);
}

#[test]
fn finalize_exact_threshold_is_agreed() {
    // dispute_bond_total == 3_000, THRESHOLD 2/3 -> boundary approve == 2_000:
    // 2_000 * 3 == 3_000 * 2, and approve (2_000) > duplicate (0) -> agreed.
    let (mut ctx, oracle, vault) = seed();
    let fact = submit_one(&mut ctx, oracle, vault, 1);
    advance_to_voting(&mut ctx, oracle);

    cast_vote(&mut ctx, oracle, vault, fact, VOTE_APPROVE, 2_000);

    ctx.warp(WINDOW + 1);
    ctx.send(finalize_facts_ix(&ctx, oracle, &[fact]), &[])
        .expect("finalize_facts should succeed");

    let f = ctx.fact(fact);
    assert_eq!(f.agreed, 1, "exact 2/3 boundary must pass (>=)");
    assert_eq!(f.settled, 1);
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

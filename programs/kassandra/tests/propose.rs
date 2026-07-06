//! `propose` integration tests (Task H3): proposer registration with a KASS
//! bond, the deadline gate, the proposal-window logic (normal / empty-window
//! seeding / closed-with-proposers), and the on-chain `MAX_PROPOSERS` cap.
//!
//! These drive the real deployed program in LiteSVM against an oracle created
//! via `create_oracle` (so it sits in `Phase::Proposal` with a future deadline).

mod common;
use common::*;

use kassandra_program::{
    config::{MAX_PROPOSERS, PROPOSAL_WINDOW},
    error::KassandraError,
    state::CLAIM_OPTION_NONE,
};
use solana_keypair::Keypair;
use solana_signer::Signer;

/// Seconds between an oracle's creation `now` and its `deadline`; tests warp
/// past it to open the proposal window.
const DEADLINE_DELTA: i64 = 1_000;

/// Decode a LiteSVM transaction error into its `Custom(u32)` code, if any.
fn custom_code(res: &litesvm::types::TransactionResult) -> Option<u32> {
    use solana_instruction_error::InstructionError;
    use solana_transaction_error::TransactionError;
    match res {
        Err(meta) => match &meta.err {
            TransactionError::InstructionError(_, InstructionError::Custom(code)) => Some(*code),
            _ => None,
        },
        Ok(_) => None,
    }
}

/// Init the protocol and create an oracle in `Phase::Proposal` with the given
/// `options_count` and a `deadline = now + DEADLINE_DELTA`. Returns the Oracle
/// PDA. Does NOT warp past the deadline (the caller decides).
fn setup(ctx: &mut TestCtx, nonce: u64, options_count: u8) -> solana_pubkey::Pubkey {
    let (_p, res) = ctx.init_protocol();
    assert!(res.is_ok(), "init_protocol should succeed: {res:?}");
    let deadline = ctx.now() + DEADLINE_DELTA;
    let (oracle, res) = ctx.create_oracle(nonce, options_count, deadline, 600);
    assert!(res.is_ok(), "create_oracle should succeed: {res:?}");
    oracle
}

#[test]
fn propose_before_deadline_fails() {
    let mut ctx = TestCtx::new();
    let oracle = setup(&mut ctx, 1, 3);

    // No warp: now < deadline.
    let authority = Keypair::new();
    let (_pda, res) = ctx.propose(oracle, &authority, 1, 5_000);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::DeadlineNotReached as u32),
        "propose before deadline must fail DeadlineNotReached: {res:?}"
    );
}

#[test]
fn propose_happy_path() {
    let mut ctx = TestCtx::new();
    let oracle = setup(&mut ctx, 1, 3);
    ctx.warp(DEADLINE_DELTA); // now >= deadline, inside the window

    let (vault, _) = TestCtx::stake_vault_pda(&ctx.program_id, &oracle);
    let vault_before = ctx.token_balance(vault);

    let authority = Keypair::new();
    let bond = 5_000u64;
    let option = 2u8;
    let (proposer_pda, res) = ctx.propose(oracle, &authority, option, bond);
    assert!(res.is_ok(), "propose should succeed: {res:?}");

    let p = ctx.proposer(proposer_pda);
    assert_eq!(p.oracle, oracle.to_bytes().into());
    assert_eq!(p.authority, authority.pubkey().to_bytes().into());
    assert_eq!(p.bond, bond);
    assert_eq!(p.original_option, option);
    assert_eq!(
        p.claim_option, CLAIM_OPTION_NONE,
        "claim_option must be the not-yet-claimed sentinel"
    );
    assert_eq!(p.disqualified, 0);
    assert_eq!(p.slashed, 0);
    assert_eq!(p.flipped, 0);
    assert_eq!(p.ai_finalized, 0);
    assert_eq!(p.slashed_amount, 0);

    // Bond escrowed; oracle bookkeeping bumped.
    assert_eq!(ctx.token_balance(vault), vault_before + bond);
    let o = ctx.oracle(oracle);
    assert_eq!(o.proposer_count, 1);
    assert_eq!(o.surviving_count, 1);
    assert_eq!(o.total_oracle_stake, bond);
}

#[test]
fn propose_duplicate_authority_fails() {
    let mut ctx = TestCtx::new();
    let oracle = setup(&mut ctx, 1, 3);
    ctx.warp(DEADLINE_DELTA);

    let authority = Keypair::new();
    let (_pda, res) = ctx.propose(oracle, &authority, 1, 5_000);
    assert!(res.is_ok(), "first propose should succeed: {res:?}");

    let (_pda2, res2) = ctx.propose(oracle, &authority, 1, 5_000);
    assert_eq!(
        custom_code(&res2),
        Some(KassandraError::DuplicateProposer as u32),
        "second propose by same authority must fail DuplicateProposer: {res2:?}"
    );
}

#[test]
fn propose_option_out_of_range_fails() {
    let mut ctx = TestCtx::new();
    let oracle = setup(&mut ctx, 1, 3); // options 0..=2
    ctx.warp(DEADLINE_DELTA);

    let authority = Keypair::new();
    let (_pda, res) = ctx.propose(oracle, &authority, 3, 5_000);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidOptionsCount as u32),
        "option >= options_count must fail InvalidOptionsCount: {res:?}"
    );
}

#[test]
fn propose_zero_bond_fails() {
    let mut ctx = TestCtx::new();
    let oracle = setup(&mut ctx, 1, 3);
    ctx.warp(DEADLINE_DELTA);

    let authority = Keypair::new();
    let (_pda, res) = ctx.propose(oracle, &authority, 1, 0);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::ZeroStake as u32),
        "bond == 0 must fail ZeroStake: {res:?}"
    );
}

#[test]
fn propose_max_proposers_cap_enforced() {
    let mut ctx = TestCtx::new();
    // options_count 2 keeps every proposal in range; the cap is what we test.
    let oracle = setup(&mut ctx, 1, 2);
    ctx.warp(DEADLINE_DELTA);

    // Register exactly MAX_PROPOSERS distinct authorities; all succeed.
    for i in 0..MAX_PROPOSERS {
        let authority = Keypair::new();
        let (_pda, res) = ctx.propose(oracle, &authority, (i % 2) as u8, 1_000);
        assert!(res.is_ok(), "proposer {i} should register: {res:?}");
    }
    assert_eq!(ctx.oracle(oracle).proposer_count, MAX_PROPOSERS);

    // The next distinct authority is over the cap.
    let overflow = Keypair::new();
    let (_pda, res) = ctx.propose(oracle, &overflow, 0, 1_000);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::TooManyProposers as u32),
        "registration beyond MAX_PROPOSERS must fail TooManyProposers: {res:?}"
    );
    // The cap did not brick the oracle; count is unchanged.
    assert_eq!(ctx.oracle(oracle).proposer_count, MAX_PROPOSERS);
}

#[test]
fn propose_empty_window_seeds_and_extends() {
    let mut ctx = TestCtx::new();
    let oracle = setup(&mut ctx, 1, 3);

    // Warp PAST phase_ends_at (= deadline + PROPOSAL_WINDOW) with 0 proposers.
    ctx.warp(DEADLINE_DELTA + PROPOSAL_WINDOW + 10);
    let now_at_propose = ctx.now();
    assert!(now_at_propose >= ctx.oracle(oracle).phase_ends_at);

    let authority = Keypair::new();
    let (proposer_pda, res) = ctx.propose(oracle, &authority, 1, 5_000);
    assert!(
        res.is_ok(),
        "first proposal after an empty window must be accepted (seeding): {res:?}"
    );
    let _ = proposer_pda;

    let o = ctx.oracle(oracle);
    assert_eq!(o.proposer_count, 1);
    // Window re-opened: phase_ends_at extended to ~now + PROPOSAL_WINDOW.
    assert_eq!(
        o.phase_ends_at,
        now_at_propose + PROPOSAL_WINDOW,
        "empty-window seeding must extend phase_ends_at to now + PROPOSAL_WINDOW"
    );
}

#[test]
fn propose_window_closed_with_proposers_fails() {
    let mut ctx = TestCtx::new();
    let oracle = setup(&mut ctx, 1, 3);
    ctx.warp(DEADLINE_DELTA); // open the window

    // One proposal lands inside the window (proposer_count -> 1).
    let first = Keypair::new();
    let (_pda, res) = ctx.propose(oracle, &first, 0, 5_000);
    assert!(res.is_ok(), "first propose should succeed: {res:?}");

    // Now cross phase_ends_at with proposers already present.
    ctx.warp(PROPOSAL_WINDOW + 10);

    let late = Keypair::new();
    let (_pda, res) = ctx.propose(oracle, &late, 1, 5_000);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::ProposalWindowClosed as u32),
        "propose after a closed window with proposers must fail ProposalWindowClosed: {res:?}"
    );
}

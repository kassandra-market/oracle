//! `finalize_proposals` integration tests (Task H4): at the proposal-window end,
//! resolve the oracle if every proposer agrees, else hand off to the dispute core
//! by opening `Phase::FactProposal` with `dispute_bond_total` set.
//!
//! These drive the real deployed program in LiteSVM against an oracle created via
//! `create_oracle` and populated via real `propose` calls.

mod common;
use common::*;

use kassandra_program::{
    config::{PHASE_WINDOW, PROPOSAL_WINDOW},
    error::KassandraError,
    state::Phase,
};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;

/// Seconds between an oracle's creation `now` and its `deadline`.
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

/// Init protocol + create an oracle in `Phase::Proposal`, then warp past the
/// deadline so the proposal window is open. Returns the Oracle PDA.
fn setup_open(ctx: &mut TestCtx, nonce: u64, options_count: u8) -> Pubkey {
    let (_p, res) = ctx.init_protocol();
    assert!(res.is_ok(), "init_protocol should succeed: {res:?}");
    let deadline = ctx.now() + DEADLINE_DELTA;
    let (oracle, res) = ctx.create_oracle(nonce, options_count, deadline, 600);
    assert!(res.is_ok(), "create_oracle should succeed: {res:?}");
    ctx.warp(DEADLINE_DELTA); // now >= deadline, inside the proposal window
    oracle
}

/// Register `options.len()` distinct proposers (one per option) against `oracle`,
/// each with the given `bond`. Returns the proposer PDAs in order.
fn propose_each(ctx: &mut TestCtx, oracle: Pubkey, options: &[u8], bond: u64) -> Vec<Pubkey> {
    let mut pdas = Vec::new();
    for &opt in options {
        let authority = Keypair::new();
        let (pda, res) = ctx.propose(oracle, &authority, opt, bond);
        assert!(res.is_ok(), "propose(option={opt}) should succeed: {res:?}");
        pdas.push(pda);
    }
    pdas
}

#[test]
fn finalize_all_agree_resolves() {
    let mut ctx = TestCtx::new();
    let oracle = setup_open(&mut ctx, 1, 3);
    let bond = 5_000u64;
    let proposers = propose_each(&mut ctx, oracle, &[2, 2, 2], bond);

    let (vault, _) = TestCtx::stake_vault_pda(&ctx.program_id, &oracle);
    let vault_before = ctx.token_balance(vault);

    // Cross the proposal window.
    ctx.warp(PROPOSAL_WINDOW + 10);

    let ix = ctx.finalize_proposals_ix(oracle, &proposers);
    let res = ctx.send(ix, &[]);
    assert!(res.is_ok(), "finalize_proposals should succeed: {res:?}");

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::Resolved.as_u8(), "all-agree => Resolved");
    assert_eq!(o.resolved_option, 2, "resolved_option == the agreed option");
    // No token CPI: the vault is untouched and dispute_bond_total stays 0.
    assert_eq!(ctx.token_balance(vault), vault_before, "vault untouched");
    assert_eq!(o.dispute_bond_total, 0, "no dispute opened => bond_total 0");
}

#[test]
fn finalize_conflict_opens_dispute() {
    let mut ctx = TestCtx::new();
    let oracle = setup_open(&mut ctx, 1, 3);
    let bond = 5_000u64;
    // Two distinct options => conflict.
    let proposers = propose_each(&mut ctx, oracle, &[0, 1], bond);

    let total_stake = ctx.oracle(oracle).total_oracle_stake;
    assert_eq!(total_stake, 2 * bond);

    ctx.warp(PROPOSAL_WINDOW + 10);
    let now_at_finalize = ctx.now();

    let ix = ctx.finalize_proposals_ix(oracle, &proposers);
    let res = ctx.send(ix, &[]);
    assert!(res.is_ok(), "finalize_proposals should succeed: {res:?}");

    let o = ctx.oracle(oracle);
    assert_eq!(
        o.phase,
        Phase::FactProposal.as_u8(),
        "conflict => FactProposal (dispute core seam)"
    );
    assert_eq!(
        o.dispute_bond_total, total_stake,
        "dispute_bond_total == Σ bonds (fact-quorum denominator)"
    );
    assert_eq!(
        o.phase_ends_at,
        now_at_finalize + PHASE_WINDOW,
        "FactProposal window = now + PHASE_WINDOW"
    );
}

#[test]
fn finalize_window_still_open_fails() {
    let mut ctx = TestCtx::new();
    let oracle = setup_open(&mut ctx, 1, 3);
    let proposers = propose_each(&mut ctx, oracle, &[0, 0], 5_000);

    // No warp past phase_ends_at: window still open.
    assert!(ctx.now() < ctx.oracle(oracle).phase_ends_at);

    let ix = ctx.finalize_proposals_ix(oracle, &proposers);
    let res = ctx.send(ix, &[]);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::WindowNotElapsed as u32),
        "finalize before window end must fail WindowNotElapsed: {res:?}"
    );
}

#[test]
fn finalize_wrong_phase_fails() {
    let mut ctx = TestCtx::new();
    let oracle = setup_open(&mut ctx, 1, 3);
    let proposers = propose_each(&mut ctx, oracle, &[0, 0], 5_000);
    ctx.warp(PROPOSAL_WINDOW + 10);

    // Force a non-Proposal phase.
    ctx.set_phase(oracle, Phase::FactProposal);

    let ix = ctx.finalize_proposals_ix(oracle, &proposers);
    let res = ctx.send(ix, &[]);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::WrongPhase as u32),
        "finalize in a non-Proposal phase must fail WrongPhase: {res:?}"
    );
}

#[test]
fn finalize_subset_of_proposers_fails() {
    let mut ctx = TestCtx::new();
    let oracle = setup_open(&mut ctx, 1, 3);
    let proposers = propose_each(&mut ctx, oracle, &[0, 0, 0], 5_000);
    ctx.warp(PROPOSAL_WINDOW + 10);

    // Pass only a subset of the proposer set (count mismatch).
    let ix = ctx.finalize_proposals_ix(oracle, &proposers[..2]);
    let res = ctx.send(ix, &[]);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidAccount as u32),
        "subset of proposers must fail InvalidAccount: {res:?}"
    );
}

#[test]
fn finalize_foreign_oracle_proposer_fails() {
    let mut ctx = TestCtx::new();
    // Oracle A.
    let oracle_a = setup_open(&mut ctx, 1, 3);
    let prop_a = propose_each(&mut ctx, oracle_a, &[0, 0], 5_000);

    // Oracle B (same protocol) with its own proposer.
    let deadline_b = ctx.now() + DEADLINE_DELTA;
    let (oracle_b, res) = ctx.create_oracle(2, 3, deadline_b, 600);
    assert!(res.is_ok(), "create_oracle B should succeed: {res:?}");
    ctx.warp(DEADLINE_DELTA);
    let prop_b = propose_each(&mut ctx, oracle_b, &[0], 5_000);

    ctx.warp(PROPOSAL_WINDOW + 10);

    // Finalize A but slip in B's proposer (count still matches A's count == 2).
    let tail = vec![prop_a[0], prop_b[0]];
    let ix = ctx.finalize_proposals_ix(oracle_a, &tail);
    let res = ctx.send(ix, &[]);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidAccount as u32),
        "a foreign-oracle proposer in the tail must fail InvalidAccount: {res:?}"
    );
}

#[test]
fn finalize_duplicate_proposer_fails() {
    let mut ctx = TestCtx::new();
    let oracle = setup_open(&mut ctx, 1, 3);
    let proposers = propose_each(&mut ctx, oracle, &[0, 0], 5_000);
    ctx.warp(PROPOSAL_WINDOW + 10);

    // proposer_count == 2, but pass the same proposer twice.
    let tail = vec![proposers[0], proposers[0]];
    let ix = ctx.finalize_proposals_ix(oracle, &tail);
    let res = ctx.send(ix, &[]);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidAccount as u32),
        "a duplicate proposer in the tail must fail InvalidAccount: {res:?}"
    );
}

#[test]
fn finalize_no_proposers_fails() {
    let mut ctx = TestCtx::new();
    let oracle = setup_open(&mut ctx, 1, 3);
    // No proposals at all; cross phase_ends_at.
    ctx.warp(PROPOSAL_WINDOW + 10);

    let ix = ctx.finalize_proposals_ix(oracle, &[]);
    let res = ctx.send(ix, &[]);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::NoProposals as u32),
        "finalize with no proposers must fail NoProposals: {res:?}"
    );
}

#[test]
fn finalize_idempotent_second_call_fails() {
    let mut ctx = TestCtx::new();
    let oracle = setup_open(&mut ctx, 1, 3);
    let proposers = propose_each(&mut ctx, oracle, &[2, 2], 5_000);
    ctx.warp(PROPOSAL_WINDOW + 10);

    let ix = ctx.finalize_proposals_ix(oracle, &proposers);
    let res = ctx.send(ix, &[]);
    assert!(res.is_ok(), "first finalize should succeed: {res:?}");
    assert_eq!(ctx.oracle(oracle).phase, Phase::Resolved.as_u8());

    // Second call: phase is now Resolved, so it fails the phase gate.
    let ix = ctx.finalize_proposals_ix(oracle, &proposers);
    let res = ctx.send(ix, &[]);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::WrongPhase as u32),
        "second finalize must fail WrongPhase (idempotency): {res:?}"
    );
}

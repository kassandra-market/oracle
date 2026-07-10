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

use kassandra_oracles_program::state::Phase;
use solana_instruction::Instruction;
use solana_pubkey::Pubkey;

#[path = "finalize_oracle/gating.rs"]
mod gating;
#[path = "finalize_oracle/resolve.rs"]
mod resolve;

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

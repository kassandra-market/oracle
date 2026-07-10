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

use kassandra_oracles_program::{instruction::Ix, state::Phase};
use solana_pubkey::Pubkey;
use solana_signer::Signer;

#[path = "ai_claim/submit.rs"]
mod submit;
#[path = "ai_claim/finalize.rs"]
mod finalize;

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

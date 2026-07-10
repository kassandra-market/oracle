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

use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

#[path = "finalize_facts/agreed.rs"]
mod agreed;
#[path = "finalize_facts/gating.rs"]
mod gating;

// ----- instruction builders -------------------------------------------------

/// Build a `finalize_facts` instruction: oracle (writable) + a tail of the
/// given accounts (all writable). No signer is required.
fn finalize_facts_ix(ctx: &TestCtx, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
    ctx.finalize_facts_ix(oracle, tail)
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
    ctx.send(ix, &[&submitter])
        .expect("submit_fact should succeed");
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

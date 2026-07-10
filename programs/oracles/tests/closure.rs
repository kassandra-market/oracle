//! Task S4 — account closure: `close_ai_claim` (Ix 20) + `close_market` (Ix 21).
//!
//! These are permissionless, post-resolution RENT reclaims (no token movement
//! beyond the already-drained escrow). `close_ai_claim` drains an `AiClaim`'s
//! rent to the proposer's authority and closes it; `close_market` closes a
//! settled `Market` + its (empty) `challenger_usdc_vault` escrow, reclaiming both
//! rents to the challenger. Each test asserts the rent landed at the bound
//! recipient, the account(s) closed, gating (terminal phase / settled / empty
//! escrow / oracle binding), and idempotency by closure.

mod common;
use common::*;

use kassandra_oracles_program::{error::KassandraError, state::Phase};
use solana_instruction_error::InstructionError;
use solana_keypair::Keypair;
use solana_signer::Signer;
use solana_transaction_error::TransactionError;

/// The custom-error expectation for a failed instruction at index 0.
fn custom(e: KassandraError) -> TransactionError {
    TransactionError::InstructionError(0, InstructionError::Custom(e as u32))
}

/// A minimal terminal Resolved oracle carrying a single proposer (for the
/// AiClaim binding) and no facts.
fn terminal_with_proposer(ctx: &mut TestCtx) -> TerminalSeed {
    ctx.seed_terminal_oracle(
        Phase::Resolved,
        1,
        &[ClaimProposerSpec {
            bond: 1_000,
            claim_option: 1,
            disqualified: false,
            slashed_amount: 0,
        }],
        &[],
        1,
        2,
    )
}

// ---------------------------------------------------------------------------
// close_ai_claim (Ix 20)
// ---------------------------------------------------------------------------

#[test]
fn close_ai_claim_after_resolved_reclaims_rent() {
    let mut ctx = TestCtx::new();
    let seed = terminal_with_proposer(&mut ctx);
    let p = &seed.proposers[0];
    let recip = p.authority.pubkey();
    let ai_claim = ctx.seed_ai_claim(seed.oracle, p.account, recip);

    let claim_rent = ctx.lamports(ai_claim);
    let recip_before = ctx.lamports(recip);
    assert!(claim_rent > 0);

    let ix = ctx.close_ai_claim_ix(seed.oracle, ai_claim, recip);
    ctx.send(ix, &[]).unwrap();

    assert!(ctx.is_closed(ai_claim), "ai_claim should be closed");
    assert_eq!(
        ctx.lamports(recip),
        recip_before + claim_rent,
        "rent → ai_claim.authority",
    );
}

/// The previously-stranding order: `claim_proposer` CLOSES the Proposer first,
/// then `close_ai_claim` still reclaims the AiClaim's rent — the recipient comes
/// from `ai_claim.authority`, not the (now-gone) Proposer.
#[test]
fn close_ai_claim_after_proposer_closed_still_reclaims() {
    let mut ctx = TestCtx::new();
    let seed = terminal_with_proposer(&mut ctx);
    let p = &seed.proposers[0];
    let recip = p.authority.pubkey();
    let ai_claim = ctx.seed_ai_claim(seed.oracle, p.account, recip);

    // Crank claim_proposer → the Proposer account is closed FIRST.
    let claim_ix = ctx.claim_proposer_ix(
        seed.oracle,
        seed.nonce,
        p.account,
        p.dest_kass,
        seed.stake_vault,
        recip,
    );
    ctx.send(claim_ix, &[]).unwrap();
    assert!(ctx.is_closed(p.account), "proposer closed first");

    // close_ai_claim still works (no Proposer dependency).
    let claim_rent = ctx.lamports(ai_claim);
    let recip_before = ctx.lamports(recip);
    let ix = ctx.close_ai_claim_ix(seed.oracle, ai_claim, recip);
    ctx.send(ix, &[]).unwrap();

    assert!(ctx.is_closed(ai_claim));
    assert_eq!(ctx.lamports(recip), recip_before + claim_rent);
}

#[test]
fn close_ai_claim_non_terminal_fails() {
    let mut ctx = TestCtx::new();
    let seed = terminal_with_proposer(&mut ctx);
    let p = &seed.proposers[0];
    let recip = p.authority.pubkey();
    // Force a non-terminal phase.
    ctx.set_phase(seed.oracle, Phase::Challenge);
    let ai_claim = ctx.seed_ai_claim(seed.oracle, p.account, recip);

    let ix = ctx.close_ai_claim_ix(seed.oracle, ai_claim, recip);
    assert_eq!(
        ctx.send(ix, &[]).unwrap_err().err,
        custom(KassandraError::WrongPhase)
    );
}

#[test]
fn close_ai_claim_double_close_fails() {
    let mut ctx = TestCtx::new();
    let seed = terminal_with_proposer(&mut ctx);
    let p = &seed.proposers[0];
    let recip = p.authority.pubkey();
    let ai_claim = ctx.seed_ai_claim(seed.oracle, p.account, recip);

    let ix = ctx.close_ai_claim_ix(seed.oracle, ai_claim, recip);
    ctx.send(ix, &[]).unwrap();
    assert!(ctx.is_closed(ai_claim));

    // Second close: the account is gone → load guard fails.
    let ix2 = ctx.close_ai_claim_ix(seed.oracle, ai_claim, recip);
    assert_eq!(
        ctx.send(ix2, &[]).unwrap_err().err,
        custom(KassandraError::InvalidAccount),
    );
}

#[test]
fn close_ai_claim_other_oracle_fails() {
    let mut ctx = TestCtx::new();
    let seed = terminal_with_proposer(&mut ctx);
    let p = &seed.proposers[0];
    let recip = p.authority.pubkey();
    // AiClaim bound to a DIFFERENT oracle than the one passed in.
    let other_oracle = solana_pubkey::Pubkey::new_unique();
    let ai_claim = ctx.seed_ai_claim(other_oracle, p.account, recip);

    let ix = ctx.close_ai_claim_ix(seed.oracle, ai_claim, recip);
    assert_eq!(
        ctx.send(ix, &[]).unwrap_err().err,
        custom(KassandraError::InvalidAccount),
    );
}

// ---------------------------------------------------------------------------
// close_market (Ix 21)
// ---------------------------------------------------------------------------

/// Seed a terminal oracle + a challenger (airdropped) + a USDC escrow owned by
/// the oracle PDA + a Market. Returns (seed, challenger, escrow, market).
fn market_fixture(
    ctx: &mut TestCtx,
    escrow_amount: u64,
    settled: bool,
) -> (
    TerminalSeed,
    Keypair,
    solana_pubkey::Pubkey,
    solana_pubkey::Pubkey,
) {
    let seed = ctx.seed_terminal_oracle(Phase::Resolved, 1, &[], &[], 1, 2);
    let challenger = Keypair::new();
    ctx.airdrop(&challenger, 1_000_000_000);
    let escrow = ctx.seed_usdc_escrow(seed.oracle, escrow_amount);
    let market = ctx.seed_market(seed.oracle, challenger.pubkey(), escrow, settled);
    (seed, challenger, escrow, market)
}

#[test]
fn close_market_after_settle_reclaims_rent() {
    let mut ctx = TestCtx::new();
    let (seed, challenger, escrow, market) = market_fixture(&mut ctx, 0, true);

    let escrow_rent = ctx.lamports(escrow);
    let market_rent = ctx.lamports(market);
    let chal_before = ctx.lamports(challenger.pubkey());
    assert!(escrow_rent > 0 && market_rent > 0);

    let ix = ctx.close_market_ix(seed.oracle, seed.nonce, market, escrow, challenger.pubkey());
    ctx.send(ix, &[]).unwrap();

    assert!(ctx.is_closed(market), "market PDA should be closed");
    assert!(
        ctx.is_closed(escrow),
        "escrow token account should be closed"
    );
    assert_eq!(
        ctx.lamports(challenger.pubkey()),
        chal_before + escrow_rent + market_rent,
        "both rents → challenger",
    );
}

#[test]
fn close_market_unsettled_fails() {
    let mut ctx = TestCtx::new();
    let (seed, challenger, escrow, market) = market_fixture(&mut ctx, 0, false);

    let ix = ctx.close_market_ix(seed.oracle, seed.nonce, market, escrow, challenger.pubkey());
    assert_eq!(
        ctx.send(ix, &[]).unwrap_err().err,
        custom(KassandraError::MarketNotSettled),
    );
}

#[test]
fn close_market_nonempty_escrow_fails() {
    let mut ctx = TestCtx::new();
    let (seed, challenger, escrow, market) = market_fixture(&mut ctx, 500_000, true);

    let ix = ctx.close_market_ix(seed.oracle, seed.nonce, market, escrow, challenger.pubkey());
    assert_eq!(
        ctx.send(ix, &[]).unwrap_err().err,
        custom(KassandraError::EscrowNotEmpty),
    );
}

#[test]
fn close_market_double_close_fails() {
    let mut ctx = TestCtx::new();
    let (seed, challenger, escrow, market) = market_fixture(&mut ctx, 0, true);

    let ix = ctx.close_market_ix(seed.oracle, seed.nonce, market, escrow, challenger.pubkey());
    ctx.send(ix, &[]).unwrap();
    assert!(ctx.is_closed(market));

    let ix2 = ctx.close_market_ix(seed.oracle, seed.nonce, market, escrow, challenger.pubkey());
    assert_eq!(
        ctx.send(ix2, &[]).unwrap_err().err,
        custom(KassandraError::InvalidAccount),
    );
}

#[test]
fn close_market_non_terminal_fails() {
    let mut ctx = TestCtx::new();
    let (seed, challenger, escrow, market) = market_fixture(&mut ctx, 0, true);
    ctx.set_phase(seed.oracle, Phase::Challenge);

    let ix = ctx.close_market_ix(seed.oracle, seed.nonce, market, escrow, challenger.pubkey());
    assert_eq!(
        ctx.send(ix, &[]).unwrap_err().err,
        custom(KassandraError::WrongPhase),
    );
}

//! Task S2 — the three claim-and-close instructions (`claim_proposer`,
//! `claim_fact`, `claim_fact_vote`): the first PHYSICAL settlement payouts.
//!
//! Each test drives a TERMINAL oracle seeded by
//! [`TestCtx::seed_terminal_oracle`] (which stamps self-consistent resolution
//! totals + a `reward_pool` equal to the physically-slashed KASS), claims an
//! account, and asserts: the exact KASS delta to the owner's account, the
//! claimant account closed (rent reclaimed to its authority), and the stake
//! vault decremented by exactly the entitlement. Conservation arms prove the
//! whole sweep drains the vault to floor dust (Resolved) or to the burned
//! slashed pool (InvalidDeadend: the dead-end finalize burned the slashed
//! `bond_pool`, so claims return only the non-slashed principal), sourced ONLY
//! from the stake vault.

mod common;
use common::*;

use kassandra_oracles_program::state::{Phase, VOTE_APPROVE, VOTE_DUPLICATE};

#[path = "claims/resolved_matrix.rs"]
mod resolved_matrix;

#[path = "claims/invalid_deadend.rs"]
mod invalid_deadend;

#[path = "claims/proposer_reward.rs"]
mod proposer_reward;

#[path = "claims/guards.rs"]
mod guards;

/// A rich Resolved oracle exercising every matrix row at once. `resolved_option
/// == 1`. Returns the seed; `slash = 1/2`.
fn resolved_full(ctx: &mut TestCtx) -> TerminalSeed {
    let proposers = vec![
        // correct survivor
        ClaimProposerSpec {
            bond: 1_000,
            claim_option: 1,
            disqualified: false,
            slashed_amount: 0,
        },
        // wrong-but-survived
        ClaimProposerSpec {
            bond: 1_000,
            claim_option: 0,
            disqualified: false,
            slashed_amount: 0,
        },
        // disqualified (full slash to bond_pool)
        ClaimProposerSpec {
            bond: 1_000,
            claim_option: 0,
            disqualified: true,
            slashed_amount: 1_000,
        },
    ];
    let facts = vec![
        // agreed: submitter + two approve voters earn the fact rate
        ClaimFactSpec {
            stake: 1_000,
            agreed: true,
            duplicate: false,
            votes: vec![
                ClaimVoteSpec {
                    stake: 500,
                    kind: VOTE_APPROVE,
                },
                ClaimVoteSpec {
                    stake: 500,
                    kind: VOTE_APPROVE,
                },
            ],
        },
        // rejected: submitter forfeits 100%, approve voters slashed 1/2 (even stakes)
        ClaimFactSpec {
            stake: 1_000,
            agreed: false,
            duplicate: false,
            votes: vec![
                ClaimVoteSpec {
                    stake: 400,
                    kind: VOTE_APPROVE,
                },
                ClaimVoteSpec {
                    stake: 600,
                    kind: VOTE_APPROVE,
                },
            ],
        },
        // duplicate-dominant: submitter + dup voter get stake; an approve voter
        // on it gets stake too (no reward, no slash)
        ClaimFactSpec {
            stake: 1_000,
            agreed: false,
            duplicate: true,
            votes: vec![
                ClaimVoteSpec {
                    stake: 500,
                    kind: VOTE_DUPLICATE,
                },
                ClaimVoteSpec {
                    stake: 300,
                    kind: VOTE_APPROVE,
                },
            ],
        },
    ];
    ctx.seed_terminal_oracle(Phase::Resolved, 1, &proposers, &facts, 1, 2)
}

/// Claim a proposer and assert: dest credited exactly `expected`, account
/// closed, vault decremented by exactly `expected`, rent reclaimed to authority.
fn assert_claim(
    ctx: &mut TestCtx,
    ix: solana_instruction::Instruction,
    account: solana_pubkey::Pubkey,
    dest: solana_pubkey::Pubkey,
    stake_vault: solana_pubkey::Pubkey,
    recipient: solana_pubkey::Pubkey,
    expected: u64,
) {
    let vault_before = ctx.token_balance(stake_vault);
    let dest_before = ctx.token_balance(dest);
    let recip_before = ctx.lamports(recipient);
    let rent = ctx.lamports(account);

    let res = ctx.send(ix, &[]);
    assert!(res.is_ok(), "claim should succeed: {res:?}");

    assert_eq!(
        ctx.token_balance(dest) - dest_before,
        expected,
        "dest credited exactly the entitlement"
    );
    assert_eq!(
        vault_before - ctx.token_balance(stake_vault),
        expected,
        "stake vault decremented by exactly the entitlement"
    );
    assert!(ctx.is_closed(account), "claimant account closed");
    assert_eq!(
        ctx.lamports(recipient) - recip_before,
        rent,
        "rent reclaimed to the account authority"
    );
}

//! `finalize_oracle` — terminal resolution outcomes (Winner / dead-end).

use super::*;
use kassandra_oracles_program::state::CLAIM_OPTION_NONE;

#[test]
fn clear_winner_resolves() {
    // Three survivors voting [1, 1, 2] -> option 1 wins.
    let (mut ctx, oracle, pdas) = seed_challenge(
        &[
            ProposerSpec {
                option: 1,
                bond: 1_000,
            },
            ProposerSpec {
                option: 1,
                bond: 1_000,
            },
            ProposerSpec {
                option: 2,
                bond: 1_000,
            },
        ],
        &[1, 1, 2],
    );
    let vault = ctx.seeded(oracle).stake_vault;
    let vault_before = ctx.token_balance(vault);

    ctx.warp(WINDOW + 1);
    ctx.send(finalize_oracle_ix(&ctx, oracle, &pdas), &[])
        .expect("finalize should succeed");

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::Resolved as u8);
    assert_eq!(o.resolved_option, 1);
    // No token CPI: the vault is untouched.
    assert_eq!(ctx.token_balance(vault), vault_before);
    // S1: total_correct_proposer_stake == Σ bond of survivors voting option 1
    // (the two 1_000-bond proposers; the option-2 proposer is excluded).
    assert_eq!(o.total_correct_proposer_stake, 2_000);
    // reward_pool == bond_pool (0 here; S3 folds emission in).
    assert_eq!(o.reward_pool, o.bond_pool);
    assert_eq!(o.reward_pool, 0);
}

#[test]
fn tie_is_invalid_deadend() {
    // Two survivors voting [0, 1] -> tie -> dead-end.
    let (mut ctx, oracle, pdas) = seed_challenge(
        &[
            ProposerSpec {
                option: 0,
                bond: 1_000,
            },
            ProposerSpec {
                option: 1,
                bond: 1_000,
            },
        ],
        &[0, 1],
    );
    ctx.warp(WINDOW + 1);
    ctx.send(finalize_oracle_ix(&ctx, oracle, &pdas), &[])
        .expect("finalize should succeed");

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::InvalidDeadend as u8);
    assert_eq!(o.resolved_option, CLAIM_OPTION_NONE);
    // S1: a dead-end distributes no rewards — both stamps stay 0.
    assert_eq!(o.total_correct_proposer_stake, 0);
    assert_eq!(o.reward_pool, 0);
}

#[test]
fn resolved_stamps_correct_total_and_reward_pool() {
    // S1: total_correct_proposer_stake == Σ bond of SURVIVORS voting the resolved
    // option (excludes a wrong-but-survived proposer); reward_pool == bond_pool
    // (non-zero here via a prior flip-slash already in the pool).
    let (mut ctx, oracle, pdas) = seed_challenge(
        &[
            ProposerSpec {
                option: 1,
                bond: 1_000,
            },
            ProposerSpec {
                option: 1,
                bond: 3_000,
            },
            ProposerSpec {
                option: 2,
                bond: 5_000,
            },
        ],
        &[1, 1, 2],
    );
    // A prior flip-slash on the option-2 proposer puts 700 into bond_pool (it
    // stays surviving + non-disqualified, so finalize_oracle still counts it as a
    // survivor voting option 2 — excluded from the correct-proposer total).
    ctx.set_proposer_prior_slash(oracle, pdas[2], 700);

    ctx.warp(WINDOW + 1);
    ctx.send(finalize_oracle_ix(&ctx, oracle, &pdas), &[])
        .expect("finalize should succeed");

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::Resolved as u8);
    assert_eq!(o.resolved_option, 1);
    // Σ bond over survivors voting option 1: 1_000 + 3_000 = 4_000.
    assert_eq!(o.total_correct_proposer_stake, 4_000);
    // reward_pool finalized to bond_pool (the 700 prior slash).
    assert_eq!(o.bond_pool, 700);
    assert_eq!(o.reward_pool, 700);
}

#[test]
fn all_disqualified_is_invalid_deadend() {
    // Both proposers disqualified, surviving_count forced to 0 -> NoSurvivors.
    let (mut ctx, oracle, pdas) = seed_challenge(
        &[
            ProposerSpec {
                option: 0,
                bond: 1_000,
            },
            ProposerSpec {
                option: 1,
                bond: 1_000,
            },
        ],
        &[0, 1],
    );
    for pda in &pdas {
        ctx.set_proposer_disqualified(*pda);
    }
    ctx.set_surviving_count(oracle, 0);

    ctx.warp(WINDOW + 1);
    ctx.send(finalize_oracle_ix(&ctx, oracle, &pdas), &[])
        .expect("finalize should succeed");

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::InvalidDeadend as u8);
    // Dead-end: resolved_option is stamped with the loud sentinel, not 0.
    assert_eq!(o.resolved_option, CLAIM_OPTION_NONE);
}

#[test]
fn one_survivor_among_disqualified_resolves() {
    // p0 survives (votes 2); p1 disqualified. surviving_count = 1 -> Winner(2).
    let (mut ctx, oracle, pdas) = seed_challenge(
        &[
            ProposerSpec {
                option: 2,
                bond: 1_000,
            },
            ProposerSpec {
                option: 1,
                bond: 1_000,
            },
        ],
        &[2, 1],
    );
    ctx.set_proposer_disqualified(pdas[1]);
    ctx.set_surviving_count(oracle, 1);

    ctx.warp(WINDOW + 1);
    ctx.send(finalize_oracle_ix(&ctx, oracle, &pdas), &[])
        .expect("finalize should succeed");

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::Resolved as u8);
    assert_eq!(o.resolved_option, 2);
}

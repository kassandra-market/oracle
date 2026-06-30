//! Task S5 — end-to-end STAKER-SETTLEMENT lifecycle tests.
//!
//! These drive a full lifecycle to a terminal state and then have EVERY staker
//! claim (the S2 `claim_proposer` / `claim_fact` / `claim_fact_vote`) and every
//! account close (the S4 `close_ai_claim` / `close_market`), asserting the
//! per-actor matrix, that all accounts close, that the stake vault drains to
//! floor dust, and KASS conservation sourced ONLY from the stake vault.
//!
//! # What is REAL vs SEEDED (honest split)
//! * **Tests 1-2 (no emission)** drive the dispute core through the GENUINE front
//!   door — `create_oracle → propose×2 (conflict) → finalize_proposals →
//!   submit_fact → advance_phase → vote_fact → finalize_facts → submit_ai_claim×2
//!   → finalize_ai_claims → finalize_oracle` (no `set_phase`; only `warp` moves
//!   time) — then run REAL claims + closes. Everything is a real instruction.
//! * **Tests 3-5 (emission)** SEED the disputed oracle (the dispute mechanics are
//!   exhaustively covered by `lifecycle_e2e` / `invariants` Arm A) but keep the
//!   emission MOVEMENT + settlement REAL: the creation-time `reward_emission` is
//!   placed in the vault (backed by mint supply), then the REAL `finalize_oracle`
//!   folds it into `reward_pool` (Resolved) or BURNS it back (InvalidDeadend), and
//!   the REAL claims/closes settle it. The mint-AT-CREATION half of emission
//!   (`create_oracle` minting from the reservoir, the fee-burn boost, the
//!   mint-authority guard) is covered by `tests/emissions.rs`.
//!
//! The independent-reference CONSERVATION FUZZ (every matrix combination ×
//! emission × outcome, asserted against a reimplemented reference) lives in
//! `tests/invariants.rs` (Arms D + E).

mod common;
use common::*;

use kassandra_program::{
    config::PHASE_WINDOW,
    instruction::Ix,
    reward,
    state::{Phase, CLAIM_OPTION_NONE, VOTE_APPROVE},
};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    system_program,
};
use spl_token::ID as TOKEN_PROGRAM_ID;

// ---------------------------------------------------------------------------
// Dispute-core instruction builders (mirror lifecycle_e2e.rs / challenge_e2e.rs)
// ---------------------------------------------------------------------------

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

/// AiClaim PDA seeds `[b"claim", oracle, proposer]`.
fn claim_pda(program_id: &Pubkey, oracle: &Pubkey, proposer: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"claim", oracle.as_ref(), proposer.as_ref()], program_id)
}

fn submit_ai_payload(option: u8) -> Vec<u8> {
    let mut data = Vec::with_capacity(1 + 32 + 32 + 32 + 1);
    data.push(Ix::SubmitAiClaim as u8);
    data.extend_from_slice(&[0xAA; 32]);
    data.extend_from_slice(&[0xBB; 32]);
    data.extend_from_slice(&[0xCC; 32]);
    data.push(option);
    data
}

fn submit_ai_claim_ix(
    ctx: &TestCtx,
    oracle: Pubkey,
    proposer: Pubkey,
    claim: Pubkey,
    authority: Pubkey,
    data: Vec<u8>,
) -> Instruction {
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(proposer, false),
            AccountMeta::new(claim, false),
            AccountMeta::new(authority, true),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data,
    }
}

fn finalize_ai_claims_ix(ctx: &TestCtx, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
    let mut accounts = Vec::with_capacity(1 + tail.len());
    accounts.push(AccountMeta::new(oracle, false));
    for k in tail {
        accounts.push(AccountMeta::new(*k, false));
    }
    Instruction {
        program_id: ctx.program_id,
        accounts,
        data: vec![Ix::FinalizeAiClaims as u8],
    }
}

// ---------------------------------------------------------------------------
// Real front-door dispute driver (tests 1-2)
// ---------------------------------------------------------------------------

/// Everything tests 1-2 need to settle the terminal oracle: the oracle handles
/// plus the staker accounts to claim + close.
struct Driven {
    oracle: Pubkey,
    nonce: u64,
    vault: Pubkey,
    /// (authority, proposer_pda, ai_claim_pda) per proposer, in spec order.
    proposers: Vec<(Keypair, Pubkey, Pubkey)>,
    fact: Pubkey,
    fact_submitter: Keypair,
    fact_vote: Pubkey,
    voter: Keypair,
}

/// Drive the REAL dispute core to a terminal state, with one AGREED fact (one
/// approve voter clearing the 2/3 quorum) and both proposers submitting the AI
/// claim options in `claim_options` (so the caller picks Resolved-with-a-winner
/// vs a tie → InvalidDeadend). No emission (genesis default).
fn drive_real_dispute(ctx: &mut TestCtx, claim_options: [u8; 2]) -> Driven {
    let bond = 1_000u64;
    // create_oracle → propose×2 (DISTINCT options 0/1) → finalize_proposals.
    let oracle = ctx.dispute_via_real_flow(&[
        ProposerSpec { option: 0, bond },
        ProposerSpec { option: 1, bond },
    ]);
    let (vault, _) = TestCtx::stake_vault_pda(&ctx.program_id, &oracle);
    let nonce = ctx.seeded(oracle).nonce;
    let proposer_pdas: Vec<Pubkey> = ctx.proposers(oracle).iter().map(|p| p.pda).collect();
    let authorities: Vec<Keypair> = ctx
        .proposers(oracle)
        .iter()
        .map(|p| p.authority.insecure_clone())
        .collect();

    // submit_fact (FactProposal window open).
    let submitter = Keypair::new();
    ctx.svm.airdrop(&submitter.pubkey(), 1_000_000_000).unwrap();
    let submitter_kass = ctx.fund_kass(&submitter, 1_000_000);
    let content_hash = [0x07u8; 32];
    let (fact, _) = TestCtx::fact_pda(&ctx.program_id, &oracle, &content_hash);
    ctx.send(
        submit_fact_ix(
            ctx,
            oracle,
            fact,
            submitter.pubkey(),
            submitter_kass,
            vault,
            submit_fact_payload(&content_hash, 300, b"ipfs://fact"),
        ),
        &[&submitter],
    )
    .expect("submit_fact");

    // warp past FactProposal, advance_phase → FactVoting.
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(advance_phase_ix(ctx, oracle), &[])
        .expect("advance_phase");

    // approve well past the 2/3 quorum of dispute_bond_total (== 2*bond == 2000).
    let voter = Keypair::new();
    ctx.svm.airdrop(&voter.pubkey(), 1_000_000_000).unwrap();
    let voter_kass = ctx.fund_kass(&voter, 2_000);
    let (fact_vote, _) = TestCtx::vote_pda(&ctx.program_id, &fact, &voter.pubkey());
    ctx.send(
        vote_fact_ix(
            ctx,
            oracle,
            fact,
            fact_vote,
            voter.pubkey(),
            voter_kass,
            vault,
            vote_payload(VOTE_APPROVE, 2_000),
        ),
        &[&voter],
    )
    .expect("vote_fact");

    // warp past voting, finalize_facts → AiClaim (fact agreed).
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(finalize_facts_ix(ctx, oracle, &[fact]), &[])
        .expect("finalize_facts");
    assert_eq!(ctx.fact(fact).agreed, 1, "fact cleared the 2/3 quorum");

    // submit_ai_claim per proposer with the chosen claim options.
    for (i, (auth, pda)) in authorities.iter().zip(&proposer_pdas).enumerate() {
        ctx.svm.airdrop(&auth.pubkey(), 1_000_000_000).unwrap();
        let (claim, _) = claim_pda(&ctx.program_id, &oracle, pda);
        ctx.send(
            submit_ai_claim_ix(
                ctx,
                oracle,
                *pda,
                claim,
                auth.pubkey(),
                submit_ai_payload(claim_options[i]),
            ),
            &[auth],
        )
        .expect("submit_ai_claim");
    }

    // warp past AiClaim, finalize_ai_claims → Challenge.
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(finalize_ai_claims_ix(ctx, oracle, &proposer_pdas), &[])
        .expect("finalize_ai_claims");
    assert_eq!(ctx.oracle(oracle).phase, Phase::Challenge.as_u8());

    // warp past Challenge (open_challenge_count stays 0), finalize_oracle → term.
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(ctx.finalize_oracle_ix(oracle, &proposer_pdas), &[])
        .expect("finalize_oracle");

    let proposers = authorities
        .into_iter()
        .zip(&proposer_pdas)
        .map(|(auth, &pda)| {
            let (ai_claim, _) = claim_pda(&ctx.program_id, &oracle, &pda);
            (auth, pda, ai_claim)
        })
        .collect();

    Driven {
        oracle,
        nonce,
        vault,
        proposers,
        fact,
        fact_submitter: submitter,
        fact_vote,
        voter,
    }
}

/// The proposer-reward bucket for the oracle's current resolution stamps.
fn proposer_bucket_of(o: &kassandra_program::state::Oracle) -> u64 {
    reward::reward_buckets(
        o.reward_pool,
        o.reward_proposer_weight,
        o.reward_fact_weight,
        o.total_correct_proposer_stake,
        o.total_approved_fact_stake,
    )
    .0
}

/// The fact-reward bucket for the oracle's current resolution stamps.
fn fact_bucket_of(o: &kassandra_program::state::Oracle) -> u64 {
    reward::reward_buckets(
        o.reward_pool,
        o.reward_proposer_weight,
        o.reward_fact_weight,
        o.total_correct_proposer_stake,
        o.total_approved_fact_stake,
    )
    .1
}

// ---------------------------------------------------------------------------
// Test 1 — real dispute → Resolved → full claim + close sweep + conservation
// ---------------------------------------------------------------------------

#[test]
fn e2e_resolved_full_settlement_real_dispute() {
    let mut ctx = TestCtx::new();
    // Both proposers claim option 0 → Winner(0). P0 (orig 0) does NOT flip; P1
    // (orig 1) flips (partial slash, still surviving + correct).
    let d = drive_real_dispute(&mut ctx, [0, 0]);

    let o = ctx.oracle(d.oracle);
    assert_eq!(o.phase, Phase::Resolved as u8, "terminal: Resolved");
    assert_eq!(o.resolved_option, 0);
    // No emission at genesis: reward_pool is purely the physical bond_pool (the
    // flip slash on P1), and the vault holds exactly Σ stakes.
    assert_eq!(o.reward_emission, 0);
    assert_eq!(o.reward_pool, o.bond_pool);
    assert!(o.bond_pool > 0, "the flipper funded bond_pool");
    let vault_initial = ctx.token_balance(d.vault);

    let pbucket = proposer_bucket_of(&o);
    let fbucket = fact_bucket_of(&o);
    let mut total_claimed = 0u64;

    // ---- claim every fact vote, then the submitter (votes-first ordering) -----
    let vote = ctx.fact_vote(d.fact_vote);
    let voter_dest = ctx.fund_kass(&d.voter, 0);
    let expected_vote =
        vote.stake + reward::fact_reward(vote.stake, fbucket, o.total_approved_fact_stake);
    let ix = ctx.claim_fact_vote_ix(
        d.oracle,
        d.nonce,
        d.fact_vote,
        d.fact,
        voter_dest,
        d.vault,
        d.voter.pubkey(),
    );
    ctx.send(ix, &[]).expect("claim_fact_vote");
    assert_eq!(
        ctx.token_balance(voter_dest),
        expected_vote,
        "approve-voter on agreed fact: stake + reward"
    );
    assert!(ctx.is_closed(d.fact_vote));
    total_claimed += expected_vote;

    let fact = ctx.fact(d.fact);
    let submitter_dest = ctx.fund_kass(&d.fact_submitter, 0);
    let expected_sub =
        fact.stake + reward::fact_reward(fact.stake, fbucket, o.total_approved_fact_stake);
    let ix = ctx.claim_fact_ix(
        d.oracle,
        d.nonce,
        d.fact,
        submitter_dest,
        d.vault,
        d.fact_submitter.pubkey(),
    );
    ctx.send(ix, &[]).expect("claim_fact");
    assert_eq!(
        ctx.token_balance(submitter_dest),
        expected_sub,
        "agreed submitter: stake + reward"
    );
    assert!(ctx.is_closed(d.fact));
    total_claimed += expected_sub;

    // ---- claim every proposer (reward receivers last → exposes any shortfall) -
    for (auth, pda, ai_claim) in &d.proposers {
        let p = ctx.proposer(*pda);
        let base = p.bond - p.slashed_amount; // neither is disqualified here
        let reward = if p.claim_option == o.resolved_option {
            reward::proposer_reward(p.bond, pbucket, o.total_correct_proposer_stake)
        } else {
            0
        };
        let expected = base + reward;
        let dest = ctx.fund_kass(auth, 0);
        let ix = ctx.claim_proposer_ix(d.oracle, d.nonce, *pda, dest, d.vault, auth.pubkey());
        ctx.send(ix, &[]).expect("claim_proposer");
        assert_eq!(
            ctx.token_balance(dest),
            expected,
            "proposer matrix entitlement"
        );
        assert!(ctx.is_closed(*pda), "Proposer closed");
        total_claimed += expected;

        // ---- close the AiClaim (rent → its authority; order-independent) ------
        assert!(!ctx.is_closed(*ai_claim), "AiClaim still open pre-close");
        let ix = ctx.close_ai_claim_ix(d.oracle, *ai_claim, auth.pubkey());
        ctx.send(ix, &[]).expect("close_ai_claim");
        assert!(ctx.is_closed(*ai_claim), "AiClaim closed");
    }

    // ---- CONSERVATION: Σ claims + dust == vault_initial (vault drained to dust)
    let dust = ctx.token_balance(d.vault);
    assert_eq!(
        total_claimed + dust,
        vault_initial,
        "Σ claims + dust == vault"
    );
    assert!(dust <= o.reward_pool, "dust is only floor remainder");
    assert!(dust < 8, "floor-division dust is tiny: {dust}");
}

// ---------------------------------------------------------------------------
// Test 2 — real dispute → tie → InvalidDeadend → full returns + close
// ---------------------------------------------------------------------------

#[test]
fn e2e_invalid_deadend_full_returns_real_dispute() {
    let mut ctx = TestCtx::new();
    // Proposers claim DISTINCT options 0 and 1 → plurality tie → InvalidDeadend.
    // P0 (orig 0, claims 0) does not flip; P1 (orig 1, claims 1) does not flip
    // either — so on this dead-end neither is slashed and every staker reclaims
    // their full stake exactly (Σ payouts == Σ stakes, vault fully drained).
    let d = drive_real_dispute(&mut ctx, [0, 1]);

    let o = ctx.oracle(d.oracle);
    assert_eq!(
        o.phase,
        Phase::InvalidDeadend as u8,
        "terminal: InvalidDeadend"
    );
    assert_eq!(o.resolved_option, CLAIM_OPTION_NONE);
    assert_eq!(o.reward_pool, 0, "no reward distribution out of a dead-end");
    let vault_initial = ctx.token_balance(d.vault);
    let mut total_claimed = 0u64;

    // Fact vote + submitter: full stake back on InvalidDeadend.
    let vote = ctx.fact_vote(d.fact_vote);
    let voter_dest = ctx.fund_kass(&d.voter, 0);
    let ix = ctx.claim_fact_vote_ix(
        d.oracle,
        d.nonce,
        d.fact_vote,
        d.fact,
        voter_dest,
        d.vault,
        d.voter.pubkey(),
    );
    ctx.send(ix, &[]).expect("claim_fact_vote");
    assert_eq!(
        ctx.token_balance(voter_dest),
        vote.stake,
        "voter full stake on dead-end"
    );
    total_claimed += vote.stake;

    let fact = ctx.fact(d.fact);
    let submitter_dest = ctx.fund_kass(&d.fact_submitter, 0);
    let ix = ctx.claim_fact_ix(
        d.oracle,
        d.nonce,
        d.fact,
        submitter_dest,
        d.vault,
        d.fact_submitter.pubkey(),
    );
    ctx.send(ix, &[]).expect("claim_fact");
    assert_eq!(
        ctx.token_balance(submitter_dest),
        fact.stake,
        "submitter full stake on dead-end"
    );
    total_claimed += fact.stake;

    // Proposers: `bond − slashed_amount` (the flip slash, if any, stays as dust).
    let mut total_slash = 0u64;
    for (auth, pda, ai_claim) in &d.proposers {
        let p = ctx.proposer(*pda);
        let expected = p.bond - p.slashed_amount;
        total_slash += p.slashed_amount;
        let dest = ctx.fund_kass(auth, 0);
        let ix = ctx.claim_proposer_ix(d.oracle, d.nonce, *pda, dest, d.vault, auth.pubkey());
        ctx.send(ix, &[]).expect("claim_proposer");
        assert_eq!(ctx.token_balance(dest), expected);
        assert!(ctx.is_closed(*pda));
        total_claimed += expected;

        let ix = ctx.close_ai_claim_ix(d.oracle, *ai_claim, auth.pubkey());
        ctx.send(ix, &[]).expect("close_ai_claim");
        assert!(ctx.is_closed(*ai_claim));
    }

    // Conservation: Σ claims + dust == vault_initial; the only dust is the flip
    // slash that funded bond_pool (never distributed on a dead-end).
    let dust = ctx.token_balance(d.vault);
    assert_eq!(
        total_claimed + dust,
        vault_initial,
        "Σ claims + dust == vault"
    );
    assert_eq!(dust, total_slash, "dust == the un-distributed flip slash");
}

// ---------------------------------------------------------------------------
// Test 3 — emission ENABLED, Resolved: real finalize folds emission into the
// reward pool, claims reflect the emission-boosted reward, conservation includes
// the emission. (Dispute SEEDED; finalize_oracle fold + claims + emission REAL.)
// ---------------------------------------------------------------------------

#[test]
fn e2e_resolved_with_emission_real_finalize_and_claims() {
    let mut ctx = TestCtx::new();
    // Two proposers, both claim option 1 (the winner). No flip, no slash, so
    // bond_pool == 0 and the WHOLE reward pool is the creation-time emission.
    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 1,
            bond: 1_000,
        },
        ProposerSpec {
            option: 1,
            bond: 3_000,
        },
    ]);
    let pdas: Vec<Pubkey> = ctx.proposers(oracle).iter().map(|p| p.pda).collect();
    let auths: Vec<Keypair> = ctx
        .proposers(oracle)
        .iter()
        .map(|p| p.authority.insecure_clone())
        .collect();
    for p in &pdas {
        ctx.set_proposer_claim_option(*p, 1);
    }
    ctx.set_phase(oracle, Phase::Challenge);

    // The creation-time emission (placed in the vault, backed by supply). The REAL
    // finalize_oracle folds it into reward_pool below.
    let emission = 900u64;
    ctx.set_reward_emission(oracle, emission);

    let vault = ctx.seeded(oracle).stake_vault;
    let nonce = ctx.seeded(oracle).nonce;
    let vault_initial = ctx.token_balance(vault);
    assert_eq!(vault_initial, 4_000 + emission, "Σ bonds + emission");

    // REAL finalize_oracle → Resolved, folding the emission into reward_pool.
    ctx.warp(WINDOW + 1);
    ctx.send(ctx.finalize_oracle_ix(oracle, &pdas), &[])
        .expect("finalize_oracle");
    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::Resolved as u8);
    assert_eq!(
        o.reward_pool,
        o.bond_pool + emission,
        "reward_pool folds emission"
    );
    assert_eq!(
        o.reward_pool, emission,
        "bond_pool 0 → pool is pure emission"
    );
    assert_eq!(o.total_correct_proposer_stake, 4_000);

    let pbucket = proposer_bucket_of(&o);
    let mut total_claimed = 0u64;
    for (auth, pda) in auths.iter().zip(&pdas) {
        let p = ctx.proposer(*pda);
        let reward = reward::proposer_reward(p.bond, pbucket, o.total_correct_proposer_stake);
        assert!(reward > 0, "emission funds a positive proposer reward");
        let expected = p.bond + reward;
        let dest = ctx.fund_kass(auth, 0);
        let ix = ctx.claim_proposer_ix(oracle, nonce, *pda, dest, vault, auth.pubkey());
        ctx.send(ix, &[]).expect("claim_proposer");
        assert_eq!(
            ctx.token_balance(dest),
            expected,
            "bond + emission-funded reward"
        );
        total_claimed += expected;
    }

    // Conservation INCLUDES the emission: Σ claims + dust == Σ stakes + emission.
    let dust = ctx.token_balance(vault);
    assert_eq!(
        total_claimed + dust,
        vault_initial,
        "Σ claims + dust == Σ stakes + emission"
    );
    assert!(dust <= emission, "dust ≤ emission (floor remainder)");
}

// ---------------------------------------------------------------------------
// Test 4 — emission ENABLED, InvalidDeadend: real finalize BURNS the emission
// back (supply returns), every staker reclaims full stake. (Dispute SEEDED;
// finalize_oracle burn + claims REAL.)
// ---------------------------------------------------------------------------

#[test]
fn e2e_invalid_deadend_emission_burned_full_returns() {
    let mut ctx = TestCtx::new();
    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: 1_500,
        },
        ProposerSpec {
            option: 1,
            bond: 2_500,
        },
    ]);
    let pdas: Vec<Pubkey> = ctx.proposers(oracle).iter().map(|p| p.pda).collect();
    let auths: Vec<Keypair> = ctx
        .proposers(oracle)
        .iter()
        .map(|p| p.authority.insecure_clone())
        .collect();
    ctx.set_proposer_claim_option(pdas[0], 0);
    ctx.set_proposer_claim_option(pdas[1], 1); // tie → InvalidDeadend
    ctx.set_phase(oracle, Phase::Challenge);

    let emission = 777u64;
    ctx.set_reward_emission(oracle, emission);

    let vault = ctx.seeded(oracle).stake_vault;
    let nonce = ctx.seeded(oracle).nonce;
    let supply_before = ctx.mint_supply(ctx.kass_mint);
    assert_eq!(ctx.token_balance(vault), 4_000 + emission);

    // REAL finalize_oracle → InvalidDeadend, burning the emission back.
    ctx.warp(WINDOW + 1);
    ctx.send(ctx.finalize_oracle_ix(oracle, &pdas), &[])
        .expect("finalize_oracle");
    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::InvalidDeadend as u8);
    assert_eq!(o.reward_pool, 0);
    assert_eq!(
        ctx.token_balance(vault),
        4_000,
        "emission burned out of the vault"
    );
    assert_eq!(
        ctx.mint_supply(ctx.kass_mint),
        supply_before - emission,
        "burn-back returned the emission to the reservoir"
    );

    // Full returns: every proposer reclaims its whole bond, vault drains to 0.
    let vault_after_burn = ctx.token_balance(vault);
    let mut total_claimed = 0u64;
    for (auth, pda) in auths.iter().zip(&pdas) {
        let bond = ctx.proposer(*pda).bond;
        let dest = ctx.fund_kass(auth, 0);
        let ix = ctx.claim_proposer_ix(oracle, nonce, *pda, dest, vault, auth.pubkey());
        ctx.send(ix, &[]).expect("claim_proposer");
        assert_eq!(ctx.token_balance(dest), bond, "full bond back on dead-end");
        total_claimed += bond;
    }
    assert_eq!(total_claimed, vault_after_burn, "Σ payouts == Σ stakes");
    assert_eq!(ctx.token_balance(vault), 0, "vault fully drained");
}

// ---------------------------------------------------------------------------
// Test 5 — the S3-flagged combination: InvalidDeadend AFTER a settled challenge,
// WITH emission present. Verifies the burn-back + full returns + the forfeit of a
// challenge-disqualified proposer all conserve, plus close_market / close_ai_claim.
// (Dispute + challenge SEEDED to the post-settle state; finalize burn + claims +
// closes REAL.)
// ---------------------------------------------------------------------------

#[test]
fn e2e_deadend_after_settled_challenge_with_emission() {
    let mut ctx = TestCtx::new();
    // Three proposers. P0 was successfully challenged (disqualified, kass_fee left
    // the vault to the challenger). P1/P2 survive but claim DISTINCT options → the
    // surviving plurality ties → InvalidDeadend.
    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: 1_000,
        },
        ProposerSpec {
            option: 0,
            bond: 1_000,
        },
        ProposerSpec {
            option: 1,
            bond: 1_000,
        },
    ]);
    let pdas: Vec<Pubkey> = ctx.proposers(oracle).iter().map(|p| p.pda).collect();
    let auths: Vec<Keypair> = ctx
        .proposers(oracle)
        .iter()
        .map(|p| p.authority.insecure_clone())
        .collect();

    // P0: settled-challenge disqualify. kass_fee = 100 left the vault; bond_pool
    // gains bond − kass_fee == 900; surviving_count drops to 2.
    let kass_fee = 100u64;
    ctx.seed_challenge_disqualify(oracle, pdas[0], kass_fee);

    // Survivors tie: P1 claims 0, P2 claims 1.
    ctx.set_proposer_claim_option(pdas[1], 0);
    ctx.set_proposer_claim_option(pdas[2], 1);
    ctx.set_phase(oracle, Phase::Challenge);

    // Emission present at creation (placed in the vault, backed by supply).
    let emission = 555u64;
    ctx.set_reward_emission(oracle, emission);

    let vault = ctx.seeded(oracle).stake_vault;
    let nonce = ctx.seeded(oracle).nonce;
    let supply_before = ctx.mint_supply(ctx.kass_mint);
    // Vault = Σ bonds (3000) − kass_fee (100) + emission (555).
    let vault_initial = ctx.token_balance(vault);
    assert_eq!(vault_initial, 3_000 - kass_fee + emission);

    // Seed a SETTLED Market + empty escrow + an AiClaim for the disqualified P0,
    // so the closes have something to reclaim.
    let challenger = Keypair::new();
    ctx.airdrop(&challenger, 1_000_000_000);
    let escrow = ctx.seed_usdc_escrow(oracle, 0);
    let market = ctx.seed_market(oracle, challenger.pubkey(), escrow, true);
    let ai_claim = ctx.seed_ai_claim(oracle, pdas[0], auths[0].pubkey());

    // REAL finalize_oracle → InvalidDeadend, burning the emission back.
    ctx.warp(WINDOW + 1);
    ctx.send(ctx.finalize_oracle_ix(oracle, &pdas), &[])
        .expect("finalize_oracle");
    let o = ctx.oracle(oracle);
    assert_eq!(
        o.phase,
        Phase::InvalidDeadend as u8,
        "deadend after settled challenge"
    );
    assert_eq!(o.reward_pool, 0);
    assert_eq!(
        ctx.mint_supply(ctx.kass_mint),
        supply_before - emission,
        "emission burned back even with a settled challenge present"
    );
    let vault_after_burn = ctx.token_balance(vault);
    assert_eq!(
        vault_after_burn,
        3_000 - kass_fee,
        "vault back to Σ bonds − kass_fee"
    );

    // Claims: P0 (disqualified) forfeits (0); P1/P2 reclaim full bonds.
    let mut total_claimed = 0u64;
    for (i, (auth, pda)) in auths.iter().zip(&pdas).enumerate() {
        let p = ctx.proposer(*pda);
        let expected = if p.disqualified != 0 { 0 } else { p.bond };
        if i == 0 {
            assert_eq!(expected, 0, "disqualified P0 forfeits the whole bond");
        }
        let dest = ctx.fund_kass(auth, 0);
        let ix = ctx.claim_proposer_ix(oracle, nonce, *pda, dest, vault, auth.pubkey());
        ctx.send(ix, &[]).expect("claim_proposer");
        assert_eq!(ctx.token_balance(dest), expected);
        assert!(ctx.is_closed(*pda));
        total_claimed += expected;
    }

    // The disqualified P0's `bond − kass_fee` (900) was never distributed (dead-end
    // reward_pool == 0), so it stays as conservation-safe vault dust; the kass_fee
    // (100) had already left to the challenger at settle time.
    let dust = ctx.token_balance(vault);
    assert_eq!(total_claimed, 2_000, "P1 + P2 full bonds");
    assert_eq!(
        dust,
        1_000 - kass_fee,
        "P0's forfeited bond − kass_fee remains as dust"
    );
    // KASS conservation across the WHOLE settled-challenge dead-end:
    //   vault_initial == Σ payouts + dust  (and earlier: kass_fee left, emission burned).
    assert_eq!(
        total_claimed + dust,
        vault_after_burn,
        "Σ payouts + dust == post-burn vault"
    );
    assert_eq!(
        total_claimed + dust + kass_fee + emission,
        3_000 + emission,
        "full KASS accounting: payouts + dust + kass_fee_out + emission_burned == Σ bonds + emission",
    );

    // ---- REAL closes: AiClaim + Market + escrow rent reclamation ---------------
    let p0_auth_before = ctx.lamports(auths[0].pubkey());
    let ai_rent = ctx.lamports(ai_claim);
    ctx.send(
        ctx.close_ai_claim_ix(oracle, ai_claim, auths[0].pubkey()),
        &[],
    )
    .expect("close_ai_claim");
    assert!(ctx.is_closed(ai_claim));
    assert_eq!(
        ctx.lamports(auths[0].pubkey()),
        p0_auth_before + ai_rent,
        "AiClaim rent → authority"
    );

    let chal_before = ctx.lamports(challenger.pubkey());
    let market_rent = ctx.lamports(market);
    let escrow_rent = ctx.lamports(escrow);
    ctx.send(
        ctx.close_market_ix(oracle, nonce, market, escrow, challenger.pubkey()),
        &[],
    )
    .expect("close_market");
    assert!(ctx.is_closed(market));
    assert!(ctx.is_closed(escrow));
    assert_eq!(
        ctx.lamports(challenger.pubkey()),
        chal_before + market_rent + escrow_rent,
        "Market + escrow rents → challenger",
    );
}

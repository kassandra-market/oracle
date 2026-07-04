//! Task S5 — end-to-end STAKER-SETTLEMENT lifecycle tests.
//!
//! These drive a full lifecycle to a terminal state and then have EVERY staker
//! claim (the S2 `claim_proposer` / `claim_fact` / `claim_fact_vote`) and every
//! account close (the S4 `close_ai_claim` / `close_market`), asserting the
//! per-actor matrix, that all accounts close, that the stake vault drains to
//! floor dust, and KASS conservation sourced ONLY from the stake vault.
//!
//! # What is REAL vs SEEDED (honest split)
//! * **Tests 1-2** drive the dispute core through the GENUINE front door —
//!   `create_oracle → propose×2 (conflict) → finalize_proposals → submit_fact →
//!   advance_phase → vote_fact → finalize_facts → submit_ai_claim×2 →
//!   finalize_ai_claims → finalize_oracle` (no `set_phase`; only `warp` moves
//!   time) — then run REAL claims + closes. Everything is a real instruction,
//!   including the creation-time emission the real `create_oracle` MINTS into the
//!   vault (ON by default): test 1 (Resolved) folds it into `reward_pool`, test 2
//!   (InvalidDeadend) burns it back.
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
    config::{FACT_VOTE_SLASH_DEN, FACT_VOTE_SLASH_NUM, PHASE_WINDOW},
    reward,
    state::{Phase, CLAIM_OPTION_NONE, VOTE_APPROVE},
};
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

// ---------------------------------------------------------------------------
// Dispute-core instruction builders (mirror lifecycle_e2e.rs / challenge_e2e.rs)
// ---------------------------------------------------------------------------

fn finalize_facts_ix(ctx: &TestCtx, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
    ctx.finalize_facts_ix(oracle, tail)
}

/// AiClaim PDA seeds `[b"claim", oracle, proposer]`.
fn claim_pda(program_id: &Pubkey, oracle: &Pubkey, proposer: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"claim", oracle.as_ref(), proposer.as_ref()], program_id)
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
/// vs a tie → InvalidDeadend). The real `create_oracle` mints the creation-time
/// emission into the vault (ON by default); the terminal `finalize_oracle` then
/// folds it into `reward_pool` (Resolved) or burns it back (InvalidDeadend).
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
    // Emission is ON by default: the real create_oracle minted a `reward_emission`
    // into the vault, and the Resolved finalize folds it ON TOP of the physical
    // bond_pool (the flip slash on P1). Both terms are positive, and the vault
    // still holds Σ stakes + the emission (it is distributed via the claims below).
    assert!(
        o.reward_emission > 0,
        "genesis create minted a real emission"
    );
    assert_eq!(
        o.reward_pool,
        o.bond_pool + o.reward_emission,
        "reward_pool == bond_pool + folded emission"
    );
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
// WITH emission present. Verifies the burn-back of BOTH emission AND the slashed
// bond_pool (with NO double-count of the kass_fee already paid out at settle) +
// full survivor returns + the forfeit of the challenge-disqualified proposer all
// conserve and FULLY DRAIN the vault (no stranding), plus close_market /
// close_ai_claim. (Dispute + challenge SEEDED to the post-settle state; finalize
// burn + claims + closes REAL.)
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
    // P0's settled-challenge slash sits in bond_pool (= bond − kass_fee == 900).
    let bond_pool = ctx.oracle(oracle).bond_pool;
    assert_eq!(bond_pool, 1_000 - kass_fee, "P0 slash in bond_pool");
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

    // REAL finalize_oracle → InvalidDeadend, burning BOTH the emission AND the
    // slashed bond_pool (P0's 900) back. Crucially NO double-count: the kass_fee
    // (100) already left the vault to the challenger at settle time and was
    // recorded as `bond − kass_fee` in bond_pool, so burning bond_pool burns only
    // the 900 still physically in the vault.
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
        supply_before - emission - bond_pool,
        "emission AND slashed bond_pool burned back (no double-count of the kass_fee)"
    );
    let vault_after_burn = ctx.token_balance(vault);
    assert_eq!(
        vault_after_burn,
        3_000 - kass_fee - bond_pool,
        "vault = Σ bonds − kass_fee_out − burned bond_pool == survivors' returnable principal"
    );
    assert_eq!(
        vault_after_burn, 2_000,
        "exactly P1 + P2's returnable bonds"
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

    // The disqualified P0's `bond − kass_fee` (900) was BURNED (it funded the now-
    // burned bond_pool), so unlike before it is NOT stranded as dust: the vault
    // fully drains to 0. The kass_fee (100) had already left to the challenger at
    // settle time.
    let dust = ctx.token_balance(vault);
    assert_eq!(total_claimed, 2_000, "P1 + P2 full bonds");
    assert_eq!(
        dust, 0,
        "no stranding: P0's forfeited bond_pool was burned, vault drained"
    );
    // KASS conservation across the WHOLE settled-challenge dead-end:
    //   vault_after_burn == Σ payouts + dust.
    assert_eq!(
        total_claimed + dust,
        vault_after_burn,
        "Σ payouts + dust == post-burn vault"
    );
    assert_eq!(
        total_claimed + dust + kass_fee + emission + bond_pool,
        3_000 + emission,
        "full KASS accounting: payouts + dust + kass_fee_out + emission_burned + bond_pool_burned == Σ bonds + emission",
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

// ===========================================================================
// Tests 6-7 (DS2) — end-to-end FACT/VOTE dead-end conservation, REAL DRIVEN.
// ===========================================================================
//
// The DS1 conservation arms (deadend_settlement.rs + invariants Arm F) proved
// the dead-end burn over PROPOSER slashes (challenge-disqualify / flip). The
// FACT/VOTE dead-end claim path — a REJECTED fact (its submitter stake → the
// burned bond_pool) AND a SLASHED approve-voter on that rejected fact (the
// slashed fraction → the burned bond_pool), alongside an AGREED fact (returned)
// — was only validated by the harness mirror in Arm E (a seeded post-burn
// vault), NOT by a real on-chain finalize-burn-then-claim.
//
// These two tests close that gap. They DRIVE a real dispute through the genuine
// front door (create_oracle → propose → finalize_proposals → submit_fact×2 →
// advance_phase → vote_fact×2 → finalize_facts → submit_ai_claim×2 →
// finalize_ai_claims → finalize_oracle; only `warp` moves time) to a Tie
// dead-end that carries:
//   * an AGREED fact (submitter + approve-voter both reclaim full stake),
//   * a REJECTED fact whose submitter forfeits (→ 0) and whose lone approve-voter
//     is slashed (the on-chain rejected-fact voter slash, fact_vote_slash 1/2),
// then assert the REAL `finalize_oracle` burned the slashed `bond_pool` (= the
// rejected submitter stake + the FLOOR aggregate voter slash) + the emission, and
// the REAL S2 claims pay the per-actor matrix and drain the vault to bounded dust.
//
// The rejected-fact approve stake is ODD (501) ON PURPOSE: `finalize_facts`
// credits `bond_pool` with the FLOOR aggregate `floor(501·1/2) = 250`, but
// `claim_fact_vote` slashes the lone voter `ceil(501·1/2) = 251`. That floor-vs-
// ceil asymmetry is the conservation safety margin — the vault retains exactly
// `ceil − floor = 1` as conservation-SAFE dust (never short). This proves the
// rounding choice end-to-end on a real driven path, not just by the argument in
// `claims::slash_amount`. Test 6 is a plain InvalidDeadend; test 7 is the
// governance-resolved (`resolve_deadend` → Resolved) path, which MUST pay
// IDENTICALLY (the no-marker insight, here on the fact/vote path).

/// Bonds / stakes for the driven fact/vote dead-end (chosen so the rejected fact
/// stays below the 2/3 quorum and the rejected approve stake is ODD).
const FVD_BOND: u64 = 1_000;
const FVD_AGREED_SUB: u64 = 400;
const FVD_AGREED_VOTE: u64 = 2_000;
const FVD_REJECTED_SUB: u64 = 300;
const FVD_REJECTED_VOTE: u64 = 501; // odd → exercises the ceil-vs-floor margin

/// Everything a fact/vote dead-end settlement test needs: the oracle handles, the
/// two proposers, both facts (+ their submitter/voter), and the pre-burn snapshots
/// (`supply_before` / `bond_pool` / `emission` / `sum_stakes`) needed to assert
/// the burn delta + full conservation equation.
struct DrivenFactVoteDeadend {
    oracle: Pubkey,
    nonce: u64,
    vault: Pubkey,
    proposers: Vec<(Keypair, Pubkey)>,
    agreed_fact: Pubkey,
    agreed_submitter: Keypair,
    agreed_vote: Pubkey,
    agreed_voter: Keypair,
    rejected_fact: Pubkey,
    rejected_submitter: Keypair,
    rejected_vote: Pubkey,
    rejected_voter: Keypair,
    emission: u64,
    bond_pool: u64,
    supply_before: u64,
    sum_stakes: u64,
}

/// `ceil(value · num / den)` — the per-voter rejected-fact slash (mirrors
/// `claims::slash_amount`).
fn ceil_slash(value: u64, num: u64, den: u64) -> u64 {
    (value as u128 * num as u128).div_ceil(den as u128) as u64
}

/// `floor(value · num / den)` — the aggregate bond_pool credit (mirrors
/// `finalize_facts`'s rejected-fact voter slash accumulation).
fn floor_slash(value: u64, num: u64, den: u64) -> u64 {
    (value as u128 * num as u128 / den as u128) as u64
}

/// Drive a REAL dispute to a Tie dead-end carrying one AGREED + one REJECTED fact
/// (the rejected fact has a SLASHED approve-voter). The creation-time emission
/// (minted by the real `create_oracle`, ON by default) sits in the vault and is
/// burned by the terminal `finalize_oracle`. Snapshots supply + bond_pool just
/// before the burn. The oracle ends in InvalidDeadend.
fn drive_real_fact_vote_deadend(ctx: &mut TestCtx) -> DrivenFactVoteDeadend {
    // create_oracle → propose×2 (DISTINCT options 0/1) → finalize_proposals.
    let oracle = ctx.dispute_via_real_flow(&[
        ProposerSpec {
            option: 0,
            bond: FVD_BOND,
        },
        ProposerSpec {
            option: 1,
            bond: FVD_BOND,
        },
    ]);
    let (vault, _) = TestCtx::stake_vault_pda(&ctx.program_id, &oracle);
    let nonce = ctx.seeded(oracle).nonce;
    let proposer_pdas: Vec<Pubkey> = ctx.proposers(oracle).iter().map(|p| p.pda).collect();
    let authorities: Vec<Keypair> = ctx
        .proposers(oracle)
        .iter()
        .map(|p| p.authority.insecure_clone())
        .collect();
    // dispute_bond_total == 2*FVD_BOND == 2000 → 2/3 quorum == 1334.

    // ---- submit_fact ×2 (FactProposal window) ------------------------------
    let agreed_submitter = Keypair::new();
    ctx.svm
        .airdrop(&agreed_submitter.pubkey(), 1_000_000_000)
        .unwrap();
    let agreed_sub_kass = ctx.fund_kass(&agreed_submitter, FVD_AGREED_SUB);
    let agreed_hash = [0x07u8; 32];
    let (agreed_fact, _) = TestCtx::fact_pda(&ctx.program_id, &oracle, &agreed_hash);
    ctx.send(
        submit_fact_ix(
            ctx,
            oracle,
            agreed_fact,
            agreed_submitter.pubkey(),
            agreed_sub_kass,
            vault,
            submit_fact_payload(&agreed_hash, FVD_AGREED_SUB, b"ipfs://agreed"),
        ),
        &[&agreed_submitter],
    )
    .expect("submit_fact (agreed)");

    let rejected_submitter = Keypair::new();
    ctx.svm
        .airdrop(&rejected_submitter.pubkey(), 1_000_000_000)
        .unwrap();
    let rejected_sub_kass = ctx.fund_kass(&rejected_submitter, FVD_REJECTED_SUB);
    let rejected_hash = [0x09u8; 32];
    let (rejected_fact, _) = TestCtx::fact_pda(&ctx.program_id, &oracle, &rejected_hash);
    ctx.send(
        submit_fact_ix(
            ctx,
            oracle,
            rejected_fact,
            rejected_submitter.pubkey(),
            rejected_sub_kass,
            vault,
            submit_fact_payload(&rejected_hash, FVD_REJECTED_SUB, b"ipfs://rejected"),
        ),
        &[&rejected_submitter],
    )
    .expect("submit_fact (rejected)");

    // ---- advance to FactVoting ---------------------------------------------
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(advance_phase_ix(ctx, oracle), &[])
        .expect("advance_phase");

    // ---- vote_fact ×2 (FactVoting window) ----------------------------------
    // Agreed fact: approve 2000 (>= 1334 quorum) → agreed.
    let agreed_voter = Keypair::new();
    ctx.svm
        .airdrop(&agreed_voter.pubkey(), 1_000_000_000)
        .unwrap();
    let agreed_voter_kass = ctx.fund_kass(&agreed_voter, FVD_AGREED_VOTE);
    let (agreed_vote, _) = TestCtx::vote_pda(&ctx.program_id, &agreed_fact, &agreed_voter.pubkey());
    ctx.send(
        vote_fact_ix(
            ctx,
            oracle,
            agreed_fact,
            agreed_vote,
            agreed_voter.pubkey(),
            agreed_voter_kass,
            vault,
            vote_payload(VOTE_APPROVE, FVD_AGREED_VOTE),
        ),
        &[&agreed_voter],
    )
    .expect("vote_fact (agreed)");

    // Rejected fact: approve 501 (> duplicate 0 but < 1334 quorum) → rejected;
    // this lone approve-voter is slashed `ceil(501·1/2) = 251` at claim.
    let rejected_voter = Keypair::new();
    ctx.svm
        .airdrop(&rejected_voter.pubkey(), 1_000_000_000)
        .unwrap();
    let rejected_voter_kass = ctx.fund_kass(&rejected_voter, FVD_REJECTED_VOTE);
    let (rejected_vote, _) =
        TestCtx::vote_pda(&ctx.program_id, &rejected_fact, &rejected_voter.pubkey());
    ctx.send(
        vote_fact_ix(
            ctx,
            oracle,
            rejected_fact,
            rejected_vote,
            rejected_voter.pubkey(),
            rejected_voter_kass,
            vault,
            vote_payload(VOTE_APPROVE, FVD_REJECTED_VOTE),
        ),
        &[&rejected_voter],
    )
    .expect("vote_fact (rejected)");

    // ---- finalize_facts → AiClaim ------------------------------------------
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(
        finalize_facts_ix(ctx, oracle, &[agreed_fact, rejected_fact]),
        &[],
    )
    .expect("finalize_facts");
    assert_eq!(
        ctx.fact(agreed_fact).agreed,
        1,
        "agreed fact cleared quorum"
    );
    let rf = ctx.fact(rejected_fact);
    assert_eq!(rf.agreed, 0, "rejected fact not agreed");
    assert_eq!(rf.duplicate, 0, "rejected fact not duplicate-dominant");

    // bond_pool == rejected submitter stake + FLOOR aggregate voter slash.
    let expected_bond_pool =
        FVD_REJECTED_SUB + floor_slash(FVD_REJECTED_VOTE, FACT_VOTE_SLASH_NUM, FACT_VOTE_SLASH_DEN);
    assert_eq!(
        ctx.oracle(oracle).bond_pool,
        expected_bond_pool,
        "bond_pool = rejected submitter stake + floor(approve·1/2)"
    );

    // ---- submit_ai_claim ×2 (DISTINCT options 0/1 → tie) -------------------
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
                submit_ai_payload(i as u8),
            ),
            &[auth],
        )
        .expect("submit_ai_claim");
    }

    // ---- finalize_ai_claims → Challenge ------------------------------------
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(finalize_ai_claims_ix(ctx, oracle, &proposer_pdas), &[])
        .expect("finalize_ai_claims");
    assert_eq!(ctx.oracle(oracle).phase, Phase::Challenge.as_u8());

    // ---- the creation-time emission is ALREADY in the vault (emission is ON by
    // default: the real create_oracle minted it), so capture the REAL amount
    // rather than seeding one. Then snapshot supply + bond_pool before the burn.
    let emission = ctx.oracle(oracle).reward_emission;
    assert!(emission > 0, "genesis create minted a real emission");
    let sum_stakes =
        2 * FVD_BOND + FVD_AGREED_SUB + FVD_AGREED_VOTE + FVD_REJECTED_SUB + FVD_REJECTED_VOTE;
    assert_eq!(
        ctx.token_balance(vault),
        sum_stakes + emission,
        "vault holds Σ stakes + emission before the terminal burn"
    );
    let supply_before = ctx.mint_supply(ctx.kass_mint);
    let bond_pool = ctx.oracle(oracle).bond_pool;

    // ---- finalize_oracle → InvalidDeadend (burns bond_pool + emission) -----
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(ctx.finalize_oracle_ix(oracle, &proposer_pdas), &[])
        .expect("finalize_oracle");
    assert_eq!(ctx.oracle(oracle).phase, Phase::InvalidDeadend as u8);

    let proposers = authorities.into_iter().zip(proposer_pdas).collect();
    DrivenFactVoteDeadend {
        oracle,
        nonce,
        vault,
        proposers,
        agreed_fact,
        agreed_submitter,
        agreed_vote,
        agreed_voter,
        rejected_fact,
        rejected_submitter,
        rejected_vote,
        rejected_voter,
        emission,
        bond_pool,
        supply_before,
        sum_stakes,
    }
}

/// Run the full S2 claim sweep over a driven fact/vote dead-end and assert the
/// per-actor matrix + the floor-vs-ceil conservation-safe dust + the full
/// conservation equation. Works for BOTH terminal phases (plain InvalidDeadend
/// and governance-resolved Resolved) — the payouts are identical because
/// `reward_pool == 0` on both.
fn assert_fact_vote_deadend_drains(ctx: &mut TestCtx, d: &DrivenFactVoteDeadend) {
    let o = ctx.oracle(d.oracle);
    assert_eq!(o.reward_pool, 0, "no reward distribution out of a dead-end");

    // The slashed bond_pool + the emission were BURNED back to the reservoir.
    assert_eq!(
        ctx.mint_supply(ctx.kass_mint),
        d.supply_before - d.bond_pool - d.emission,
        "bond_pool (rejected stake + voter floor-slash) + emission burned"
    );
    // Post-burn vault == Σ stakes − bond_pool (the returnable non-slashed
    // principal; the emission was burned too, netting to Σ stakes − bond_pool).
    let vault_after_burn = ctx.token_balance(d.vault);
    assert_eq!(vault_after_burn, d.sum_stakes - d.bond_pool);

    let mut returned = 0u64;

    // --- AGREED fact: approve-voter then submitter both reclaim full stake ---
    let av = ctx.fact_vote(d.agreed_vote);
    let dest = ctx.fund_kass(&d.agreed_voter, 0);
    ctx.send(
        ctx.claim_fact_vote_ix(
            d.oracle,
            d.nonce,
            d.agreed_vote,
            d.agreed_fact,
            dest,
            d.vault,
            d.agreed_voter.pubkey(),
        ),
        &[],
    )
    .expect("claim_fact_vote (agreed)");
    assert_eq!(
        ctx.token_balance(dest),
        av.stake,
        "agreed approve-voter: full stake (no reward, dead-end)"
    );
    returned += av.stake;

    let af = ctx.fact(d.agreed_fact);
    let dest = ctx.fund_kass(&d.agreed_submitter, 0);
    ctx.send(
        ctx.claim_fact_ix(
            d.oracle,
            d.nonce,
            d.agreed_fact,
            dest,
            d.vault,
            d.agreed_submitter.pubkey(),
        ),
        &[],
    )
    .expect("claim_fact (agreed)");
    assert_eq!(
        ctx.token_balance(dest),
        af.stake,
        "agreed submitter: full stake (no reward, dead-end)"
    );
    assert!(ctx.is_closed(d.agreed_fact));
    returned += af.stake;

    // --- REJECTED fact: voter slashed ceil; submitter forfeits to 0 ----------
    let rv = ctx.fact_vote(d.rejected_vote);
    let voter_slash = ceil_slash(rv.stake, FACT_VOTE_SLASH_NUM, FACT_VOTE_SLASH_DEN);
    let dest = ctx.fund_kass(&d.rejected_voter, 0);
    ctx.send(
        ctx.claim_fact_vote_ix(
            d.oracle,
            d.nonce,
            d.rejected_vote,
            d.rejected_fact,
            dest,
            d.vault,
            d.rejected_voter.pubkey(),
        ),
        &[],
    )
    .expect("claim_fact_vote (rejected)");
    assert_eq!(
        ctx.token_balance(dest),
        rv.stake - voter_slash,
        "rejected approve-voter: stake − ceil(stake·1/2)"
    );
    returned += rv.stake - voter_slash;

    let dest = ctx.fund_kass(&d.rejected_submitter, 0);
    ctx.send(
        ctx.claim_fact_ix(
            d.oracle,
            d.nonce,
            d.rejected_fact,
            dest,
            d.vault,
            d.rejected_submitter.pubkey(),
        ),
        &[],
    )
    .expect("claim_fact (rejected)");
    assert_eq!(
        ctx.token_balance(dest),
        0,
        "rejected submitter forfeits (stake funded the burned bond_pool)"
    );
    assert!(ctx.is_closed(d.rejected_fact), "rejected fact still closes");
    // returned += 0;

    // --- proposers: bond − slashed_amount (no flip slash here → full bond) ---
    for (auth, pda) in &d.proposers {
        let p = ctx.proposer(*pda);
        let expected = p.bond - p.slashed_amount;
        let dest = ctx.fund_kass(auth, 0);
        ctx.send(
            ctx.claim_proposer_ix(d.oracle, d.nonce, *pda, dest, d.vault, auth.pubkey()),
            &[],
        )
        .expect("claim_proposer");
        assert_eq!(
            ctx.token_balance(dest),
            expected,
            "survivor: bond − slashed"
        );
        assert!(ctx.is_closed(*pda));
        returned += expected;
    }

    // --- conservation: dust is exactly the floor-vs-ceil voter-slash margin --
    let dust = ctx.token_balance(d.vault);
    let ceil_margin = voter_slash - floor_slash(rv.stake, FACT_VOTE_SLASH_NUM, FACT_VOTE_SLASH_DEN);
    assert_eq!(
        returned + dust,
        vault_after_burn,
        "Σ returned + dust == post-burn vault"
    );
    assert_eq!(
        dust, ceil_margin,
        "dust == ceil(stake·1/2) − floor(stake·1/2) (conservation-safe, never short)"
    );
    // Full equation: Σ returned + dust + bond_pool_burned + emission_burned
    //   == Σ stakes + emission.
    assert_eq!(
        returned + dust + d.bond_pool + d.emission,
        d.sum_stakes + d.emission,
        "Σ returned + dust + bond_pool_burned + emission_burned == Σ stakes + emission"
    );
}

// ---------------------------------------------------------------------------
// Test 6 — plain InvalidDeadend, real fact/vote dead-end conservation.
// ---------------------------------------------------------------------------

#[test]
fn e2e_fact_vote_deadend_burns_and_drains_real_dispute() {
    let mut ctx = TestCtx::new();
    let d = drive_real_fact_vote_deadend(&mut ctx);
    assert_eq!(
        ctx.oracle(d.oracle).resolved_option,
        CLAIM_OPTION_NONE,
        "plain dead-end carries the sentinel"
    );
    assert_fact_vote_deadend_drains(&mut ctx, &d);
}

// ---------------------------------------------------------------------------
// Test 7 — governance-resolved (resolve_deadend → Resolved) fact/vote dead-end:
// pays IDENTICALLY to the plain InvalidDeadend (reward_pool == 0 ⇒ no rewards),
// drains the same, no marker / claim-path divergence on the fact/vote path.
// ---------------------------------------------------------------------------

#[test]
fn e2e_fact_vote_deadend_governance_resolved_pays_identically() {
    let mut ctx = TestCtx::new();
    let d = drive_real_fact_vote_deadend(&mut ctx);

    // Governance force-resolves the dead-end to option 1 → Resolved (the burn
    // already happened at finalize; resolve_deadend moves no tokens).
    let dao = Keypair::new();
    ctx.airdrop(&dao, 1_000_000_000);
    let (_da, kass_dao) = TestCtx::stand_in_governance(0x44);
    ctx.force_governance(dao.pubkey(), kass_dao);
    let (_p, res) = ctx.resolve_deadend(d.oracle, &dao, 1);
    assert!(res.is_ok(), "resolve_deadend should succeed: {res:?}");
    let o = ctx.oracle(d.oracle);
    assert_eq!(o.phase, Phase::Resolved as u8, "phase flipped to Resolved");
    assert_eq!(o.resolved_option, 1, "governance option recorded");
    assert_eq!(
        o.reward_pool, 0,
        "still no reward pool on a resolved-from-dead-end"
    );

    // Same claim sweep, same payouts, same dust — proving the no-marker insight
    // holds on the FACT/VOTE dead-end path too.
    assert_fact_vote_deadend_drains(&mut ctx, &d);
}

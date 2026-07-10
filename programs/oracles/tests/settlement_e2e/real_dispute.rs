use super::*;

use kassandra_oracles_program::state::CLAIM_OPTION_NONE;

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

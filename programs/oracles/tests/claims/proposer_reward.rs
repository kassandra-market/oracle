use super::*;

use solana_signer::Signer;

#[test]
fn flipped_survivor_not_overpaid() {
    let mut ctx = TestCtx::new();
    // resolved_option == 1. No facts, so the fact bucket rolls into the proposer
    // cohort: reward_pool = Σ slashed = 500 + 500 = 1000 = proposer_bucket;
    // total_correct = 1000 + 1000 = 2000 (both correct survivors weigh by bond).
    let proposers = vec![
        // honest correct survivor: 1000 − 0 + 500 = 1500
        ClaimProposerSpec {
            bond: 1_000,
            claim_option: 1,
            disqualified: false,
            slashed_amount: 0,
        },
        // FLIP-slashed but SURVIVING + correct: 1000 − 500 + 500 = 1000 (NOT 1500)
        ClaimProposerSpec {
            bond: 1_000,
            claim_option: 1,
            disqualified: false,
            slashed_amount: 500,
        },
        // FLIP-slashed but SURVIVING + wrong: 1000 − 500 + 0 = 500 (no reward)
        ClaimProposerSpec {
            bond: 1_000,
            claim_option: 0,
            disqualified: false,
            slashed_amount: 500,
        },
    ];
    let seed = ctx.seed_terminal_oracle(Phase::Resolved, 1, &proposers, &[], 1, 2);

    let expects: Vec<u64> = seed.proposers.iter().map(|p| p.expected).collect();
    assert_eq!(
        expects,
        vec![1_500, 1_000, 500],
        "flip-slashed survivors get bond − slashed_amount (+reward iff correct)"
    );
    assert_eq!(seed.reward_pool, 1_000);

    let mut sum = 0u64;
    for i in 0..seed.proposers.len() {
        let p = &seed.proposers[i];
        sum += p.expected;
        let ix = ctx.claim_proposer_ix(
            seed.oracle,
            seed.nonce,
            p.account,
            p.dest_kass,
            seed.stake_vault,
            p.authority.pubkey(),
        );
        let (account, dest, recip, expected) =
            (p.account, p.dest_kass, p.authority.pubkey(), p.expected);
        assert_claim(
            &mut ctx,
            ix,
            account,
            dest,
            seed.stake_vault,
            recip,
            expected,
        );
    }
    // Conservation holds WITH flipped survivors present: Σ + dust == vault.
    let dust = ctx.token_balance(seed.stake_vault);
    assert_eq!(sum + dust, seed.vault_initial);
    assert_eq!(dust, 0, "exact split here — no floor dust");
}

#[test]
fn disqualified_forfeits_full_bond() {
    // C1: a CHALLENGE-disqualified proposer has `slashed_amount = bond − kass_fee`
    // (< bond), and `settle_challenge` already paid `kass_fee` out of the vault to
    // the challenger. The claim must pay the fraudster 0 (forfeit the WHOLE bond),
    // NOT `bond − slashed_amount == kass_fee` (which is already gone → would short
    // the vault).
    let mut ctx = TestCtx::new();
    let proposers = vec![
        // correct survivor (so the oracle has a real winner + a reward cohort)
        ClaimProposerSpec {
            bond: 1_000,
            claim_option: 1,
            disqualified: false,
            slashed_amount: 0,
        },
        // challenge-disqualified: bond 1000, kass_fee 100 ⇒ slashed_amount 900 (<bond)
        ClaimProposerSpec {
            bond: 1_000,
            claim_option: 0,
            disqualified: true,
            slashed_amount: 900,
        },
    ];
    let seed = ctx.seed_terminal_oracle(Phase::Resolved, 1, &proposers, &[], 1, 2);
    assert_eq!(
        seed.proposers[1].expected, 0,
        "disqualified forfeits the FULL bond (0), not bond − slashed_amount"
    );

    for i in 0..seed.proposers.len() {
        let p = &seed.proposers[i];
        let ix = ctx.claim_proposer_ix(
            seed.oracle,
            seed.nonce,
            p.account,
            p.dest_kass,
            seed.stake_vault,
            p.authority.pubkey(),
        );
        let (account, dest, recip, expected) =
            (p.account, p.dest_kass, p.authority.pubkey(), p.expected);
        assert_claim(
            &mut ctx,
            ix,
            account,
            dest,
            seed.stake_vault,
            recip,
            expected,
        );
    }
    // The 100 kass_fee that (in the real flow) left the vault to the challenger is
    // here modeled as conservation-safe leftover dust — never over-paid.
    let dust = ctx.token_balance(seed.stake_vault);
    assert_eq!(
        dust, 100,
        "the kass_fee remains as dust, never paid to the fraudster"
    );
}

#[test]
fn ceil_voter_slash_no_shortfall() {
    // I1: a rejected fact with ODD approve stakes where floor(Σ·r) ≠ Σ floor(stakeᵢ·r).
    // bond_pool was credited floor(1002·1/2)=501; with FLOOR per-voter the vault
    // would retain only 500 → the reward claimant (claimed LAST) would come up 1
    // short. CEIL per-voter retains 502 ≥ 501, so every claim succeeds.
    let mut ctx = TestCtx::new();
    let proposers = vec![ClaimProposerSpec {
        bond: 1_000,
        claim_option: 1,
        disqualified: false,
        slashed_amount: 0,
    }];
    let facts = vec![ClaimFactSpec {
        stake: 100,
        agreed: false,
        duplicate: false,
        votes: vec![
            ClaimVoteSpec {
                stake: 401,
                kind: VOTE_APPROVE,
            },
            ClaimVoteSpec {
                stake: 601,
                kind: VOTE_APPROVE,
            },
        ],
    }];
    let seed = ctx.seed_terminal_oracle(Phase::Resolved, 1, &proposers, &facts, 1, 2);
    // reward_pool = 100 (rejected stake) + floor(1002/2)=501 = 601.
    assert_eq!(seed.reward_pool, 601);
    // voter returns: 401 − ceil(401/2)=201 ⇒ 200; 601 − ceil(601/2)=301 ⇒ 300.
    let vote_expects: Vec<u64> = seed.facts[0].votes.iter().map(|v| v.expected).collect();
    assert_eq!(vote_expects, vec![200, 300]);
    // correct proposer: 1000 + proposer_reward(1000, 601, 1000) = 1601.
    assert_eq!(seed.proposers[0].expected, 1_601);

    let fact_account = seed.facts[0].submitter.account;
    // Claim voters + submitter FIRST, then the reward claimant (proposer) LAST so
    // a vault shortfall would surface as its claim failing.
    for vi in 0..seed.facts[0].votes.len() {
        let v = &seed.facts[0].votes[vi];
        let ix = ctx.claim_fact_vote_ix(
            seed.oracle,
            seed.nonce,
            v.account,
            fact_account,
            v.dest_kass,
            seed.stake_vault,
            v.authority.pubkey(),
        );
        assert!(ctx.send(ix, &[]).is_ok(), "voter claim should succeed");
    }
    let s = &seed.facts[0].submitter;
    let ix = ctx.claim_fact_ix(
        seed.oracle,
        seed.nonce,
        s.account,
        s.dest_kass,
        seed.stake_vault,
        s.authority.pubkey(),
    );
    assert!(
        ctx.send(ix, &[]).is_ok(),
        "rejected submitter claim should succeed"
    );

    let p = &seed.proposers[0];
    let ix = ctx.claim_proposer_ix(
        seed.oracle,
        seed.nonce,
        p.account,
        p.dest_kass,
        seed.stake_vault,
        p.authority.pubkey(),
    );
    let res = ctx.send(ix, &[]);
    assert!(
        res.is_ok(),
        "reward claimant must NOT be short-changed (ceil slash keeps the vault solvent): {res:?}"
    );
    assert_eq!(ctx.token_balance(p.dest_kass), 1_601);
    // 1 unit of conservation-safe dust remains (ceil excess + reward floor).
    assert_eq!(ctx.token_balance(seed.stake_vault), 1);
}

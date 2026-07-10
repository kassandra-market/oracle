use super::*;

use solana_signer::Signer;

#[test]
fn invalid_deadend_returns_nonslashed_principal() {
    let mut ctx = TestCtx::new();
    // On InvalidDeadend the slashed pool (rejected submitter stake + approve-voter
    // slash) was BURNED out of the vault at finalize, so claims pay only the
    // NON-SLASHED principal: survivors get their bond, agreed/duplicate stakers
    // get their stake, but the REJECTED submitter forfeits (0) and the rejected-
    // fact approve-voter reclaims only `stake − slash`. reward_pool is 0 (no
    // reward on either path). The post-burn vault drains to exactly 0.
    let proposers = vec![
        ClaimProposerSpec {
            bond: 1_000,
            claim_option: 0,
            disqualified: false,
            slashed_amount: 0,
        },
        ClaimProposerSpec {
            bond: 2_000,
            claim_option: 1,
            disqualified: false,
            slashed_amount: 0,
        },
    ];
    let facts = vec![ClaimFactSpec {
        stake: 1_500,
        agreed: false,
        duplicate: false,
        votes: vec![
            ClaimVoteSpec {
                stake: 700,
                kind: VOTE_APPROVE,
            },
            ClaimVoteSpec {
                stake: 300,
                kind: VOTE_DUPLICATE,
            },
        ],
    }];
    let seed = ctx.seed_terminal_oracle(Phase::InvalidDeadend, 0xFF, &proposers, &facts, 1, 2);
    assert_eq!(seed.reward_pool, 0);

    let mut sum: u64 = 0;
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
    for fi in 0..seed.facts.len() {
        let fact_account = seed.facts[fi].submitter.account;
        for vi in 0..seed.facts[fi].votes.len() {
            let v = &seed.facts[fi].votes[vi];
            sum += v.expected;
            let ix = ctx.claim_fact_vote_ix(
                seed.oracle,
                seed.nonce,
                v.account,
                fact_account,
                v.dest_kass,
                seed.stake_vault,
                v.authority.pubkey(),
            );
            let (account, dest, recip, expected) =
                (v.account, v.dest_kass, v.authority.pubkey(), v.expected);
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

        let s = &seed.facts[fi].submitter;
        sum += s.expected;
        let ix = ctx.claim_fact_ix(
            seed.oracle,
            seed.nonce,
            s.account,
            s.dest_kass,
            seed.stake_vault,
            s.authority.pubkey(),
        );
        let (account, dest, recip, expected) =
            (s.account, s.dest_kass, s.authority.pubkey(), s.expected);
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
    // On InvalidDeadend Σ payouts == the post-burn vault exactly: the non-slashed
    // principal is returned and the burned slashed pool is gone (no stranding).
    assert_eq!(
        sum, seed.vault_initial,
        "non-slashed principal drains the post-burn vault exactly"
    );
    assert_eq!(
        ctx.token_balance(seed.stake_vault),
        0,
        "vault fully drained"
    );
}

#[test]
fn flipped_survivor_invalid_deadend_drains() {
    // M2: a flip-slashed but SURVIVING proposer that ties into InvalidDeadend gets
    // `bond − slashed_amount` (reward_pool == 0). Its flip-slash portion was
    // BURNED out of the vault at finalize, so nothing is stranded — the vault
    // drains to exactly 0 (no more dust).
    let mut ctx = TestCtx::new();
    let proposers = vec![ClaimProposerSpec {
        bond: 1_000,
        claim_option: 0,
        disqualified: false,
        slashed_amount: 400,
    }];
    let seed = ctx.seed_terminal_oracle(Phase::InvalidDeadend, 0xFF, &proposers, &[], 1, 2);
    assert_eq!(seed.reward_pool, 0);
    assert_eq!(
        seed.proposers[0].expected, 600,
        "bond − slashed_amount, no reward"
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
    // The 400 flip-slash portion was burned at finalize, so the vault drains to 0
    // (no stranding — the dead-end-settlement fix).
    assert_eq!(ctx.token_balance(seed.stake_vault), 0);
}

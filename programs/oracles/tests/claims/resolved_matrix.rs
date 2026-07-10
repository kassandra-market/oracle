use super::*;

use solana_signer::Signer;

#[test]
fn resolved_proposer_matrix() {
    let mut ctx = TestCtx::new();
    let seed = resolved_full(&mut ctx);

    // correct → bond + reward; wrong → bond; disqualified → bond − slashed.
    let expects: Vec<u64> = seed.proposers.iter().map(|p| p.expected).collect();
    assert_eq!(expects, vec![2_666, 1_000, 0]);

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
}

#[test]
fn resolved_fact_and_vote_matrix() {
    let mut ctx = TestCtx::new();
    let seed = resolved_full(&mut ctx);

    // agreed submitter 1416, rejected submitter 0, duplicate submitter 1000.
    let sub_expects: Vec<u64> = seed.facts.iter().map(|f| f.submitter.expected).collect();
    assert_eq!(sub_expects, vec![1_416, 0, 1_000]);
    // votes: agreed approve 708/708, rejected approve 200/300, dup 500 / approve 300.
    let vote_expects: Vec<Vec<u64>> = seed
        .facts
        .iter()
        .map(|f| f.votes.iter().map(|v| v.expected).collect())
        .collect();
    assert_eq!(
        vote_expects,
        vec![vec![708, 708], vec![200, 300], vec![500, 300]]
    );

    for fi in 0..seed.facts.len() {
        let fact_account = seed.facts[fi].submitter.account;
        // Votes FIRST (each keeps the Fact alive but decrements its voter total),
        // then the submitter (which closes the Fact).
        for vi in 0..seed.facts[fi].votes.len() {
            let v = &seed.facts[fi].votes[vi];
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
}

#[test]
fn resolved_conservation_sweep() {
    let mut ctx = TestCtx::new();
    let seed = resolved_full(&mut ctx);

    let mut sum: u64 = 0;
    // proposers
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
        assert!(ctx.send(ix, &[]).is_ok());
    }
    // facts + votes (votes first so the submitter can close the fact last)
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
            assert!(ctx.send(ix, &[]).is_ok());
        }
        {
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
            assert!(ctx.send(ix, &[]).is_ok());
        }
    }

    let dust = ctx.token_balance(seed.stake_vault);
    // CONSERVATION: Σ claims + dust == vault_initial; nothing reads total_oracle_stake.
    assert_eq!(sum + dust, seed.vault_initial, "Σ claims + dust == vault");
    // Floor-division dust only (bucket + pro-rata floors); never over-paid.
    assert!(
        dust <= seed.reward_pool,
        "dust is just reward floor remainder"
    );
    assert_eq!(dust, 2, "expected 2 base units of floor dust");
}

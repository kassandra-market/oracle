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

use kassandra_oracles_program::{
    error::KassandraError,
    state::{Phase, VOTE_APPROVE, VOTE_DUPLICATE},
};
use solana_instruction_error::InstructionError;
use solana_signer::Signer;
use solana_transaction_error::TransactionError;

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
fn double_claim_fails_account_gone() {
    let mut ctx = TestCtx::new();
    let seed = resolved_full(&mut ctx);
    let p = &seed.proposers[0];
    let recip = p.authority.pubkey();

    let ix = ctx.claim_proposer_ix(
        seed.oracle,
        seed.nonce,
        p.account,
        p.dest_kass,
        seed.stake_vault,
        recip,
    );
    assert!(ctx.send(ix, &[]).is_ok());
    assert!(ctx.is_closed(p.account));

    // Second claim: the account is gone (zeroed/reaped) → owner/type guard fails.
    let ix = ctx.claim_proposer_ix(
        seed.oracle,
        seed.nonce,
        p.account,
        p.dest_kass,
        seed.stake_vault,
        recip,
    );
    let err = ctx.send(ix, &[]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::InvalidAccount as u32),
        ),
    );
}

#[test]
fn dest_owner_mismatch_rejected() {
    let mut ctx = TestCtx::new();
    let seed = resolved_full(&mut ctx);
    let p = &seed.proposers[0];

    // A KASS account owned by a DIFFERENT party cannot receive the payout.
    let attacker = solana_keypair::Keypair::new();
    let bad_dest = ctx.fund_kass(&attacker, 0);

    let ix = ctx.claim_proposer_ix(
        seed.oracle,
        seed.nonce,
        p.account,
        bad_dest,
        seed.stake_vault,
        p.authority.pubkey(),
    );
    let err = ctx.send(ix, &[]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::InvalidAccount as u32),
        ),
    );
}

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

#[test]
fn submitter_before_voters_rejected() {
    let mut ctx = TestCtx::new();
    let seed = resolved_full(&mut ctx);
    // Fact 0 (agreed) still has two unclaimed approve voters; the submitter's
    // claim must run last (it closes the Fact every voter still reads).
    let s = &seed.facts[0].submitter;
    let ix = ctx.claim_fact_ix(
        seed.oracle,
        seed.nonce,
        s.account,
        s.dest_kass,
        seed.stake_vault,
        s.authority.pubkey(),
    );
    let err = ctx.send(ix, &[]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::VotersOutstanding as u32),
        ),
    );
}

#[test]
fn non_terminal_oracle_rejected() {
    let mut ctx = TestCtx::new();
    // A disputed (FactProposal) oracle is NOT terminal.
    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: 1_000,
        },
        ProposerSpec {
            option: 1,
            bond: 1_000,
        },
    ]);
    let nonce = ctx.seeded(oracle).nonce;
    let stake_vault = ctx.seeded(oracle).stake_vault;
    let pda = ctx.proposers(oracle)[0].pda;
    let authority = ctx.proposers(oracle)[0].authority.insecure_clone();
    let dest = ctx.fund_kass(&authority, 0);

    let ix = ctx.claim_proposer_ix(oracle, nonce, pda, dest, stake_vault, authority.pubkey());
    let err = ctx.send(ix, &[]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::WrongPhase as u32),
        ),
    );
}

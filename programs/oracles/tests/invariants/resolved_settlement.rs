// ===========================================================================
// Arms D + E: PHYSICAL staker-settlement conservation fuzz (Task S5)
// ===========================================================================
//
// Arms A/B/C above are the PRE-settlement (counter-only) invariant fuzz: they
// assert the dispute-core ledger (`stake_vault == total_oracle_stake`, the
// bond_pool identity, plurality) at the terminal counter state, with emission
// DISABLED and NO claims run — exactly as built for Task 13/H6. They are left
// untouched (still green, emission-disabled).
//
// Arms D/E are the POST-settlement physical fuzz added by S5: for a TERMINAL
// oracle with fuzzed bonds / fact stakes / vote kinds / per-fact agreed-vs-
// rejected-vs-duplicate / proposer correct-vs-wrong-vs-flipped-vs-disqualified /
// outcome AND emission enabled with a fuzzed amount, they run EVERY real S2 claim
// (+ S4 close) and assert each payout against an INDEPENDENT reference that
// REIMPLEMENTS the bucket / pro-rata / ceil-slash math (it does NOT call the
// program's `kassandra_oracles_program::reward` — that would be circular), plus the
// conservation equation: `Σ payouts + dust == Σ stakes + reward_emission`
// (Resolved) / `Σ payouts == Σ stakes` (InvalidDeadend, emission burned). Every
// payout is sourced from the stake vault; `total_oracle_stake` is never read.
//
// What is REAL vs SEEDED here: the CLAIM + CLOSE instructions (the token movers)
// are REAL, and on the InvalidDeadend arm the emission BURN-BACK is a REAL
// `finalize_oracle`. The terminal RESOLUTION (stamping reward_pool / cohort
// totals) is seeded — the organic stamping path is covered by Arm A + the
// `finalize_*` unit tests + `emissions.rs`. The deadend-after-SETTLED-CHALLENGE-
// with-emission combination flagged in S3 is covered deterministically by
// `settlement_e2e::e2e_deadend_after_settled_challenge_with_emission`.

use super::*;

use kassandra_oracles_program::state::{Phase, VOTE_APPROVE, VOTE_DUPLICATE};
use solana_signer::Signer;

/// Independent reimplementation of `reward::reward_buckets` (NOT a call to it).
fn ref_buckets(pool: u64, pw: u64, fw: u64, total_correct: u64, total_approved: u64) -> (u64, u64) {
    if total_approved == 0 {
        return (pool, 0);
    }
    if total_correct == 0 {
        return (0, pool);
    }
    let denom = pw as u128 + fw as u128;
    if denom == 0 {
        return (pool, 0);
    }
    let p = pool as u128;
    (
        (p * pw as u128 / denom) as u64,
        (p * fw as u128 / denom) as u64,
    )
}

/// Independent pro-rata reward (floor); 0 when the cohort total is 0.
fn ref_share(stake: u64, bucket: u64, total: u64) -> u64 {
    if total == 0 {
        return 0;
    }
    (stake as u128 * bucket as u128 / total as u128) as u64
}

/// Independent CEIL voter slash `ceil(stake·num/den)` (the on-chain rejected-fact
/// approve-voter slash; ceil keeps the vault solvent against the floor-aggregate
/// bond_pool credit).
fn ref_ceil_slash(stake: u64, num: u64, den: u64) -> u64 {
    if den == 0 {
        return 0;
    }
    ((stake as u128 * num as u128).div_ceil(den as u128)) as u64
}

const PW: u64 = kassandra_oracles_program::config::REWARD_PROPOSER_WEIGHT;
const FW: u64 = kassandra_oracles_program::config::REWARD_FACT_WEIGHT;
const SLASH_NUM: u64 = 1;
const SLASH_DEN: u64 = 2;
const RESOLVED_OPT: u8 = 1;

/// A fuzzed proposer for the Resolved settlement arm.
#[derive(Clone, Copy, Debug)]
struct SettleProposerGen {
    bond: u64,
    /// Claims the winning option (vs the losing one).
    correct: bool,
    disqualified: bool,
    /// Slashed fraction of the bond in percent (0..=100): the flip slash for a
    /// survivor, or the bond_pool contribution for a disqualified proposer.
    slash_pct: u8,
}

/// A fuzzed fact (+ its votes) for the Resolved settlement arm.
#[derive(Clone, Debug)]
struct SettleFactGen {
    stake: u64,
    /// 0 = agreed, 1 = rejected, 2 = duplicate-dominant.
    class: u8,
    /// (stake, is_approve) per vote.
    votes: Vec<(u64, bool)>,
}

fn settle_proposer_strategy() -> impl Strategy<Value = SettleProposerGen> {
    (1_000u64..3_001, any::<bool>(), any::<bool>(), 0u8..=100).prop_map(
        |(bond, correct, disqualified, slash_pct)| SettleProposerGen {
            bond,
            correct,
            disqualified,
            slash_pct,
        },
    )
}

fn settle_fact_strategy() -> impl Strategy<Value = SettleFactGen> {
    (
        100u64..1_001,
        0u8..3,
        prop::collection::vec((100u64..1_001, any::<bool>()), 0..=2),
    )
        .prop_map(|(stake, class, votes)| SettleFactGen {
            stake,
            class,
            votes,
        })
}

/// Run the full Resolved settlement sweep for one fuzzed scenario and assert the
/// matrix + conservation against the independent reference.
fn run_resolved_settlement(
    proposers: &[SettleProposerGen],
    facts: &[SettleFactGen],
    emission: u64,
) -> Result<(), TestCaseError> {
    // Map the generated scenario to the harness terminal-oracle specs.
    let p_specs: Vec<ClaimProposerSpec> = proposers
        .iter()
        .map(|p| {
            let slashed_amount = (p.bond as u128 * p.slash_pct as u128 / 100) as u64;
            ClaimProposerSpec {
                bond: p.bond,
                claim_option: if p.correct { RESOLVED_OPT } else { 0 },
                disqualified: p.disqualified,
                slashed_amount,
            }
        })
        .collect();
    let f_specs: Vec<ClaimFactSpec> = facts
        .iter()
        .map(|f| ClaimFactSpec {
            stake: f.stake,
            agreed: f.class == 0,
            duplicate: f.class == 2,
            votes: f
                .votes
                .iter()
                .map(|&(stake, approve)| ClaimVoteSpec {
                    stake,
                    kind: if approve {
                        VOTE_APPROVE
                    } else {
                        VOTE_DUPLICATE
                    },
                })
                .collect(),
        })
        .collect();

    // ----- independent reference stamps (recomputed from the scenario) --------
    let mut total_correct = 0u64;
    for p in &p_specs {
        if !p.disqualified && p.claim_option == RESOLVED_OPT {
            total_correct += p.bond;
        }
    }
    let mut total_approved = 0u64;
    for f in &f_specs {
        if f.agreed {
            let approve: u64 = f
                .votes
                .iter()
                .filter(|v| v.kind == VOTE_APPROVE)
                .map(|v| v.stake)
                .sum();
            total_approved += f.stake + approve;
        }
    }
    let mut bond_pool = 0u64;
    for p in &p_specs {
        bond_pool += p.slashed_amount;
    }
    for f in &f_specs {
        if !f.agreed && !f.duplicate {
            let approve: u64 = f
                .votes
                .iter()
                .filter(|v| v.kind == VOTE_APPROVE)
                .map(|v| v.stake)
                .sum();
            bond_pool += f.stake + (approve as u128 * SLASH_NUM as u128 / SLASH_DEN as u128) as u64;
        }
    }
    let reward_pool = bond_pool + emission;
    let (pbucket, fbucket) = ref_buckets(reward_pool, PW, FW, total_correct, total_approved);

    // ----- seed the terminal oracle + fold the emission -----------------------
    let mut ctx = TestCtx::new();
    let seed = ctx.seed_terminal_oracle(
        Phase::Resolved,
        RESOLVED_OPT,
        &p_specs,
        &f_specs,
        SLASH_NUM,
        SLASH_DEN,
    );
    // The seeded resolution stamps must agree with the independent reference.
    let o0 = ctx.oracle(seed.oracle);
    prop_assert_eq!(o0.total_correct_proposer_stake, total_correct);
    prop_assert_eq!(o0.total_approved_fact_stake, total_approved);
    prop_assert_eq!(o0.reward_pool, bond_pool, "pre-emission reward_pool");
    if emission > 0 {
        ctx.fold_reward_emission(seed.oracle, emission);
    }
    let vault_initial = ctx.token_balance(seed.stake_vault);
    prop_assert_eq!(vault_initial, seed.vault_initial + emission);

    let mut total_payout = 0u64;

    // ----- claim every fact vote, then the submitter (votes-first ordering) ----
    for f in &seed.facts {
        let fact = ctx.fact(f.submitter.account);
        let resolved_fact_agreed = fact.agreed == 1;
        let resolved_fact_dup = fact.duplicate == 1;
        for v in &f.votes {
            let vote = ctx.fact_vote(v.account);
            let predicted = if vote.kind == VOTE_DUPLICATE {
                vote.stake
            } else if resolved_fact_agreed {
                vote.stake + ref_share(vote.stake, fbucket, total_approved)
            } else if resolved_fact_dup {
                vote.stake
            } else {
                vote.stake - ref_ceil_slash(vote.stake, SLASH_NUM, SLASH_DEN)
            };
            let ix = ctx.claim_fact_vote_ix(
                seed.oracle,
                seed.nonce,
                v.account,
                f.submitter.account,
                v.dest_kass,
                seed.stake_vault,
                v.authority.pubkey(),
            );
            let res = ctx.send(ix, &[]);
            prop_assert!(
                res.is_ok(),
                "vote claim must not run the vault short: {:?}",
                res
            );
            prop_assert_eq!(
                ctx.token_balance(v.dest_kass),
                predicted,
                "vote payout matches reference"
            );
            total_payout += predicted;
        }
        let predicted_sub = if resolved_fact_agreed {
            fact.stake + ref_share(fact.stake, fbucket, total_approved)
        } else if resolved_fact_dup {
            fact.stake
        } else {
            0
        };
        let ix = ctx.claim_fact_ix(
            seed.oracle,
            seed.nonce,
            f.submitter.account,
            f.submitter.dest_kass,
            seed.stake_vault,
            f.submitter.authority.pubkey(),
        );
        let res = ctx.send(ix, &[]);
        prop_assert!(
            res.is_ok(),
            "submitter claim must not run the vault short: {:?}",
            res
        );
        prop_assert_eq!(
            ctx.token_balance(f.submitter.dest_kass),
            predicted_sub,
            "submitter payout matches reference"
        );
        total_payout += predicted_sub;
    }

    // ----- claim every proposer (+ close its AiClaim) — reward receivers last --
    for p in &seed.proposers {
        let pr = ctx.proposer(p.account);
        let predicted = if pr.disqualified != 0 {
            0
        } else {
            let base = pr.bond - pr.slashed_amount;
            let reward = if pr.claim_option == RESOLVED_OPT {
                ref_share(pr.bond, pbucket, total_correct)
            } else {
                0
            };
            base + reward
        };
        // Seed + close an AiClaim for this proposer (rent-only; honors claims+closes).
        let ai_claim = ctx.seed_ai_claim(seed.oracle, p.account, p.authority.pubkey());
        let ix = ctx.claim_proposer_ix(
            seed.oracle,
            seed.nonce,
            p.account,
            p.dest_kass,
            seed.stake_vault,
            p.authority.pubkey(),
        );
        let res = ctx.send(ix, &[]);
        prop_assert!(
            res.is_ok(),
            "proposer claim must not run the vault short: {:?}",
            res
        );
        prop_assert_eq!(
            ctx.token_balance(p.dest_kass),
            predicted,
            "proposer payout matches reference"
        );
        total_payout += predicted;

        let ix = ctx.close_ai_claim_ix(seed.oracle, ai_claim, p.authority.pubkey());
        prop_assert!(ctx.send(ix, &[]).is_ok(), "close_ai_claim should succeed");
        prop_assert!(ctx.is_closed(ai_claim));
    }

    // ----- CONSERVATION: Σ payouts + dust == Σ stakes + reward_emission --------
    let dust = ctx.token_balance(seed.stake_vault);
    prop_assert_eq!(
        total_payout + dust,
        vault_initial,
        "Σ payouts + dust == Σ stakes + reward_emission"
    );
    // Dust = floor/ceil reward remainders PLUS, for each disqualified proposer, the
    // forfeited-but-uncredited `bond − slashed_amount`. In the real flow that
    // remainder is the `kass_fee` already sent OUT of the vault to the challenger
    // (so it is never in the vault); the seeded model over-funds the vault by it,
    // leaving it as conservation-SAFE dust (an under-pay, never an over-pay). The
    // bound below catches any gross over-retention while allowing that surplus.
    let disq_forfeit_surplus: u64 = p_specs
        .iter()
        .filter(|p| p.disqualified)
        .map(|p| p.bond - p.slashed_amount)
        .sum();
    // Per-voter CEIL excess: a rejected fact credits bond_pool with the FLOOR
    // aggregate `floor(Σapprove·num/den)` but each approve-voter is slashed
    // `ceil(stakeᵢ·num/den)`, so the vault safely retains up to (ceil − floor) per
    // voter (the S2 ceil-slash solvency margin). Allow it in the dust bound.
    let mut ceil_excess: u64 = 0;
    for f in &f_specs {
        if !f.agreed && !f.duplicate {
            for v in &f.votes {
                if v.kind == VOTE_APPROVE {
                    let floor = (v.stake as u128 * SLASH_NUM as u128 / SLASH_DEN as u128) as u64;
                    ceil_excess += ref_ceil_slash(v.stake, SLASH_NUM, SLASH_DEN) - floor;
                }
            }
        }
    }
    prop_assert!(
        dust <= reward_pool + disq_forfeit_surplus + ceil_excess,
        "dust is floor/ceil reward remainder + disqualified forfeit surplus + ceil-slash margin"
    );
    Ok(())
}

proptest! {
    // Arm D drives a full physical claim+close sweep per case (fresh LiteSVM +
    // program deploy + seed + up to ~12 claim/close txs), so the case count is
    // kept modest (48) to stay fast and non-flaky.
    #![proptest_config(ProptestConfig {
        cases: 48,
        max_shrink_iters: 128,
        .. ProptestConfig::default()
    })]

    /// Arm D (Task S5) — RESOLVED physical-settlement conservation with emission.
    /// Every matrix combination (proposer correct/wrong/flipped/disqualified ×
    /// fact agreed/rejected/duplicate × vote approve/duplicate) × a fuzzed
    /// emission, claimed via the REAL S2/S4 instructions and asserted against the
    /// INDEPENDENT reference + `Σ payouts + dust == Σ stakes + reward_emission`.
    #[test]
    fn resolved_settlement_conservation(
        proposers in prop::collection::vec(settle_proposer_strategy(), 1..=3),
        facts in prop::collection::vec(settle_fact_strategy(), 0..=2),
        emission in 0u64..2_001,
    ) {
        run_resolved_settlement(&proposers, &facts, emission)?;
    }
}

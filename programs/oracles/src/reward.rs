//! Pure reward-bucket + pro-rata settlement math (Task S1).
//!
//! These functions turn the resolution-time stamps (`reward_pool`, the cohort
//! weights `PW`/`FW`, and the two cohort totals `total_correct_proposer_stake`
//! / `total_approved_fact_stake`) into the per-staker reward entitlements that
//! the S2 pull-claims pay out. Like [`crate::plurality`], they are pure,
//! allocation-free, and `no_std`-safe so they can be called from on-chain
//! processors AND unit-tested without LiteSVM.
//!
//! # NO token movement
//! S1 does NOT move tokens. These helpers only COMPUTE amounts; the actual
//! `stake_vault` transfers happen in the S2 claim instructions, which source
//! every payout from the real vault balance per the conservation contract.
//!
//! # Rounding / dust direction
//! All division is done in `u128` and FLOORS (truncates toward zero). Two
//! sources of floor remainder ("dust"):
//!
//! 1. The bucket split: `proposer_bucket + fact_bucket <= reward_pool` (each
//!    floored), so up to ~1 base unit of the pool can be unallocated.
//! 2. The pro-rata share: `Σ proposer_reward(bond_i) <= proposer_bucket` and
//!    `Σ fact_reward(stake_i) <= fact_bucket` (each share floored).
//!
//! Net: `Σ all rewards <= reward_pool`. The floor dust is never over-paid; it
//! simply remains in `stake_vault`, un-claimable by anyone this milestone. A
//! future governance sweep can reclaim the residual (noted in the settlement
//! plan's "Out of scope — dust sweeping").

/// Split `reward_pool` into `(proposer_bucket, fact_bucket)` by the cohort
/// weights `pw : fw`.
///
/// `proposer_bucket = reward_pool·pw/(pw+fw)`, `fact_bucket =
/// reward_pool·fw/(pw+fw)` (u128, floor).
///
/// # Empty-cohort roll-in
/// A bucket is meaningless if its cohort has no stake to distribute to, so the
/// whole pool rolls into the OTHER cohort:
/// * `total_approved_fact_stake == 0` → `(reward_pool, 0)` (the fact bucket
///   rolls into the proposer cohort — the design's rule; on `Resolved` there is
///   always ≥1 correct proposer).
/// * `total_correct_proposer_stake == 0` → `(0, reward_pool)` (symmetric;
///   shouldn't happen on `Resolved`, but handled so no pool is ever stranded by
///   a degenerate split).
///
/// If BOTH totals are zero, returns `(reward_pool, 0)` (proposer-cohort
/// fallback). If `pw + fw == 0` (guarded against by `set_config`, but defended
/// here) the same fallback applies so there is never a divide-by-zero.
pub fn reward_buckets(
    reward_pool: u64,
    pw: u64,
    fw: u64,
    total_correct_proposer_stake: u64,
    total_approved_fact_stake: u64,
) -> (u64, u64) {
    // Empty-cohort roll-in (and the both-empty / degenerate fallbacks).
    if total_approved_fact_stake == 0 {
        return (reward_pool, 0);
    }
    if total_correct_proposer_stake == 0 {
        return (0, reward_pool);
    }
    let denom = (pw as u128) + (fw as u128);
    if denom == 0 {
        // set_config guarantees pw + fw >= 1; defensive fallback otherwise.
        return (reward_pool, 0);
    }
    let pool = reward_pool as u128;
    let proposer_bucket = (pool * (pw as u128) / denom) as u64;
    let fact_bucket = (pool * (fw as u128) / denom) as u64;
    (proposer_bucket, fact_bucket)
}

/// One correct proposer's reward: `bond·proposer_bucket/total_correct_proposer_stake`
/// (u128, floor). Returns 0 if `total_correct_proposer_stake == 0` (no cohort to
/// divide among — no divide-by-zero).
pub fn proposer_reward(bond: u64, proposer_bucket: u64, total_correct_proposer_stake: u64) -> u64 {
    if total_correct_proposer_stake == 0 {
        return 0;
    }
    ((bond as u128) * (proposer_bucket as u128) / (total_correct_proposer_stake as u128)) as u64
}

/// One approved-fact staker's reward (submitter or approve-voter):
/// `stake·fact_bucket/total_approved_fact_stake` (u128, floor). Returns 0 if
/// `total_approved_fact_stake == 0`.
pub fn fact_reward(stake: u64, fact_bucket: u64, total_approved_fact_stake: u64) -> u64 {
    if total_approved_fact_stake == 0 {
        return 0;
    }
    ((stake as u128) * (fact_bucket as u128) / (total_approved_fact_stake as u128)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_split_exact() {
        // pool 300, PW:FW = 2:1 -> (200, 100), no dust.
        let (p, f) = reward_buckets(300, 2, 1, 1_000, 1_000);
        assert_eq!(p, 200);
        assert_eq!(f, 100);
        assert_eq!(p + f, 300);
    }

    #[test]
    fn bucket_split_equal_weights() {
        let (p, f) = reward_buckets(1_000, 1, 1, 500, 500);
        assert_eq!(p, 500);
        assert_eq!(f, 500);
    }

    #[test]
    fn bucket_split_floors_to_dust() {
        // pool 100, PW:FW = 2:1 -> 66 + 33 = 99, 1 unit of floor dust unallocated.
        let (p, f) = reward_buckets(100, 2, 1, 1_000, 1_000);
        assert_eq!(p, 66);
        assert_eq!(f, 33);
        assert!(p + f <= 100, "buckets never exceed the pool");
        assert_eq!(100 - (p + f), 1, "1 unit of split dust stays in the vault");
    }

    #[test]
    fn empty_fact_cohort_rolls_into_proposer() {
        // No approved-fact stake: the whole pool is the proposer bucket.
        let (p, f) = reward_buckets(1_000, 2, 1, 5_000, 0);
        assert_eq!(p, 1_000);
        assert_eq!(f, 0);
    }

    #[test]
    fn empty_proposer_cohort_rolls_into_fact() {
        // No correct-proposer stake (degenerate; shouldn't happen on Resolved):
        // the whole pool rolls into the fact cohort, never stranded.
        let (p, f) = reward_buckets(1_000, 2, 1, 0, 5_000);
        assert_eq!(p, 0);
        assert_eq!(f, 1_000);
    }

    #[test]
    fn both_cohorts_empty_falls_back_to_proposer() {
        let (p, f) = reward_buckets(1_000, 2, 1, 0, 0);
        assert_eq!(p, 1_000);
        assert_eq!(f, 0);
    }

    #[test]
    fn zero_weight_denominator_guarded() {
        // set_config forbids pw + fw == 0; the helper still never divides by zero.
        let (p, f) = reward_buckets(1_000, 0, 0, 1_000, 1_000);
        assert_eq!(p, 1_000);
        assert_eq!(f, 0);
    }

    #[test]
    fn proposer_reward_pro_rata() {
        // bucket 200 split over total 2_000: a 1_000-bond proposer earns 100.
        assert_eq!(proposer_reward(1_000, 200, 2_000), 100);
        assert_eq!(proposer_reward(500, 200, 2_000), 50);
    }

    #[test]
    fn fact_reward_pro_rata() {
        assert_eq!(fact_reward(1_000, 100, 2_000), 50);
        assert_eq!(fact_reward(1_500, 100, 2_000), 75);
    }

    #[test]
    fn zero_total_guards_return_zero() {
        assert_eq!(proposer_reward(1_000, 200, 0), 0);
        assert_eq!(fact_reward(1_000, 100, 0), 0);
    }

    #[test]
    fn pro_rata_sum_never_exceeds_bucket_dust_stays() {
        // Three bonds 1_000 / 1_000 / 1_001, total 3_001, bucket 1_000.
        // Each floored share: 333 + 333 + 333 = 999 <= 1_000; 1 unit of dust.
        let total = 3_001u64;
        let bucket = 1_000u64;
        let r0 = proposer_reward(1_000, bucket, total);
        let r1 = proposer_reward(1_000, bucket, total);
        let r2 = proposer_reward(1_001, bucket, total);
        assert_eq!(r0, 333);
        assert_eq!(r1, 333);
        assert_eq!(r2, 333);
        assert!(
            r0 + r1 + r2 <= bucket,
            "pro-rata shares never exceed the bucket"
        );
    }

    #[test]
    fn end_to_end_conservation_sum_le_pool() {
        // Full split + pro-rata: Σ all rewards <= reward_pool (floor dust only).
        let pool = 1_000u64;
        let (pb, fb) = reward_buckets(pool, 2, 1, 3_000, 1_500);
        // Proposer cohort: bonds 1_000 + 2_000 = 3_000.
        let pr = proposer_reward(1_000, pb, 3_000) + proposer_reward(2_000, pb, 3_000);
        // Fact cohort: stakes 500 + 1_000 = 1_500.
        let fr = fact_reward(500, fb, 1_500) + fact_reward(1_000, fb, 1_500);
        assert!(
            pr + fr <= pool,
            "total payouts never exceed the reward pool"
        );
    }

    #[test]
    fn large_values_no_overflow() {
        // u64-scale pool and stakes: u128 intermediates must not overflow.
        let pool = u64::MAX / 2;
        let (pb, fb) = reward_buckets(pool, 2, 1, u64::MAX, u64::MAX);
        assert!(pb <= pool && fb <= pool);
        let r = proposer_reward(u64::MAX, pb, u64::MAX);
        assert!(r <= pb);
    }
}

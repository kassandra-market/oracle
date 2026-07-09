//! Activity-scaled minimum-stake floor (bootstrapping, design
//! `2026-07-08-oracle-stake-floor-bootstrap`).
//!
//! To let the first oracles be created + participated in with **no premined
//! KASS**, the minimum stake for `propose` / `submit_fact` / `vote_fact` starts at
//! 0 and grows with network activity. Activity is the same
//! [`crate::state::Protocol::fee_ema`] creation-activity signal that drives the
//! creation fee; the floor is a piecewise-linear function of it:
//!
//! * `ema <= threshold` → **0** (free — the genesis / low-activity bootstrap band)
//! * `threshold < ema < cap` → **linear ramp** 0 → `max`
//! * `ema >= cap` → **`max`** (capped)
//!
//! The params (`threshold`, `cap`, `max`) are governable ([`crate::state::Protocol`],
//! retuned by `set_config`) and snapshotted onto each oracle's `min_stake` at
//! `create_oracle`, so an oracle's floor is frozen for its whole life. With
//! `max == 0` (the genesis default, mirroring the emission switch) the floor is
//! always 0 — participation is free until governance activates the ramp.
//!
//! The fee-EMA is scaled by [`crate::config::FEE_EMA_SCALE`] and bumped by
//! [`crate::config::FEE_EMA_INCREMENT`] per creation with a
//! [`crate::config::FEE_EMA_HALFLIFE_SECS`] half-life, so at a steady rate of `n`
//! oracles/day it settles at `E(n) = FEE_EMA_INCREMENT / (1 − 2^(−1/n))`. The
//! recommended defaults ([`crate::config::STAKE_FLOOR_EMA_THRESHOLD`] /
//! [`crate::config::STAKE_FLOOR_EMA_CAP`]) place the free band below ~10 oracles/day
//! and the cap at ~1000 oracles/day.

/// The minimum stake (KASS base units) for an oracle whose creation-time
/// (decayed) fee-EMA was `ema`, given the governable curve params. Piecewise
/// linear (see the module docs). Returns 0 when disabled (`max == 0`) or
/// degenerate (`cap <= threshold`). All arithmetic is done in `u128`, so it is
/// overflow-safe for any `u64` inputs.
pub fn stake_floor(ema: u64, threshold: u64, cap: u64, max: u64) -> u64 {
    // Disabled (magnitude 0), degenerate curve, or inside the free band → 0.
    if max == 0 || cap <= threshold || ema <= threshold {
        return 0;
    }
    if ema >= cap {
        return max;
    }
    // Linear ramp. `max as u128 * pos` cannot overflow u128 (both < 2^64), and
    // `span > 0` here (cap > threshold), so the division is safe.
    let span = (cap - threshold) as u128;
    let pos = (ema - threshold) as u128;
    ((max as u128) * pos / span) as u64
}

#[cfg(test)]
mod tests {
    use super::stake_floor;

    const THRESHOLD: u64 = 15_000_000_000; // ≈10 oracles/day
    const CAP: u64 = 1_443_000_000_000; // ≈1000 oracles/day
    const MAX: u64 = 1_000_000_000; // 1 KASS

    #[test]
    fn free_at_or_below_threshold() {
        assert_eq!(stake_floor(0, THRESHOLD, CAP, MAX), 0);
        assert_eq!(stake_floor(THRESHOLD, THRESHOLD, CAP, MAX), 0);
        assert_eq!(stake_floor(THRESHOLD - 1, THRESHOLD, CAP, MAX), 0);
    }

    #[test]
    fn capped_at_or_above_cap() {
        assert_eq!(stake_floor(CAP, THRESHOLD, CAP, MAX), MAX);
        assert_eq!(stake_floor(CAP + 1, THRESHOLD, CAP, MAX), MAX);
        assert_eq!(stake_floor(u64::MAX, THRESHOLD, CAP, MAX), MAX);
    }

    #[test]
    fn linear_midpoint() {
        // Exactly halfway across the ramp → ~half of max (floor of the division).
        let mid = THRESHOLD + (CAP - THRESHOLD) / 2;
        let f = stake_floor(mid, THRESHOLD, CAP, MAX);
        // within 1 base unit of max/2 (integer-division floor)
        assert!(
            f.abs_diff(MAX / 2) <= 1,
            "midpoint floor {f} not ≈ {}",
            MAX / 2
        );
    }

    #[test]
    fn monotone_nondecreasing_across_ramp() {
        let mut prev = 0u64;
        for k in 0..=10u64 {
            let ema = THRESHOLD + (CAP - THRESHOLD) * k / 10;
            let f = stake_floor(ema, THRESHOLD, CAP, MAX);
            assert!(f >= prev, "not monotone at k={k}: {f} < {prev}");
            prev = f;
        }
        assert_eq!(prev, MAX);
    }

    #[test]
    fn disabled_when_max_zero() {
        // The genesis default: any activity level still yields a 0 floor.
        assert_eq!(stake_floor(CAP, THRESHOLD, CAP, 0), 0);
        assert_eq!(stake_floor(u64::MAX, 0, u64::MAX, 0), 0);
    }

    #[test]
    fn degenerate_curve_is_zero() {
        // cap <= threshold → treated as disabled (no divide-by-zero / no negative span).
        assert_eq!(stake_floor(u64::MAX, CAP, THRESHOLD, MAX), 0);
        assert_eq!(stake_floor(500, 100, 100, MAX), 0);
    }

    #[test]
    fn no_overflow_at_extremes() {
        // max = u64::MAX, full-width ramp — u128 intermediate must not overflow.
        // Halfway across a [0, u64::MAX] ramp with max u64::MAX ≈ u64::MAX / 2.
        let f = stake_floor(u64::MAX / 2, 0, u64::MAX, u64::MAX);
        assert!(f > 0);
        assert!(f.abs_diff(u64::MAX / 2) <= 2, "midpoint {f}");
    }
}

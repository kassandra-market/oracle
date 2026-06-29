//! Plurality resolution over surviving proposers.
//!
//! After the AI-claim phase and challenge settlement, the oracle's resolution
//! is the plurality (most-common categorical option) over the SURVIVING
//! proposers: one proposer = one vote for its `claim_option`. This is a pure,
//! allocation-free function so it can be called from on-chain processors
//! (Tasks 11/12) and unit-tested without LiteSVM.

/// Outcome of plurality over surviving proposers.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Plurality {
    /// The option with the strictly-highest vote count.
    Winner(u8),
    /// Two or more options tie for the highest count.
    Tie,
    /// Zero surviving proposers.
    NoSurvivors,
}

/// Compute the strict-plurality winner over surviving proposers.
///
/// `votes` is the `claim_option` of each SURVIVING proposer; the caller
/// (Tasks 11/12) is responsible for excluding disqualified proposers. One
/// proposer = one vote. Returns the option with the strictly-highest count,
/// or [`Plurality::Tie`] when two or more options share the maximum, or
/// [`Plurality::NoSurvivors`] when the slice is empty.
///
/// CONTRACT: `CLAIM_OPTION_NONE` (`0xFF`) must NOT be passed — a no-show is
/// disqualified before this point. (It is not special-cased here; if passed it
/// is counted as a vote for option `0xFF`.)
///
/// Allocation-free: counts into a fixed `[u32; 256]` stack array indexed by
/// option value (options are `u8`, so `< 256`).
pub fn plurality(votes: &[u8]) -> Plurality {
    if votes.is_empty() {
        return Plurality::NoSurvivors;
    }

    let mut counts = [0u32; 256];
    for &v in votes {
        counts[v as usize] += 1;
    }

    let mut best_option: u8 = 0;
    let mut best_count: u32 = 0;
    let mut tied = false;
    for (option, &count) in counts.iter().enumerate() {
        if count > best_count {
            best_count = count;
            best_option = option as u8;
            tied = false;
        } else if count == best_count && count != 0 {
            tied = true;
        }
    }

    if tied {
        Plurality::Tie
    } else {
        Plurality::Winner(best_option)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_winner() {
        assert_eq!(plurality(&[1, 1, 2]), Plurality::Winner(1));
    }

    #[test]
    fn two_way_tie() {
        assert_eq!(plurality(&[0, 1]), Plurality::Tie);
        assert_eq!(plurality(&[1, 1, 2, 2]), Plurality::Tie);
    }

    #[test]
    fn empty_no_survivors() {
        assert_eq!(plurality(&[]), Plurality::NoSurvivors);
    }

    #[test]
    fn single_vote() {
        assert_eq!(plurality(&[3]), Plurality::Winner(3));
    }

    #[test]
    fn larger_mixed() {
        assert_eq!(plurality(&[0, 2, 2, 2, 1, 1]), Plurality::Winner(2));
    }

    #[test]
    fn three_way_tie() {
        assert_eq!(plurality(&[0, 1, 2]), Plurality::Tie);
    }

    #[test]
    fn all_same() {
        assert_eq!(plurality(&[5, 5, 5]), Plurality::Winner(5));
    }

    #[test]
    fn winner_is_option_zero() {
        // Ensure option 0 can win and is not confused with "no votes".
        assert_eq!(plurality(&[0, 0, 1]), Plurality::Winner(0));
    }
}

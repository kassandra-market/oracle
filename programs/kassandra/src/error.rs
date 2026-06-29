//! Program error codes surfaced to clients as `ProgramError::Custom(u32)`.
//!
//! Discriminants are a **stable public contract**: clients decode them from
//! `InstructionError::Custom(n)`, so existing values must never be renumbered.
//! New errors are APPENDED with the next free discriminant.

use pinocchio::program_error::ProgramError;

/// Custom error codes for the Kassandra dispute-core program.
///
/// Each variant maps to `ProgramError::Custom(discriminant)`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KassandraError {
    /// Instruction is recognized but its processor is not implemented yet.
    NotImplemented = 0,
    /// The oracle is not in the phase this instruction requires.
    WrongPhase = 1,
    /// The current phase window has already closed (`now >= phase_ends_at`).
    WindowClosed = 2,
    /// The current phase window has not yet elapsed (`now < phase_ends_at`).
    WindowNotElapsed = 3,
    /// The signer is not authorized to perform this action.
    Unauthorized = 4,
    /// An account passed to the instruction is invalid (wrong owner, address,
    /// or contents).
    InvalidAccount = 5,
    /// A fact with this `content_hash` already exists for this oracle (the
    /// Fact PDA is already initialized).
    DuplicateFact = 6,
    /// A stake amount of zero was supplied where a positive stake is required
    /// (a zero-stake fact would pollute quorum for free).
    ZeroStake = 7,
    /// This voter has already voted on this fact (the FactVote PDA is already
    /// initialized): one vote per voter per fact.
    DuplicateVote = 8,
    /// `finalize_facts` was called with an empty account tail: at least one
    /// fact (or proposer, in the no-facts dead-end) must be supplied per call.
    /// Finalization is incremental, so a subset is allowed — but not nothing.
    IncompleteFactSet = 9,
    /// A fact passed to `finalize_facts` is already `settled` (idempotency
    /// guard): finalize must run exactly once over each fact. Also reused for
    /// an already-slashed proposer in the no-facts dead-end branch.
    AlreadySettled = 10,
    /// `finalize_facts` was invoked on an oracle whose `dispute_bond_total` is
    /// zero, so the fact-approval threshold would be undefined (defensive).
    NoDisputeBond = 11,
    /// An AI claim already exists for this proposer (the AiClaim PDA is already
    /// initialized): one claim per proposer.
    DuplicateClaim = 12,
    /// The claimed `option` is out of range for this oracle
    /// (`option >= options_count`).
    InvalidOption = 13,
    /// `open_challenge` was called against an `AiClaim` that already has an open
    /// challenge market (`ai_claim.challenged == 1`): one market per claim.
    AlreadyChallenged = 14,
    /// `settle_challenge` was called before the market's TWAP window elapsed
    /// (`now < market.twap_end`): the decision market is still trading.
    TwapWindowOpen = 15,
    /// `finalize_oracle` was called while one or more challenge decision markets
    /// are still open (`oracle.open_challenge_count != 0`). The final plurality
    /// recompute must not run until every challenged claim has been settled, or a
    /// not-yet-disqualified challenged proposer would be miscounted as surviving.
    ChallengesOutstanding = 16,
    /// `init_protocol` was called on a protocol PDA that is already initialized
    /// (non-zero lamports or non-empty data): the protocol singleton is created
    /// exactly once.
    AlreadyInitialized = 17,
    /// `create_oracle` was called with a `deadline` in the past (`deadline <
    /// now`): proposals open at the deadline, so it must be in the future.
    InvalidDeadline = 18,
    /// `create_oracle` was called with `options_count < 2`: a categorical oracle
    /// needs at least two options to be meaningful.
    InvalidOptionsCount = 19,
}

impl From<KassandraError> for ProgramError {
    fn from(e: KassandraError) -> Self {
        ProgramError::Custom(e as u32)
    }
}

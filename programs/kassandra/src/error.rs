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
    /// An options-count / option-index range violation. Two reuse sites:
    /// `create_oracle` with `options_count < 2` (a categorical oracle needs at
    /// least two options to be meaningful), and `propose` with an out-of-range
    /// proposed `option` index (`option >= oracle.options_count`).
    InvalidOptionsCount = 19,
    /// `propose` was called before the oracle's `deadline` (`now < deadline`):
    /// the proposal window opens only at the creation-time deadline.
    DeadlineNotReached = 20,
    /// `propose` was called after the proposal window closed: `now >=
    /// phase_ends_at` while `proposer_count > 0` (an empty window instead
    /// re-opens for the seeding first proposal). The caller must
    /// `finalize_proposals` rather than register late.
    ProposalWindowClosed = 21,
    /// `propose` would push `proposer_count` past [`crate::config::MAX_PROPOSERS`]:
    /// the on-chain liveness guarantee that keeps the one-shot `finalize_oracle`
    /// within a single transaction's account-lock budget.
    TooManyProposers = 22,
    /// `propose` was called by an authority that already registered a proposal on
    /// this oracle (the Proposer PDA is already initialized): one proposal per
    /// (oracle, authority).
    DuplicateProposer = 23,
    /// `finalize_proposals` was called on an oracle with `proposer_count == 0`:
    /// there is nothing to finalize, and an empty oracle stays open waiting for
    /// its first proposal (the empty-window seeding handled by `propose`).
    NoProposals = 24,
    /// `set_governance` was called after the DAO linkage was already recorded
    /// (`governance_set == 1`) by a signer that is not the current
    /// `dao_authority`: the admin→DAO handoff is one-shot, and only the DAO may
    /// rotate the linkage thereafter.
    GovernanceAlreadySet = 25,
    /// `set_config` was given an out-of-bounds governable parameter: a zero
    /// denominator (`threshold_den` / `market_threshold_den` / `flip_slash_den`
    /// / `fact_vote_slash_den` / `emission_den`), a fraction numerator that
    /// exceeds its denominator (`threshold`, `flip_slash`, `fact_vote_slash`,
    /// `emission`, `market_threshold`), a non-positive window
    /// (`phase_window` / `proposal_window` / `fee_ema_halflife`), or both reward
    /// weights zero. Rejecting these at the gate prevents a later
    /// divide-by-zero / nonsensical config on the create_oracle / settlement
    /// paths.
    InvalidConfig = 26,
    /// `claim_fact` (the submitter claim, which CLOSES the `Fact` account) was
    /// called while the fact still has unclaimed voter stake
    /// (`approve_stake != 0` or `duplicate_stake != 0`). Each `claim_fact_vote`
    /// decrements the relevant running total as a voter claims; the submitter's
    /// claim must run LAST so the `Fact` it closes stays alive for every voter's
    /// disposition read. Retry after the voters have claimed.
    VotersOutstanding = 27,
    /// `create_oracle` was about to mint `reward_emission` KASS (Task S3) but the
    /// canonical `kass_mint`'s SPL mint authority is NOT the program's
    /// mint-authority PDA (`[b"mint_authority"]`). Emission can only be trusted
    /// when the program PDA is the sole minter, so a mint whose authority was not
    /// handed to the PDA (or is `None`) is rejected here rather than silently
    /// minting against an attacker-controlled authority.
    BadMintAuthority = 28,
}

impl From<KassandraError> for ProgramError {
    fn from(e: KassandraError) -> Self {
        ProgramError::Custom(e as u32)
    }
}

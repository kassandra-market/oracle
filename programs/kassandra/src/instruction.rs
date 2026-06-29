//! Instruction wire format.
//!
//! The first byte of `instruction_data` is the discriminant selecting an
//! [`Ix`] variant; the remaining bytes are that instruction's payload (parsed
//! by the individual processors, not here).
//!
//! Discriminants are a **stable public contract** and are never renumbered.
//! New instructions are APPENDED with the next free discriminant, so
//! [`Ix::from_u8`] stays trivial to extend.

/// Instruction discriminants for the Kassandra dispute-core program.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ix {
    SubmitFact = 0,
    VoteFact = 1,
    FinalizeFacts = 2,
    SubmitAiClaim = 3,
    OpenChallenge = 4,
    SettleChallenge = 5,
    FinalizeOracle = 6,
    /// Permissionless `FactProposal -> FactVoting` freeze once the proposal
    /// window has elapsed.
    AdvancePhase = 7,
    /// Incremental settlement of the AI-claim round once its window has elapsed
    /// (slash no-shows fully, flippers partially), advancing to `Challenge`.
    FinalizeAiClaims = 8,
    /// One-time protocol initializer: creates the `[b"protocol"]` singleton
    /// recording the admin + canonical KASS/USDC mints. Stable contract.
    InitProtocol = 9,
    /// Create an oracle in [`crate::state::Phase::Proposal`] with a future
    /// deadline plus its program-controlled stake vault (KASS token account at
    /// PDA `[b"vault", oracle]`, authority = oracle PDA). No fee yet (Task H2).
    CreateOracle = 10,
    /// Register a proposal against an oracle in [`crate::state::Phase::Proposal`]
    /// after its `deadline`: a categorical `option` + a KASS `bond` escrowed into
    /// the oracle's stake vault, creating one `Proposer` PDA per (oracle,
    /// authority). Enforces the `MAX_PROPOSERS` cap on-chain.
    Propose = 11,
    /// At the proposal-window end, finalize an oracle in
    /// [`crate::state::Phase::Proposal`]: if every proposer agrees, resolve it
    /// ([`crate::state::Phase::Resolved`] + `resolved_option`); if any conflict,
    /// open the dispute by setting `dispute_bond_total = total_oracle_stake` and
    /// advancing to [`crate::state::Phase::FactProposal`] (the dispute-core seam).
    FinalizeProposals = 12,
    /// One-time DAO-linkage handoff (Task F1): records `dao_authority` (the
    /// Squads v4 multisig vault PDA) + `kass_dao` (the futarchy `Dao` account)
    /// into the `Protocol`. Gated to `Protocol.admin` while `governance_set ==
    /// 0`; once set, only the current `dao_authority` may rotate it (so
    /// governance can rotate itself after handoff).
    SetGovernance = 13,
    /// DAO-gated retune of the `Protocol`-resident governable params (Task F3):
    /// overwrites the monetary + behavioral config fields wholesale from a
    /// fixed 144-byte payload, bounds-checked. Gated to `Protocol.dao_authority`
    /// (signer). Does NOT touch existing oracles (their snapshots are frozen);
    /// subsequently-created oracles snapshot the new values.
    SetConfig = 14,
    /// DAO-gated resolution of a dead-ended oracle (Task F4): sets a final
    /// `resolved_option` and advances an oracle from
    /// [`crate::state::Phase::InvalidDeadend`] to
    /// [`crate::state::Phase::Resolved`]. Gated to `Protocol.dao_authority`
    /// (signer). Economic settlement of the dead-end is DEFERRED to the
    /// settlement milestone; this only stamps the terminal outcome.
    ResolveDeadend = 15,
    // Future variants are APPENDED here with the next discriminant; add a
    // matching arm to `from_u8` below.
}

impl Ix {
    /// Decode the leading discriminant byte into an [`Ix`], or `None` if it
    /// does not correspond to a known instruction.
    pub fn from_u8(x: u8) -> Option<Self> {
        match x {
            0 => Some(Ix::SubmitFact),
            1 => Some(Ix::VoteFact),
            2 => Some(Ix::FinalizeFacts),
            3 => Some(Ix::SubmitAiClaim),
            4 => Some(Ix::OpenChallenge),
            5 => Some(Ix::SettleChallenge),
            6 => Some(Ix::FinalizeOracle),
            7 => Some(Ix::AdvancePhase),
            8 => Some(Ix::FinalizeAiClaims),
            9 => Some(Ix::InitProtocol),
            10 => Some(Ix::CreateOracle),
            11 => Some(Ix::Propose),
            12 => Some(Ix::FinalizeProposals),
            13 => Some(Ix::SetGovernance),
            14 => Some(Ix::SetConfig),
            15 => Some(Ix::ResolveDeadend),
            _ => None,
        }
    }
}

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
    // Future variants (e.g. AdvancePhase) are APPENDED here with the next
    // discriminant; add a matching arm to `from_u8` below.
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
            _ => None,
        }
    }
}

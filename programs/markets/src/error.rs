use pinocchio::error::ProgramError;

#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MarketError {
    InvalidAccount = 0,
    Unauthorized = 1,
    AlreadyInitialized = 2,
    InvalidSplit = 3, // UNUSED — reserved discriminant (no renumber)
    ZeroAmount = 4,
    NotFunding = 5,
    OracleNotTerminal = 6,
    AlreadyFunded = 7,  // UNUSED — reserved discriminant (no renumber)
    AlreadyClaimed = 8, // UNUSED since claim_lp/refund reap the Contribution (absence == idempotency); kept for wire stability (no renumber)
    NotCancelled = 9,
    OracleResolved = 10,
    NotBinary = 11, // UNUSED — reserved discriminant (no renumber)
    WrongMint = 12,
    NotFunded = 13,
    PoolNotEmpty = 14,
    NotActive = 15,
    AlreadySettled = 16,
    InvalidFee = 17,
    FeeNotCollected = 18,
    InvalidOutcome = 19,
    ContributionsOpen = 20,
    NotSettled = 21,
    /// `init_config` caller is not the program's on-chain upgrade authority.
    NotUpgradeAuthority = 22,
}

impl From<MarketError> for ProgramError {
    fn from(e: MarketError) -> Self {
        ProgramError::Custom(e as u32)
    }
}

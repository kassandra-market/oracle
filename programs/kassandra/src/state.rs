//! Fixed-size, zero-copy on-chain account layouts and the dispute phase enum.
//!
//! Every account struct is `#[repr(C)]`, `Pod` + `Zeroable`, and fully packed
//! (no implicit padding): fields are ordered and explicit `_pad` arrays are
//! inserted so each struct's `size_of` is a multiple of its 8-byte alignment.
//! This lets us read/write them straight out of account data with `bytemuck`.

use bytemuck::{Pod, Zeroable};

/// 32-byte Solana public key, kept as a plain byte array so it is `Pod`.
pub type Pubkey = [u8; 32];

/// `Proposer.claim_option` sentinel: no AI claim submitted yet.
pub const CLAIM_OPTION_NONE: u8 = 0xFF;
/// `FactVote.kind`: approve vote.
pub const VOTE_APPROVE: u8 = 0;
/// `FactVote.kind`: duplicate vote.
pub const VOTE_DUPLICATE: u8 = 1;

/// On-chain account-type discriminator. Stored as the FIRST byte of every Pod
/// account (each struct's `account_type` field) so processors can reject
/// type-confusion: an attacker cannot pass a `Fact` where an `Oracle` is
/// expected because the tag won't match. `Uninitialized` (0) is what a freshly
/// `CreateAccount`'d, zeroed account carries before it is stamped.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AccountType {
    Uninitialized = 0,
    Oracle = 1,
    Proposer = 2,
    Fact = 3,
    FactVote = 4,
    AiClaim = 5,
}

impl AccountType {
    /// Encode this tag as its stored `u8` discriminant.
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Lifecycle phase of an oracle dispute. Stored on-chain as a `u8`
/// discriminant (see [`Oracle::phase`]) to keep account structs `Pod`.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Created = 0,
    Proposal = 1,
    FactProposal = 2,
    FactVoting = 3,
    AiClaim = 4,
    Challenge = 5,
    FinalRecompute = 6,
    Resolved = 7,
    InvalidDeadend = 8,
}

impl Phase {
    /// Safely convert a stored `u8` discriminant back into a `Phase`.
    pub fn from_u8(x: u8) -> Option<Self> {
        match x {
            0 => Some(Phase::Created),
            1 => Some(Phase::Proposal),
            2 => Some(Phase::FactProposal),
            3 => Some(Phase::FactVoting),
            4 => Some(Phase::AiClaim),
            5 => Some(Phase::Challenge),
            6 => Some(Phase::FinalRecompute),
            7 => Some(Phase::Resolved),
            8 => Some(Phase::InvalidDeadend),
            _ => None,
        }
    }

    /// Encode this phase as its stored `u8` discriminant.
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Top-level dispute account. `size_of == 232`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Oracle {
    pub account_type: u8, // AccountType::Oracle
    pub _pad_hdr: [u8; 7],
    pub creator: Pubkey,
    pub kass_mint: Pubkey,
    pub usdc_mint: Pubkey,
    pub stake_vault: Pubkey, // PDA token account holding all KASS bonds/stakes
    pub deadline: i64,       // unix; proposals rejected before this
    pub phase_ends_at: i64,  // end of the current window
    pub twap_window: i64,    // per-oracle, seconds
    pub options_count: u8,   // number of categorical options
    pub phase: u8,           // Phase as u8
    pub proposer_count: u16,
    pub surviving_count: u16, // proposers not disqualified
    pub fact_count: u16,
    pub total_oracle_stake: u64, // conservation accumulator (== vault balance)
    pub bond_pool: u64,          // accumulated slashed KASS (base units)
    pub dispute_bond_total: u64, // Σ proposer bonds, fixed at dispute start; fact-quorum denominator
    pub settled_count: u16,      // facts settled so far (drives incremental finalize)
    pub bump: u8,
    pub _pad1: [u8; 5],
    pub prompt_hash: [u8; 32], // hash of fixed prompt + interpretation
}

impl Oracle {
    pub const LEN: usize = core::mem::size_of::<Oracle>();

    /// Decode the stored phase discriminant.
    pub fn phase(&self) -> Option<Phase> {
        Phase::from_u8(self.phase)
    }

    /// Write the phase discriminant.
    pub fn set_phase(&mut self, p: Phase) {
        self.phase = p as u8;
    }
}

/// A proposer's commitment within an oracle. `size_of == 88`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Proposer {
    pub account_type: u8, // AccountType::Proposer
    pub _pad_hdr: [u8; 7],
    pub oracle: Pubkey,
    pub authority: Pubkey,
    pub bond: u64,           // locked KASS
    pub original_option: u8, // value at proposal time (no proofs)
    pub claim_option: u8,    // value after AI claim; CLAIM_OPTION_NONE = not yet submitted
    pub disqualified: u8,    // bool
    pub slashed: u8,         // bool
    pub flipped: u8,         // bool: claim_option != original_option
    pub bump: u8,
    pub _pad: [u8; 2],
}

impl Proposer {
    pub const LEN: usize = core::mem::size_of::<Proposer>();

    pub fn is_disqualified(&self) -> bool {
        self.disqualified != 0
    }
    pub fn is_slashed(&self) -> bool {
        self.slashed != 0
    }
    pub fn is_flipped(&self) -> bool {
        self.flipped != 0
    }
}

/// A fact submitted in support of an option. `size_of == 336`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Fact {
    pub account_type: u8, // AccountType::Fact
    pub _pad_hdr: [u8; 7],
    pub oracle: Pubkey,
    pub proposer: Pubkey, // who submitted the fact
    pub content_hash: [u8; 32],
    pub stake: u64,
    pub approve_stake: u64,   // running tally
    pub duplicate_stake: u64, // running tally of "duplicate" votes
    pub uri_len: u16,
    pub agreed: u8,    // set at finalize: 1 if accepted
    pub duplicate: u8, // set at finalize: 1 if duplicate-dominant
    pub settled: u8,   // bool
    pub bump: u8,
    pub _pad: [u8; 2],
    pub uri: [u8; 200],
}

impl Fact {
    pub const LEN: usize = core::mem::size_of::<Fact>();

    pub fn is_agreed(&self) -> bool {
        self.agreed != 0
    }
    pub fn is_duplicate(&self) -> bool {
        self.duplicate != 0
    }
    pub fn is_settled(&self) -> bool {
        self.settled != 0
    }
}

/// A stake-weighted vote on a fact. `size_of == 88`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct FactVote {
    pub account_type: u8, // AccountType::FactVote
    pub _pad_hdr: [u8; 7],
    pub fact: Pubkey,
    pub voter: Pubkey,
    pub stake: u64,
    pub kind: u8, // 0 = approve, 1 = duplicate
    pub bump: u8,
    pub _pad: [u8; 6],
}

impl FactVote {
    pub const LEN: usize = core::mem::size_of::<FactVote>();
}

/// A pinned-model AI claim for a proposer's option. `size_of == 176`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct AiClaim {
    pub account_type: u8, // AccountType::AiClaim
    pub _pad_hdr: [u8; 7],
    pub oracle: Pubkey,
    pub proposer: Pubkey,
    pub model_id: [u8; 32],    // hash/ident of pinned model
    pub params_hash: [u8; 32], // hash of declared params (temp, seed, ...)
    pub io_hash: [u8; 32],     // hash(prompt + agreed facts + raw response)
    pub option: u8,
    pub challenged: u8, // bool
    pub bump: u8,
    pub _pad: [u8; 5],
}

impl AiClaim {
    pub const LEN: usize = core::mem::size_of::<AiClaim>();

    pub fn is_challenged(&self) -> bool {
        self.challenged != 0
    }
}

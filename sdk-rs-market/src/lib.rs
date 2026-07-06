pub mod ix;
pub mod metadao;
pub mod pda;

use solana_sdk::pubkey::Pubkey;
// Same pubkey as programs/kassandra-market/src/lib.rs.
pub const PROGRAM_ID: Pubkey = solana_sdk::pubkey!("FEGNHWAB7kc7VC9CCwbvVPsv4Jykz2r2WQ758V4xCT9S");

// Discriminants — mirror programs/kassandra-market/src/instruction.rs::Ix. Guarded by tests/parity.rs.
pub const IX_INIT_CONFIG: u8 = 0;
pub const IX_UPDATE_CONFIG: u8 = 1;
pub const IX_CREATE_MARKET: u8 = 2;
pub const IX_CONTRIBUTE: u8 = 3;
pub const IX_CANCEL: u8 = 4;
pub const IX_REFUND: u8 = 5;
pub const IX_ACTIVATE: u8 = 6;
pub const IX_CLAIM_LP: u8 = 7;
pub const IX_RESOLVE_MARKET: u8 = 8;
pub const IX_COLLECT_FEE: u8 = 9;
pub const IX_CLOSE_MARKET: u8 = 10;

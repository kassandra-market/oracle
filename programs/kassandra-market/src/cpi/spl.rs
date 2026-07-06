//! Shared SPL token-account byte offsets + size.
//!
//! `spl_token::state::Account` is a fixed 165-byte layout; these are the field
//! offsets the program reads (mint/owner/amount) plus the account length. Defined
//! ONCE here so `activate` / `collect_fee` / `claim_lp` / `create_market` /
//! `refund` share the same constants instead of re-declaring magic numbers.

/// `spl_token::state::Account.mint` byte offset.
pub const SPL_TOKEN_MINT_OFFSET: usize = 0;
/// `spl_token::state::Account.owner` byte offset.
pub const SPL_TOKEN_OWNER_OFFSET: usize = 32;
/// `spl_token::state::Account.amount` byte offset.
pub const SPL_TOKEN_AMOUNT_OFFSET: usize = 64;
/// Size of an SPL token account (`spl_token::state::Account::LEN`).
pub const SPL_TOKEN_ACCOUNT_LEN: usize = 165;

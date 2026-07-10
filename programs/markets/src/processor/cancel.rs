//! `cancel`: mark a `Funding` market `Cancelled` once its underlying Kassandra
//! oracle is terminal, at any funding level. Permissionless (no required signer
//! beyond the tx fee payer). A terminal oracle makes Phase-2 `activate`
//! impossible, so cancel+refund is the only exit — it must be available even to
//! a fully-funded market whose oracle resolved before activation.
//!
//! # Instruction payload (after the 1-byte discriminant)
//! empty.
//!
//! # Accounts
//! 0. market PDA — writable; must be `Funding`
//! 1. oracle     — readonly; must equal `market.oracle`, must be terminal

use pinocchio::{account::AccountView, address::Address, error::ProgramError, ProgramResult};

use crate::{
    error::MarketError,
    processor::guards::{assert_key, load_kassandra_oracle, load_market, write_market},
    state::MarketStatus,
};

pub fn process(
    program_id: &Address,
    accounts: &mut [AccountView],
    payload: &[u8],
) -> ProgramResult {
    if !payload.is_empty() {
        return Err(ProgramError::InvalidInstructionData);
    }
    let [market_ai, oracle_ai, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let market = load_market(market_ai, program_id)?;
    if market.status != MarketStatus::Funding.as_u8() {
        return Err(MarketError::NotFunding.into());
    }
    assert_key(oracle_ai, &market.oracle)?;
    let oracle = load_kassandra_oracle(oracle_ai)?;
    let terminal = oracle.phase == crate::kass_oracle::PHASE_RESOLVED
        || oracle.phase == crate::kass_oracle::PHASE_INVALID_DEADEND;
    if !terminal {
        return Err(MarketError::OracleNotTerminal.into());
    }
    // A terminal oracle makes Phase-2 `activate` impossible, so `cancel` must
    // always permit the refund exit — at ANY funding level. The `status ==
    // Funding` guard above still prevents cancelling an already-activated
    // market (which would be `status == Active`). This intentionally admits
    // fully-funded markets whose oracle resolved before activation, which would
    // otherwise strand their contributions.
    let mut m = market;
    m.status = MarketStatus::Cancelled.as_u8();
    write_market(market_ai, &m)?;
    Ok(())
}

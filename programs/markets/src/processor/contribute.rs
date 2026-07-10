//! `contribute`: add KASS from any contributor to a `Funding` market's escrow,
//! creating or incrementing that contributor's [`Contribution`].
//!
//! # Instruction payload (after the 1-byte discriminant), exactly 8 bytes
//! `amount: u64 LE`.
//!
//! # Accounts
//! 0. market PDA           — writable; must be `Funding`
//! 1. escrow PDA           — writable; must equal `market.escrow_vault`
//! 2. contributor          — signer, writable; pays rent + is the token authority
//! 3. contributor_kass_ata — writable; KASS source, authority == contributor
//! 4. contribution PDA     — writable (created or incremented)
//! 5. token program

use pinocchio::{account::AccountView, address::Address, error::ProgramError, ProgramResult};

use crate::{
    error::MarketError,
    processor::{
        contribution::record_contribution,
        guards::{assert_key, assert_signer, load_market, write_market},
    },
    state::MarketStatus,
};

pub fn process(
    program_id: &Address,
    accounts: &mut [AccountView],
    payload: &[u8],
) -> ProgramResult {
    if payload.len() != 8 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let amount = u64::from_le_bytes(payload[0..8].try_into().unwrap());

    let [market_ai, escrow_ai, contributor_ai, contributor_ata_ai, contribution_ai, token_prog_ai, ..] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    assert_signer(contributor_ai)?;
    let market = load_market(market_ai, program_id)?;
    if market.status != MarketStatus::Funding.as_u8() {
        return Err(MarketError::NotFunding.into());
    }
    assert_key(escrow_ai, &market.escrow_vault)?;

    // Detect the create-vs-top-up branch BEFORE `record_contribution` (which uses
    // this exact predicate to decide whether to create the PDA). Only a brand-new
    // Contribution grows `open_contributions`; a repeat top-up by an existing
    // contributor must NOT double-count.
    let creates_new = contribution_ai.lamports() == 0 && contribution_ai.is_data_empty();

    record_contribution(
        program_id,
        market_ai.address(),
        contributor_ai,
        contributor_ata_ai,
        escrow_ai,
        contribution_ai,
        token_prog_ai,
        contributor_ai,
        amount,
    )?;

    let mut m = market;
    m.total_contributed = m
        .total_contributed
        .checked_add(amount)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    if creates_new {
        m.open_contributions = m
            .open_contributions
            .checked_add(1)
            .ok_or(ProgramError::ArithmeticOverflow)?;
    }
    write_market(market_ai, &m)?;
    Ok(())
}

//! `refund`: permissionless per-contributor refund of staked KASS out of a
//! `Cancelled` market's escrow, back to the recorded contributor's KASS ata.
//!
//! The escrow's SPL authority is the market PDA, so the transfer is
//! program-signed with the market seeds `[b"market", oracle, [outcome_index], [bump]]`.
//!
//! Because anyone may crank this, it MUST verify the destination token account
//! belongs to the recorded `contribution.contributor` (its SPL owner, bytes
//! `32..64`) — otherwise a cranker could redirect someone's refund to itself.
//!
//! # Instruction payload (after the 1-byte discriminant)
//! empty.
//!
//! # Accounts
//! 0. market PDA          — writable; must be `Cancelled` (open_contributions decremented)
//! 1. escrow              — writable; must equal `market.escrow_vault`
//! 2. contribution PDA    — writable; must belong to this market (CLOSED here)
//! 3. contributor_kass_ata — writable; SPL owner must equal `contribution.contributor`
//! 4. contributor         — writable; == contribution.contributor (Contribution rent recipient)
//! 5. token program

use pinocchio::{
    account::AccountView, address::Address, cpi::Signer, error::ProgramError, ProgramResult,
};
use pinocchio_token::instructions::Transfer;

use crate::{
    error::MarketError,
    processor::guards::{
        assert_key, close_data_account, load_contribution, load_market, market_signer_seeds,
        read_token_owner, write_market,
    },
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
    let [market_ai, escrow_ai, contribution_ai, dest_ata_ai, contributor_ai, token_prog_ai, ..] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    assert_key(token_prog_ai, &pinocchio_token::ID)?;

    let market = load_market(market_ai, program_id)?;
    if market.status != MarketStatus::Cancelled.as_u8() {
        return Err(MarketError::NotCancelled.into());
    }
    assert_key(escrow_ai, &market.escrow_vault)?;

    let contribution = load_contribution(contribution_ai, program_id)?;
    if contribution.market != *market_ai.address() {
        return Err(MarketError::InvalidAccount.into());
    }
    // No `claimed` guard needed: the Contribution is REAPED below on this refund, so a
    // second attempt fails in `load_contribution` (InvalidAccount) — the account's
    // absence is the idempotency. `MarketError::AlreadyClaimed` is now unused.
    // Permissionless: the destination must belong to the recorded contributor.
    // Defense-in-depth: assert the destination is token-program-owned before
    // trusting its byte layout, so this guard stands on its own rather than
    // leaning on the downstream `Transfer` CPI to reject a bogus account.
    if read_token_owner(dest_ata_ai)? != contribution.contributor {
        return Err(MarketError::InvalidAccount.into());
    }
    // The Contribution's rent goes back to the recorded contributor when we close
    // it below, so bind the passed `contributor` account to `contribution.contributor`.
    assert_key(contributor_ai, &contribution.contributor)?;

    // Program-signed transfer out of escrow (authority = the market PDA).
    market_signer_seeds!(market, oidx, mbump, market_seeds);
    Transfer::new(escrow_ai, dest_ata_ai, market_ai, contribution.amount)
        .invoke_signed(&[Signer::from(&market_seeds)])?;

    // CLOSE the Contribution: its rent lamports go back to the contributor (they paid
    // it) and the account is reaped, so a second refund can't even load it (absence ==
    // idempotency — no `claimed` flag write needed).
    close_data_account(contribution_ai, contributor_ai)?;

    // Decrement the live-Contribution counter and persist the market. `checked_sub`
    // can't underflow: a loadable Contribution existed, so the counter was >= 1.
    let mut m = market;
    m.open_contributions = m
        .open_contributions
        .checked_sub(1)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    write_market(market_ai, &m)?;
    Ok(())
}

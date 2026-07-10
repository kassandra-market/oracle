//! `claim_lp`: permissionless per-contributor pro-rata claim of the AMM LP
//! tokens seeded at `activate`, out of the `Active` market's Market-PDA-owned
//! `lp_vault`, to the recorded contributor's LP token account.
//!
//! The `lp_vault`'s SPL authority is the market PDA, so the transfer is
//! program-signed with the market seeds `[b"market", oracle, [outcome_index], [bump]]`.
//!
//! Because anyone may crank this, it MUST verify the destination LP token
//! account belongs to the recorded `contribution.contributor` (its SPL owner,
//! bytes `32..64`) and is on `market.lp_mint` (its mint, bytes `0..32`) —
//! otherwise a cranker could redirect someone's LP to itself.
//!
//! `Contribution.claimed` is a single settle flag shared with `refund`: a
//! contribution is either refunded (Cancelled market) XOR LP-claimed (Active
//! market), never both — the two paths' status guards are mutually exclusive.
//!
//! # Instruction payload (after the 1-byte discriminant)
//! empty.
//!
//! # Accounts
//! 0. market PDA           — writable; must be `Active` (open_contributions decremented)
//! 1. lp_vault             — writable; must equal `market.lp_vault`
//! 2. contribution PDA     — writable; must belong to this market (CLOSED here)
//! 3. contributor_lp_ata   — writable; SPL owner == contributor, mint == lp_mint
//! 4. contributor          — writable; == contribution.contributor (Contribution rent recipient)
//! 5. token program

use pinocchio::{
    account::AccountView, address::Address, cpi::Signer, error::ProgramError, ProgramResult,
};
use pinocchio_token::instructions::Transfer;

use crate::{
    cpi::metadao,
    cpi::spl::SPL_TOKEN_AMOUNT_OFFSET,
    error::MarketError,
    processor::guards::{
        assert_key, close_data_account, load_contribution, load_market, market_signer_seeds,
        read_token_mint, read_token_owner, write_market,
    },
    state::MarketStatus,
};

/// Floor pro-rata share via a u128 intermediate (mirrors the sibling
/// `fee_amount`): `floor(lp_total × amount / total_contributed)`. Never
/// over-distributes; the floor-division remainder stays in `lp_vault`.
fn pro_rata_share(lp_total: u64, amount: u64, total_contributed: u64) -> Result<u64, ProgramError> {
    if total_contributed == 0 {
        return Err(MarketError::InvalidAccount.into());
    }
    let scaled = (lp_total as u128)
        .checked_mul(amount as u128)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    u64::try_from(scaled / total_contributed as u128).map_err(|_| ProgramError::ArithmeticOverflow)
}

pub fn process(
    program_id: &Address,
    accounts: &mut [AccountView],
    payload: &[u8],
) -> ProgramResult {
    if !payload.is_empty() {
        return Err(ProgramError::InvalidInstructionData);
    }
    let [market_ai, lp_vault_ai, contribution_ai, dest_ata_ai, contributor_ai, token_prog_ai, ..] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    assert_key(token_prog_ai, &pinocchio_token::ID)?;

    let market = load_market(market_ai, program_id)?;
    // Status guard: claim_lp only applies to an activated market. Funding (not yet
    // activated → no LP) and Cancelled (its exit is `refund`) are rejected here;
    // `refund` (Cancelled-only) stays mutually exclusive with claim_lp (a market is
    // never both Cancelled and activated). The `Active` arm is retained ONLY so an
    // Active market reports the more specific `FeeNotCollected` below rather than
    // `NotActive` — it can never reach the success path (see the fee gate).
    let activated = market.status == MarketStatus::Active.as_u8()
        || market.status == MarketStatus::Resolved.as_u8()
        || market.status == MarketStatus::Void.as_u8();
    if !activated {
        return Err(MarketError::NotActive.into());
    }
    // Fee gate (SECURITY-CRITICAL, INTENTIONAL — do NOT reopen Active claims):
    // claim_lp opens only after the resolve → collect_fee sequence has stamped
    // `fee_collected == 1`, so `lp_total` is final (reduced by the fee slice) and
    // pro-rata shares are computed off the post-fee total. `fee_collected` is set
    // only at/after resolution (`resolve_market` stamps it immediately for
    // fee-free / no-LP markets; `collect_fee` stamps it for every other market once
    // the fee is removed), so an ACTIVE market can NEVER satisfy this gate — it
    // always returns `FeeNotCollected`. That is deliberate and non-bypassable: an
    // early Active-market claim would move LP out of `lp_vault` before `collect_fee`
    // runs, potentially draining the vault below `fee_lp` and letting LPs escape the
    // protocol fee. Placed AFTER the status guard so Funding still reports
    // `NotActive`.
    if market.fee_collected != 1 {
        return Err(MarketError::FeeNotCollected.into());
    }
    assert_key(lp_vault_ai, &market.lp_vault)?;

    let contribution = load_contribution(contribution_ai, program_id)?;
    if contribution.market != *market_ai.address() {
        return Err(MarketError::InvalidAccount.into());
    }
    // No `claimed` guard needed: the Contribution is REAPED below on this claim, so a
    // second attempt fails in `load_contribution` (InvalidAccount) — the account's
    // absence is the idempotency. `MarketError::AlreadyClaimed` is now unused.
    // Permissionless: the destination must belong to the recorded contributor
    // AND be on the LP mint. Defense-in-depth: assert token-program ownership
    // before trusting the byte layout, so this guard stands on its own rather
    // than leaning on the downstream `Transfer` CPI.
    if read_token_mint(dest_ata_ai)? != market.lp_mint
        || read_token_owner(dest_ata_ai)? != contribution.contributor
    {
        return Err(MarketError::InvalidAccount.into());
    }
    // The Contribution's rent goes back to the recorded contributor when we close
    // it below, so bind the passed `contributor` account to `contribution.contributor`.
    assert_key(contributor_ai, &contribution.contributor)?;

    // LP amount. The LAST claimer (`open_contributions == 1`) sweeps the ENTIRE
    // remaining `lp_vault` balance so it ends at exactly 0 — absorbing the
    // floor-division dust every earlier claimer left behind, so `close_market` never
    // faces an un-closeable non-zero balance. Every earlier claimer takes the floor
    // pro-rata share (`floor(lp_total × amount / total_contributed)`).
    let share = if market.open_contributions == 1 {
        let d = lp_vault_ai.try_borrow()?;
        metadao::read_u64(&d, SPL_TOKEN_AMOUNT_OFFSET)?
    } else {
        pro_rata_share(
            market.lp_total,
            contribution.amount,
            market.total_contributed,
        )?
    };

    // Program-signed transfer out of lp_vault (authority = the market PDA).
    // Skip the CPI if the share is zero (a dust contributor's floored share), but
    // STILL close the contribution below so it cannot wedge in a retry loop.
    if share > 0 {
        market_signer_seeds!(market, oidx, mbump, market_seeds);
        Transfer::new(lp_vault_ai, dest_ata_ai, market_ai, share)
            .invoke_signed(&[Signer::from(&market_seeds)])?;
    }

    // CLOSE the Contribution: its rent lamports go back to the contributor (they paid
    // it) and the account is reaped, so a second claim can't even load it (absence ==
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

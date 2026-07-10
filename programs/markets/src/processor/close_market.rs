//! `close_market` (Ix 10): permissionless rent reclaim for a fully-settled
//! [`Market`] — SPL-`CloseAccount`s its Market-PDA-owned token accounts and closes
//! the `Market` data PDA, returning ALL rent to the creator (the original payer).
//!
//! Mirrors the sibling Kassandra program's `sweep_oracle` / `close_market` idiom:
//! program-signed SPL `CloseAccount` of the token accounts (their token authority
//! is the Market PDA), then a data-PDA close (lamport drain → recipient + `close()`).
//!
//! # Preconditions (all enforced)
//! * `status ∈ {Resolved, Void, Cancelled}` — a terminal market.
//! * `Resolved`/`Void`: `fee_collected == 1` — the fee crank has finalized `lp_total`.
//! * `open_contributions == 0` — EVERY contributor has exited (claimed/refunded), so
//!   no unclaimed LP or refund is stranded. This is the safety gate: it is impossible
//!   to close a market out from under a contributor who has not yet taken their share.
//!
//! # Which token accounts close
//! * Always: `escrow` (drained at `activate`, or fully refunded when Cancelled).
//! * Activated only (`market.lp_vault != default`): `cyes` + `cno` (drained at
//!   `activate`/`collect_fee`) + `lp_vault` (swept to 0 by the LAST `claim_lp`).
//!
//! All are 0-balance, which SPL `CloseAccount` requires; a non-zero balance would
//! (correctly) make the CPI fail loudly. MetaDAO accounts are NOT ours — never closed.
//!
//! # Instruction payload (after the 1-byte discriminant)
//! empty.
//!
//! # Accounts (FIXED order; the pool slots are ignored on the Cancelled path)
//! 0. market       — writable; the [`Market`] PDA, CLOSED here.
//! 1. creator      — writable; `== market.creator`; recipient of ALL reclaimed rent.
//! 2. escrow       — writable; `== market.escrow_vault`; CLOSED here.
//! 3. cyes         — writable; `[b"cyes", market]`; CLOSED iff activated.
//! 4. cno          — writable; `[b"cno", market]`; CLOSED iff activated.
//! 5. lp_vault     — writable; `== market.lp_vault`; CLOSED iff activated.
//! 6. token program.

use pinocchio::{
    account::AccountView,
    address::Address,
    cpi::{Seed, Signer},
    error::ProgramError,
    ProgramResult,
};
use pinocchio_token::instructions::CloseAccount;

use crate::{
    cpi::metadao,
    cpi::spl::SPL_TOKEN_AMOUNT_OFFSET,
    error::MarketError,
    processor::guards::{assert_key, close_data_account, load_market, market_signer_seeds},
    state::MarketStatus,
};

/// Close a Market-PDA-owned SPL token account ONLY IF it is empty (0 balance),
/// program-signed with the market seeds. A non-zero balance is LEFT IN PLACE
/// rather than reverting: SPL `CloseAccount` refuses a funded account, so an
/// unconditional close lets anyone permanently brick `close_market` by donating 1
/// token unit into a derivable Market-PDA token account (escrow / cyes / cno /
/// lp_vault) after settlement — nothing can drain it post-settle, so the close (and
/// the Market-PDA rent reclaim) would revert forever. Skipping a dusted account
/// instead means the griefer strands at most that one account's own rent (paid for
/// with their own donated tokens), while the Market data PDA is always reaped.
fn close_token_account_if_empty(
    account: &AccountView,
    destination: &AccountView,
    authority: &AccountView,
    seeds: &[Seed],
) -> ProgramResult {
    let amount = {
        let d = account.try_borrow()?;
        metadao::read_u64(&d, SPL_TOKEN_AMOUNT_OFFSET)?
    };
    if amount == 0 {
        CloseAccount::new(account, destination, authority).invoke_signed(&[Signer::from(seeds)])?;
    }
    Ok(())
}

/// A default (all-zero) pubkey: `market.lp_vault` is unset until `activate` records
/// it, so a Cancelled (never-activated) market has `lp_vault == DEFAULT_PUBKEY`.
const DEFAULT_PUBKEY: Address = Address::new_from_array([0u8; 32]);

pub fn process(
    program_id: &Address,
    accounts: &mut [AccountView],
    payload: &[u8],
) -> ProgramResult {
    if !payload.is_empty() {
        return Err(ProgramError::InvalidInstructionData);
    }
    let [market_ai, creator_ai, escrow_ai, cyes_ai, cno_ai, lp_vault_ai, token_prog_ai, ..] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    assert_key(token_prog_ai, &pinocchio_token::ID)?;

    let market = load_market(market_ai, program_id)?;
    // The creator paid the market/escrow/pool-account rents; they receive it all back.
    assert_key(creator_ai, &market.creator)?;

    // Terminal-status gate. Only a settled (Resolved/Void/Cancelled) market may be
    // reaped; a still-Funding/Active market is rejected with `NotSettled`.
    let terminal = market.status == MarketStatus::Resolved.as_u8()
        || market.status == MarketStatus::Void.as_u8()
        || market.status == MarketStatus::Cancelled.as_u8();
    if !terminal {
        return Err(MarketError::NotSettled.into());
    }
    // A Resolved/Void market must have had its fee cut finalized before close (so no
    // fee CPI is ever needed here, and `lp_total` — the claim basis — is settled).
    // Cancelled never activated, so it has no fee to collect.
    let activated = market.lp_vault != DEFAULT_PUBKEY;
    if activated && market.fee_collected != 1 {
        return Err(MarketError::FeeNotCollected.into());
    }
    // SAFETY GATE: every contributor must have exited. While any Contribution is
    // live, its owner's LP/refund is still stranded in a to-be-closed account, so
    // closing now would destroy their claim. Permissionless close cannot force a
    // claim, so it simply waits (documented liveness trade-off).
    if market.open_contributions != 0 {
        return Err(MarketError::ContributionsOpen.into());
    }

    // Validate the passed token accounts against the recorded market bindings BEFORE
    // closing anything: escrow always; cyes/cno (via the market-PDA seeds) + lp_vault
    // (recorded) only when activated.
    assert_key(escrow_ai, &market.escrow_vault)?;
    if activated {
        let (expect_cyes, _) =
            Address::find_program_address(&[b"cyes", market_ai.address().as_ref()], program_id);
        let (expect_cno, _) =
            Address::find_program_address(&[b"cno", market_ai.address().as_ref()], program_id);
        assert_key(cyes_ai, &expect_cyes)?;
        assert_key(cno_ai, &expect_cno)?;
        assert_key(lp_vault_ai, &market.lp_vault)?;
    }

    // Market-PDA signer seeds (the token accounts' SPL authority), shared by every
    // CloseAccount CPI: `[b"market", oracle, [outcome_index], [bump]]`.
    market_signer_seeds!(market, oidx, mbump, market_seeds);

    // Close the token accounts (rent → creator) — but only if empty, so a dust
    // donation into a derivable Market-PDA token account can never permanently
    // brick the close (see `close_token_account_if_empty`). On the happy path all
    // four are 0-balance (drained at activate / collect_fee / the last claim_lp).
    close_token_account_if_empty(escrow_ai, creator_ai, market_ai, &market_seeds)?;
    if activated {
        close_token_account_if_empty(cyes_ai, creator_ai, market_ai, &market_seeds)?;
        close_token_account_if_empty(cno_ai, creator_ai, market_ai, &market_seeds)?;
        close_token_account_if_empty(lp_vault_ai, creator_ai, market_ai, &market_seeds)?;
    }

    // Close the Market data PDA: drain its rent → creator + reap. Idempotent by
    // closure — a second call finds the Market gone and fails the load guard.
    close_data_account(market_ai, creator_ai)
}

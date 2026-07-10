//! `close_market` (Task S4): permissionless rent reclaim for one settled
//! challenge [`Market`] PDA + its `challenger_usdc_vault` escrow.
//!
//! The challenge milestone deliberately DEFERRED this: `settle_challenge` sets
//! `market.settled = 1` and drains the escrow USDC, but does NOT close the
//! `Market` PDA or the escrow token account, leaving the challenger's rent
//! locked until a settlement-era close (mirroring the Proposer/Fact non-closure
//! convention). This is that close.
//!
//! # What it does
//! Once the oracle is TERMINAL ([`Phase::Resolved`]/[`Phase::InvalidDeadend`])
//! AND `market.settled == 1` AND the escrow balance is 0 (settle drained it),
//! it:
//! 1. closes the `challenger_usdc_vault` escrow via an SPL `CloseAccount` CPI,
//!    **program-signed by the oracle PDA** (the escrow's token authority), which
//!    sends the escrow's rent lamports to `rent_recipient`; then
//! 2. closes the `Market` PDA (lamport drain → `rent_recipient` + `close()`).
//!
//! Both rents go to `market.challenger` (the rent payer at `open_challenge`).
//! NO token movement beyond the (already-zero) escrow close. Idempotent BY
//! CLOSURE — a second call finds the `Market` reaped and fails the load guard.
//!
//! # Why the escrow close is an SPL CPI (not a lamport drain)
//! `challenger_usdc_vault` is a TOKEN-program-owned account, so the program
//! cannot just zero it: the rent must be reclaimed via the SPL `CloseAccount`
//! instruction, signed by the account's token authority (the oracle PDA). SPL
//! requires a zero balance to close; we assert `amount == 0` first
//! ([`KassandraError::EscrowNotEmpty`]) so the failure is loud and local.
//!
//! # Accounts
//! 0. oracle               — read-only; owned by this program; must be terminal;
//!    the escrow's token authority (re-derived from the payload nonce, signs the
//!    `CloseAccount`).
//! 1. market               — writable; the [`Market`] PDA, CLOSED here;
//!    `market.oracle == oracle`, `market.settled == 1`.
//! 2. challenger_usdc_vault — writable; `== market.challenger_usdc_vault`; SPL
//!    token account with `amount == 0`, CLOSED here.
//! 3. rent_recipient       — writable; `== market.challenger` (both rents).
//! 4. token program.
//!
//! # Instruction payload (after the 1-byte discriminant)
//! `oracle_nonce: u64 LE` (exactly 8 bytes) — re-derives + verifies the oracle
//! PDA signer seeds (`[b"oracle", nonce_le, [bump]]`), identical to the S2
//! claims / `settle_challenge`.

use pinocchio::{
    account::AccountView as AccountInfo, address::Address as Pubkey, cpi::Signer,
    error::ProgramError, ProgramResult,
};
use pinocchio_token::instructions::CloseAccount;
use pinocchio_token::state::Account as TokenAccount;

use crate::{
    error::KassandraError,
    processor::guards::{
        assert_key, assert_owned_by_program, drain_lamports, load_oracle, require_terminal,
        verify_oracle_pda,
    },
    state::{AccountType, Market, Oracle},
};

/// Exact payload length: `oracle_nonce[8]`.
const PAYLOAD_LEN: usize = 8;

pub fn process(program_id: &Pubkey, accounts: &mut [AccountInfo], payload: &[u8]) -> ProgramResult {
    if payload.len() != PAYLOAD_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let nonce = u64::from_le_bytes(payload[0..8].try_into().unwrap());

    let [oracle_ai, market_ai, escrow_ai, rent_recipient_ai, token_prog_ai, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    assert_key(token_prog_ai, &pinocchio_token::ID)?;

    // Oracle must be owned by this program and TERMINAL.
    let oracle = load_oracle(oracle_ai, program_id)?;
    require_terminal(&oracle)?;

    // The oracle PDA is the escrow's token authority; verify it.
    verify_oracle_pda(program_id, oracle_ai, &oracle, nonce)?;

    // Load + bind the Market.
    assert_owned_by_program(market_ai, program_id)?;
    if market_ai.data_len() < Market::LEN {
        return Err(KassandraError::InvalidAccount.into());
    }
    let market: Market = {
        let data = market_ai.try_borrow()?;
        bytemuck::pod_read_unaligned::<Market>(&data[..Market::LEN])
    };
    if market.account_type != AccountType::Market.as_u8() {
        return Err(KassandraError::InvalidAccount.into());
    }
    if &market.oracle != oracle_ai.address() {
        return Err(KassandraError::InvalidAccount.into());
    }
    if !market.is_settled() {
        return Err(KassandraError::MarketNotSettled.into());
    }
    assert_key(escrow_ai, &market.challenger_usdc_vault)?;
    assert_key(rent_recipient_ai, &market.challenger)?;

    // The escrow is a canonical SPL token account; assert it is empty before
    // closing it (settle_challenge drained it). SPL would reject a non-empty
    // close anyway.
    let escrow_amount = TokenAccount::from_account_view(escrow_ai)
        .map_err(|_| KassandraError::InvalidAccount)?
        .amount();
    if escrow_amount != 0 {
        return Err(KassandraError::EscrowNotEmpty.into());
    }

    // 1. Close the escrow token account via SPL CloseAccount, program-signed by
    //    the oracle PDA (its token authority). Rent → rent_recipient.
    let nonce_le = nonce.to_le_bytes();
    let bump_seed = [oracle.bump];
    let seeds = Oracle::signer_seeds(&nonce_le, &bump_seed);
    CloseAccount::new(escrow_ai, rent_recipient_ai, oracle_ai)
        .invoke_signed(&[Signer::from(&seeds)])?;

    // 2. Close the Market PDA: drain its rent lamports → rent_recipient, then
    //    zero it. Idempotent by closure.
    drain_lamports(market_ai, rent_recipient_ai)?;
    market_ai.close()
}

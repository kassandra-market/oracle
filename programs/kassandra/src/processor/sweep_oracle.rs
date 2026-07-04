//! `sweep_oracle` (Task SW1): permissionless, grace-gated dust sweep + terminal
//! `Oracle`/`stake_vault` closure.
//!
//! After all per-staker claims (`claim_proposer`/`claim_fact`/`claim_fact_vote`)
//! have drained a terminal oracle, its `stake_vault` retains only bounded
//! floor/ceil rounding DUST (always under-pay, never short) that no claim can
//! reach — plus, if a staker never claimed, that no-show's forfeited principal.
//! That KASS and the rent of the `Oracle` (≈0.0057 SOL) + `stake_vault`
//! (≈0.0020 SOL) accounts would otherwise be locked forever. This is the reap.
//!
//! # What it does
//! Once the oracle is TERMINAL ([`Phase::Resolved`]/[`Phase::InvalidDeadend`])
//! AND `now >= oracle.phase_ends_at + SWEEP_GRACE`, it:
//! 1. Transfers the ENTIRE residual `stake_vault` balance → the DAO treasury
//!    (the KASS ATA of `Protocol.dao_authority`), via an SPL `Transfer` CPI
//!    **program-signed by the oracle PDA** (the vault's token authority). A zero
//!    balance is a no-op.
//! 2. Closes the (now-empty) `stake_vault` via an SPL `CloseAccount` CPI,
//!    oracle-PDA signed, sending its rent lamports to `oracle.creator`.
//! 3. Closes the `Oracle` PDA (lamport drain → `oracle.creator` + `close()`).
//!
//! Both rents go to `oracle.creator` (the original payer at `create_oracle`),
//! matching the system's "rent → original payer" convention. Idempotent BY
//! CLOSURE — a second call finds the `Oracle` reaped and fails the load guard.
//!
//! # Grace gate — the terminal-time anchor
//! `phase_ends_at` is the terminal-ENTRY anchor: `finalize_oracle` can only drive
//! the oracle terminal at `now >= phase_ends_at` (the challenge window's end) and
//! does NOT advance it. The sweep is therefore gated to a FIXED, publicly known
//! instant — `phase_ends_at + SWEEP_GRACE` — regardless of when the finalize
//! actually landed. (A delayed finalize enters terminal LATER than
//! `phase_ends_at`, which can shrink the window measured from terminal-entry; the
//! guarantee is not a minimum span since terminal-entry but this fixed anchor,
//! computed from `phase_ends_at` — a value published on the oracle at creation.)
//! See the config const.
//!
//! # FORFEITURE TRADE-OFF (starkly documented)
//! There is NO outstanding-claims counter (design decision: grace-forced close,
//! no `Oracle::LEN` change). A staker who has NOT claimed within the generous
//! grace FORFEITS their unclaimed KASS principal — it is swept to the treasury
//! with the dust — and their per-account rent. Their subsequent claim then fails
//! because the `Oracle` is closed. The long grace makes this a genuine
//! abandonment, not a race.
//!
//! # Governance is REQUIRED
//! The treasury is the KASS **associated token account of `Protocol.dao_authority`**
//! (the Squads vault). The sweep therefore REQUIRES `Protocol.governance_set == 1`
//! and VALIDATES `dao_treasury == ATA(dao_authority, kass_mint)` — a wrong
//! treasury is rejected so dust can never be routed to an attacker's account. An
//! oracle cannot be swept until the DAO exists ([`KassandraError::GovernanceNotSet`]).
//!
//! # Accounts
//! 0. oracle       — writable; owned by this program; must be terminal; the
//!    `stake_vault`'s token authority (re-derived from the payload nonce, signs
//!    the `Transfer` + `CloseAccount`). CLOSED here.
//! 1. stake_vault  — writable; `== oracle.stake_vault`; SPL token account whose
//!    full balance is swept, then CLOSED here (rent → creator).
//! 2. protocol     — read-only; the `[b"protocol"]` singleton; supplies
//!    `governance_set` / `dao_authority` / `kass_mint`.
//! 3. dao_treasury — writable; `== ATA(protocol.dao_authority, protocol.kass_mint)`;
//!    the KASS destination for the swept balance.
//! 4. creator      — writable; `== oracle.creator` (both reclaimed rents).
//! 5. token program.
//!
//! # Instruction payload (after the 1-byte discriminant)
//! `oracle_nonce: u64 LE` (exactly 8 bytes) — re-derives + verifies the oracle
//! PDA signer seeds (`[b"oracle", nonce_le, [bump]]`), identical to `claims` /
//! `close_market` / `finalize_oracle`.

use pinocchio::{
    account_info::AccountInfo,
    instruction::{Seed, Signer},
    program_error::ProgramError,
    pubkey::{find_program_address, Pubkey},
    ProgramResult,
};
use pinocchio_pubkey::pubkey;
use pinocchio_token::instructions::{CloseAccount, Transfer};

use crate::{
    clock::now,
    config::SWEEP_GRACE,
    error::KassandraError,
    processor::guards::{assert_key, load_oracle, load_protocol, verify_oracle_pda},
    state::Phase,
};

/// Exact payload length: `oracle_nonce[8]`.
const PAYLOAD_LEN: usize = 8;

/// Minimum size of an SPL token account (`spl_token::state::Account::LEN`).
const SPL_TOKEN_ACCOUNT_LEN: usize = 165;
/// `spl_token::state::Account.amount` byte offset.
const SPL_TOKEN_AMOUNT_OFFSET: usize = 64;

/// SPL Associated Token Account program id. The DAO treasury is the canonical
/// KASS ATA of `dao_authority`, derived under this program from the standard
/// seeds `[owner, TOKEN_PROGRAM, mint]`.
const ATA_PROGRAM_ID: Pubkey = pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], payload: &[u8]) -> ProgramResult {
    if payload.len() != PAYLOAD_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let nonce = u64::from_le_bytes(payload[0..8].try_into().unwrap());

    let [oracle_ai, stake_vault_ai, protocol_ai, dao_treasury_ai, creator_ai, token_prog_ai, ..] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    assert_key(token_prog_ai, &pinocchio_token::ID)?;

    // Oracle must be owned by this program and TERMINAL.
    let oracle = load_oracle(oracle_ai, program_id)?;
    match oracle.phase().ok_or(KassandraError::InvalidAccount)? {
        Phase::Resolved | Phase::InvalidDeadend => {}
        _ => return Err(KassandraError::WrongPhase.into()),
    }

    // The oracle PDA is the vault's token authority; verify it, then bind the
    // vault + rent recipient.
    verify_oracle_pda(program_id, oracle_ai, &oracle, nonce)?;
    assert_key(stake_vault_ai, &oracle.stake_vault)?;
    assert_key(creator_ai, &oracle.creator)?;

    // Grace gate: the reap is delayed a generous window past the terminal-entry
    // anchor so honest claimants have ample time to claim. `checked_add` guards
    // the (attacker-uncontrolled, but defensively bounded) timestamp arithmetic.
    let grace_end = oracle
        .phase_ends_at
        .checked_add(SWEEP_GRACE)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    if now()? < grace_end {
        return Err(KassandraError::SweepGraceNotElapsed.into());
    }

    // The treasury is the DAO's KASS ATA, so governance must be set; then pin the
    // passed treasury to the canonical `ATA(dao_authority, kass_mint)`.
    let protocol = load_protocol(protocol_ai, program_id)?;
    if !protocol.is_governance_set() {
        return Err(KassandraError::GovernanceNotSet.into());
    }
    let (treasury, _) = find_program_address(
        &[
            protocol.dao_authority.as_ref(),
            pinocchio_token::ID.as_ref(),
            protocol.kass_mint.as_ref(),
        ],
        &ATA_PROGRAM_ID,
    );
    if dao_treasury_ai.key() != &treasury {
        return Err(KassandraError::InvalidTreasury.into());
    }

    // Read the residual vault balance (SPL token account, amount at offset 64).
    if !stake_vault_ai.is_owned_by(&pinocchio_token::ID) {
        return Err(KassandraError::InvalidAccount.into());
    }
    let amount = {
        let data = stake_vault_ai.try_borrow_data()?;
        if data.len() < SPL_TOKEN_ACCOUNT_LEN {
            return Err(KassandraError::InvalidAccount.into());
        }
        u64::from_le_bytes(
            data[SPL_TOKEN_AMOUNT_OFFSET..SPL_TOKEN_AMOUNT_OFFSET + 8]
                .try_into()
                .unwrap(),
        )
    };

    // Oracle-PDA signer seeds (`[b"oracle", nonce_le, [bump]]`), reused for the
    // Transfer + CloseAccount.
    let nonce_le = nonce.to_le_bytes();
    let bump_seed = [oracle.bump];
    let seeds = [
        Seed::from(b"oracle".as_ref()),
        Seed::from(nonce_le.as_ref()),
        Seed::from(&bump_seed),
    ];

    // 1. Sweep the ENTIRE residual balance → treasury (dust, or a no-show's
    //    forfeited principal). A zero balance is a no-op.
    if amount > 0 {
        Transfer {
            from: stake_vault_ai,
            to: dao_treasury_ai,
            authority: oracle_ai,
            amount,
        }
        .invoke_signed(&[Signer::from(&seeds)])?;
    }

    // 2. Close the now-empty vault via SPL CloseAccount, oracle-PDA signed. Rent
    //    → creator. SPL requires a zero balance, which step 1 guarantees.
    CloseAccount {
        account: stake_vault_ai,
        destination: creator_ai,
        authority: oracle_ai,
    }
    .invoke_signed(&[Signer::from(&seeds)])?;

    // 3. Close the Oracle PDA: drain its rent lamports → creator, then zero it.
    //    Idempotent by closure.
    {
        let mut from = oracle_ai.try_borrow_mut_lamports()?;
        let mut to = creator_ai.try_borrow_mut_lamports()?;
        *to = to
            .checked_add(*from)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        *from = 0;
    }
    oracle_ai.close()
}

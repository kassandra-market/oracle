//! `close_ai_claim` (Task S4): permissionless, post-resolution rent reclaim for
//! one [`AiClaim`] account.
//!
//! An [`AiClaim`] holds NO tokens — it is a pinned-model commitment record — so
//! this instruction performs NO token movement. Once the oracle is TERMINAL
//! ([`Phase::Resolved`] or [`Phase::InvalidDeadend`]) the claim is dead weight;
//! anyone may crank this to drain its rent lamports to the proposer's human
//! authority and CLOSE it. Idempotent BY CLOSURE — a second call finds the
//! account reaped (zero lamports → owner no longer the program) and fails the
//! load guard.
//!
//! # Rent recipient binding — ORDER-INDEPENDENT (no Proposer dependency)
//! The rent is reclaimed to `ai_claim.authority`, the proposer's human authority
//! STAMPED on the [`AiClaim`] at submit time (`submit_ai_claim`). Because the
//! recipient is read off the claim itself, this instruction does NOT load the
//! `Proposer` account and works regardless of whether `claim_proposer` has
//! already closed that Proposer. The AiClaim's rent therefore can never be
//! stranded by claim ordering — `close_ai_claim` and `claim_proposer` may run in
//! either order.
//!
//! # Accounts
//! 0. oracle         — read-only; owned by this program; must be terminal.
//! 1. ai_claim       — writable; the [`AiClaim`] account, CLOSED here;
//!    `ai_claim.oracle == oracle`.
//! 2. rent_recipient — writable; `== ai_claim.authority` (reclaimed rent).
//!
//! # Instruction payload
//! None (exactly 0 bytes after the discriminant). No PDA signature is needed —
//! the close is a pure lamport drain on a program-owned account.

use pinocchio::{
    account::AccountView as AccountInfo, address::Address as Pubkey, error::ProgramError,
    ProgramResult,
};

use crate::{
    error::KassandraError,
    processor::guards::{assert_key, drain_lamports, load_ai_claim, load_oracle, require_terminal},
};

pub fn process(program_id: &Pubkey, accounts: &mut [AccountInfo], payload: &[u8]) -> ProgramResult {
    if !payload.is_empty() {
        return Err(ProgramError::InvalidInstructionData);
    }

    let [oracle_ai, ai_claim_ai, rent_recipient_ai, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // Oracle must be owned by this program and TERMINAL.
    let oracle = load_oracle(oracle_ai, program_id)?;
    require_terminal(&oracle)?;

    // Bind the AiClaim to this oracle; pay rent to its stamped authority.
    let ai_claim = load_ai_claim(ai_claim_ai, program_id)?;
    if &ai_claim.oracle != oracle_ai.address() {
        return Err(KassandraError::InvalidAccount.into());
    }
    assert_key(rent_recipient_ai, &ai_claim.authority)?;

    // Drain rent lamports → recipient, then zero the account (data / lamports /
    // owner). Idempotent: a second call finds it reaped.
    drain_lamports(ai_claim_ai, rent_recipient_ai)?;
    ai_claim_ai.close()
}

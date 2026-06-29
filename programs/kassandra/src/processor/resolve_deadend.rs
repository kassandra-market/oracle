//! `resolve_deadend`: DAO-gated resolution of a dead-ended oracle (Task F4).
//!
//! An oracle reaches [`Phase::InvalidDeadend`] when the dispute core cannot
//! decide it (e.g. a tie or no surviving proposers) — a terminal failure the
//! market/AI could not break. The design makes this state "fixable only by KASS
//! governance": a passed v0.6 futarchy proposal, executing through the Squads v4
//! multisig vault recorded as `Protocol.dao_authority`, supplies the final
//! categorical outcome.
//!
//! This instruction does exactly that and nothing more: it stamps
//! `oracle.resolved_option` and advances the phase
//! [`Phase::InvalidDeadend`] → [`Phase::Resolved`].
//!
//! # Deferred: economic settlement
//! There is intentionally **NO token movement here.** The economic settlement of
//! a governance-resolved dead-end (whose shape is: stakes likely returned, no
//! rewards — the market/AI did NOT decide the outcome, so nobody earned a slash
//! or a reward) is DEFERRED to the settlement milestone. This milestone only
//! records the terminal outcome; the settlement processor will read
//! `phase == Resolved` + `resolved_option` and move funds then.
//!
//! # Idempotency
//! A second call fails [`require_phase(InvalidDeadend)`](require_phase): after the
//! first call the phase is `Resolved`, so the re-entry returns
//! [`KassandraError::WrongPhase`].
//!
//! # Accounts
//! 0. protocol PDA  — read-only; the `[b"protocol"]` singleton (read `dao_authority`)
//! 1. oracle        — writable; the dead-ended oracle to resolve
//! 2. dao_authority — signer; must equal `protocol.dao_authority`
//!
//! # Instruction payload (after the 1-byte discriminant), exactly 1 byte
//! `option: u8` — the winning categorical option; must be `< oracle.options_count`.

use pinocchio::{
    account_info::AccountInfo, program_error::ProgramError, pubkey::Pubkey, ProgramResult,
};

use crate::{
    clock::require_phase,
    error::KassandraError,
    processor::guards::{assert_dao_authority, load_oracle, load_protocol},
    state::{Oracle, Phase},
};

pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], payload: &[u8]) -> ProgramResult {
    let [protocol_ai, oracle_ai, dao_authority_ai, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // --- payload parse (exact length): a single categorical option ----------
    let [option] = payload else {
        return Err(ProgramError::InvalidInstructionData);
    };
    let option = *option;

    // --- gate: DAO authority signs (load_protocol pins the singleton) -------
    let protocol = load_protocol(protocol_ai, program_id)?;
    assert_dao_authority(&protocol, dao_authority_ai)?;

    // --- load + validate the oracle -----------------------------------------
    let mut oracle: Oracle = load_oracle(oracle_ai, program_id)?;

    // Only a dead-ended oracle may be governance-resolved (also the idempotency
    // guard: a second call sees `Resolved` and fails here).
    require_phase(&oracle, Phase::InvalidDeadend)?;

    // The chosen option must be a valid categorical index for this oracle.
    if option >= oracle.options_count {
        return Err(KassandraError::InvalidOptionsCount.into());
    }

    // --- effect: stamp the terminal outcome (NO token movement) -------------
    oracle.resolved_option = option;
    oracle.set_phase(Phase::Resolved);
    {
        let mut data = oracle_ai.try_borrow_mut_data()?;
        data[..Oracle::LEN].copy_from_slice(bytemuck::bytes_of(&oracle));
    }

    Ok(())
}

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
//! # NO token movement here — the dead-end was already settled at finalize
//! There is intentionally **NO token movement here**, and crucially none is
//! needed: a governance-resolved dead-end pays out IDENTICALLY to a plain
//! `InvalidDeadend` — stakers reclaim their **non-slashed principal**, with NO
//! reward (the market/AI did not decide the outcome, so nobody earned a slash or
//! a reward). That holds because the dead-end's misrouted funds were ALREADY
//! burned out of `stake_vault` at the InvalidDeadend finalize site (`finalize_
//! oracle` / `finalize_no_facts` burn the slashed `bond_pool` + the
//! `reward_emission`), leaving the vault holding exactly the returnable principal,
//! and because `reward_pool == 0` on a dead-end makes every S2 reward term 0 on
//! BOTH terminal phases. So this instruction only records the terminal outcome
//! (`resolved_option`) for downstream consumers; the S2 pull-claims then drain the
//! vault to dust regardless of whether the phase is `InvalidDeadend` or this
//! governance-flipped `Resolved`. (Earlier revisions of this doc claimed F4
//! settlement was "deferred / pays stakes-back only with no special-casing"; the
//! burn now lives at finalize, so the claim path needs no F4 branch.)
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
    account::AccountView as AccountInfo, address::Address as Pubkey, error::ProgramError,
    ProgramResult,
};

use crate::{
    clock::require_phase,
    error::KassandraError,
    processor::guards::{assert_dao_authority, load_oracle, load_protocol},
    state::{Oracle, Phase},
};

pub fn process(program_id: &Pubkey, accounts: &mut [AccountInfo], payload: &[u8]) -> ProgramResult {
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
        let mut data = oracle_ai.try_borrow_mut()?;
        data[..Oracle::LEN].copy_from_slice(bytemuck::bytes_of(&oracle));
    }

    Ok(())
}

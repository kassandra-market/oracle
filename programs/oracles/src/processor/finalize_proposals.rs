//! `finalize_proposals`: close the proposal window, resolving the oracle if the
//! proposers all agree or otherwise opening the dispute (design §3, §7).
//!
//! Runs once, after the [`Phase::Proposal`] window has elapsed
//! (`now >= phase_ends_at`). It reads the FULL proposer set's
//! `original_option`s and decides:
//! * **All equal** → [`Phase::Resolved`], `oracle.resolved_option = that option`.
//!   The uncontested value is the oracle's answer; nobody disputed it.
//! * **≥2 distinct** → open the dispute: set `oracle.dispute_bond_total =
//!   oracle.total_oracle_stake` (Σ proposer bonds), advance to
//!   [`Phase::FactProposal`], and arm a fresh `phase_ends_at = now +
//!   PHASE_WINDOW`. This is the seam into the already-built dispute core
//!   (`submit_fact` onward).
//!
//! # `dispute_bond_total` handoff (CONTRACT)
//! The dispute core (`finalize_facts`) uses `dispute_bond_total` as the FIXED
//! fact-quorum denominator and rejects a zero value ([`KassandraError::NoDisputeBond`]).
//! At end-of-Proposal no facts/votes have accrued, so `total_oracle_stake` is
//! exactly Σ proposer bonds — the correct denominator. We snapshot it into
//! `dispute_bond_total` here so the dispute starts from the state the core
//! expects: proposers locked in, bond-total fixed, `phase_ends_at` a FactProposal
//! window end.
//!
//! # One-shot full-proposer-set proof
//! Mirrors `finalize_oracle`: the caller must pass EVERY proposer account in one
//! transaction. `tail.len() == proposer_count`, each account distinct,
//! program-owned, tagged [`AccountType::Proposer`], and belonging to THIS oracle
//! — so the decision provably saw the whole set. `MAX_PROPOSERS` is the defensive
//! cap (the real liveness guarantee is the registration cap in `propose`).
//!
//! # Idempotency
//! Runs exactly once: the phase leaves `Proposal` (to Resolved or FactProposal),
//! so a second call fails `require_phase(Proposal)` with
//! [`KassandraError::WrongPhase`].
//!
//! # No token CPI / deferred settlement (design §7)
//! Like every instruction in this milestone, finalize_proposals performs NO token
//! CPI: it records the terminal/next phase only. On an all-agree resolve, the
//! proposers' bonds stay escrowed in the stake vault; per-staker bond return is a
//! DEFERRED settlement task, consistent with the counter-only convention used by
//! `finalize_facts` / `finalize_oracle`.
//!
//! # Accounts
//! 0. oracle — writable, owned by this program (the ONLY account mutated).
//! 1. onward — the FULL proposer set: exactly `proposer_count` accounts, each
//!    READ-ONLY (finalize only reads `original_option`), owned by this program,
//!    tagged Proposer, belonging to this oracle, distinct within the call.
//!
//! # Instruction payload
//! Empty (after the 1-byte discriminant).

use pinocchio::{
    account::AccountView as AccountInfo, address::Address as Pubkey, error::ProgramError,
    ProgramResult,
};

use crate::{
    clock::{now, require_after_end, require_phase},
    error::KassandraError,
    processor::guards::{load_oracle, load_proposer, require_distinct},
    state::{Oracle, Phase},
};

/// Defensive upper bound on the proposer set this one-shot call will read. Lives
/// in [`crate::config::MAX_PROPOSERS`] so `propose` (the registration cap, the
/// real liveness guarantee) and the finalizers share one constant. Mirrors
/// `finalize_oracle`'s backstop against an oversized, unfinalizable set.
const MAX_PROPOSERS: usize = crate::config::MAX_PROPOSERS as usize;

pub fn process(
    program_id: &Pubkey,
    accounts: &mut [AccountInfo],
    _payload: &[u8],
) -> ProgramResult {
    let [oracle_ai, tail @ ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // Owner + size + account_type check, then an owned copy for mutation.
    let mut oracle: Oracle = load_oracle(oracle_ai, program_id)?;

    require_phase(&oracle, Phase::Proposal)?;
    let now_ts = now()?;
    require_after_end(&oracle, now_ts)?;

    // Nothing to finalize: an empty oracle stays open for its first proposal
    // (the empty-window seeding handled by `propose`).
    if oracle.proposer_count == 0 {
        return Err(KassandraError::NoProposals.into());
    }

    // One-shot: the FULL proposer set must be supplied in this single call.
    if tail.len() != oracle.proposer_count as usize {
        return Err(KassandraError::InvalidAccount.into());
    }
    if tail.len() > MAX_PROPOSERS {
        // Defensive backstop; the registration cap in `propose` is the real
        // liveness guarantee (see the MAX_PROPOSERS config CONTRACT).
        return Err(KassandraError::InvalidAccount.into());
    }

    // Walk the full set, proving each membership and tracking whether every
    // proposer's `original_option` is identical. All-agree is simpler than the
    // plurality recompute: no vote buffer, just "are all options equal."
    let mut first_option: Option<u8> = None;
    let mut all_agree = true;
    for (i, p_ai) in tail.iter().enumerate() {
        require_distinct(&tail[..i], p_ai.address())?;

        let proposer = load_proposer(p_ai, program_id)?;
        if proposer.oracle != *oracle_ai.address() {
            return Err(KassandraError::InvalidAccount.into());
        }
        match first_option {
            None => first_option = Some(proposer.original_option),
            Some(f) => {
                if proposer.original_option != f {
                    all_agree = false;
                }
            }
        }
    }

    // `proposer_count >= 1` and `tail.len() == proposer_count`, so the loop ran
    // at least once and `first_option` is set; the `ok_or` is a defensive guard.
    let agreed_option = first_option.ok_or(KassandraError::NoProposals)?;

    if all_agree {
        // Uncontested: the single shared value is the oracle's answer. Every
        // proposer agreed on the winning option, so ALL of them are "correct" —
        // distribute the pre-minted `reward_emission` to them pro-rata by bond
        // via the S2 `claim_proposer`, exactly as the disputed `finalize_oracle`
        // Resolved path rewards the correct cohort. Without stamping the pool +
        // denominator here the emission would strand in the vault and be swept to
        // the DAO treasury. No facts/votes exist on this path, so
        // `total_oracle_stake == Σ proposer bonds` == the correct cohort, and
        // `bond_pool == 0`.
        oracle.resolved_option = agreed_option;
        oracle.reward_pool = oracle
            .bond_pool
            .checked_add(oracle.reward_emission)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        oracle.total_correct_proposer_stake = oracle.total_oracle_stake;
        oracle.set_phase(Phase::Resolved);
    } else {
        // Conflict: open the dispute. Snapshot Σ bonds as the fixed fact-quorum
        // denominator the dispute core requires, then hand off to FactProposal
        // with a fresh window.
        oracle.dispute_bond_total = oracle.total_oracle_stake;
        oracle.set_phase(Phase::FactProposal);
        oracle.phase_ends_at = now_ts
            .checked_add(oracle.phase_window)
            .ok_or(ProgramError::ArithmeticOverflow)?;
    }

    let mut data = oracle_ai.try_borrow_mut()?;
    data[..Oracle::LEN].copy_from_slice(bytemuck::bytes_of(&oracle));
    Ok(())
}

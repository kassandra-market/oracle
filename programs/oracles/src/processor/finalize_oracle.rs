//! `finalize_oracle`: the final plurality recompute that drives the oracle into
//! a terminal state (design §6, §7).
//!
//! Runs once, after the [`Phase::Challenge`] window has elapsed AND every
//! challenge decision market has settled. It recomputes the plurality over the
//! SURVIVING proposers (Task 8's pure [`plurality`]) and writes the terminal
//! phase:
//! * [`Plurality::Winner`]`(opt)` → [`Phase::Resolved`], `oracle.resolved_option
//!   = opt`.
//! * [`Plurality::Tie`] → [`Phase::InvalidDeadend`], `resolved_option =
//!   CLAIM_OPTION_NONE`.
//! * [`Plurality::NoSurvivors`] (every proposer disqualified) →
//!   [`Phase::InvalidDeadend`], `resolved_option = CLAIM_OPTION_NONE`.
//!
//! CONTRACT: `oracle.resolved_option` is the winning option ONLY when
//! `phase == Resolved`. On [`Phase::InvalidDeadend`] it is set to the loud
//! [`CLAIM_OPTION_NONE`] sentinel so a consumer that forgets to gate on the
//! phase reads `0xFF` instead of a plausible "option 0 won."
//!
//! # One-shot (NOT incremental)
//! Unlike `finalize_facts` / `finalize_ai_claims`, the plurality needs the WHOLE
//! surviving set at once, so finalize_oracle is one-shot: the caller must pass
//! every proposer account in a single transaction (`tail.len() ==
//! proposer_count`). The full set is therefore bounded by Solana's per-tx
//! account-lock limit — fine, since a dispute's proposer set is small. The
//! survivor votes are gathered into a fixed stack buffer (no heap, matching the
//! rest of the program); [`MAX_PROPOSERS`] caps it.
//!
//! # CONTRACT: `proposer_count` must stay finalizable
//! finalize_oracle is one-shot, so the WHOLE proposer set must fit in a single
//! transaction's account-lock budget. The `propose` processor caps
//! `proposer_count` at a value that fits one finalize transaction — see
//! [`MAX_PROPOSERS`]. The `tail.len() > MAX_PROPOSERS` check
//! below is a DEFENSIVE backstop against buffer overflow, NOT the liveness
//! guarantee: without a registration cap, an oversized proposer set could never
//! be finalized and would brick the oracle in [`Phase::Challenge`]. Task 13's
//! fuzzer must stay within this cap.
//!
//! # Gating
//! * [`Phase::Challenge`] (the only entry; `FinalRecompute` is reserved/unused —
//!   we transition Challenge → terminal directly).
//! * `now >= phase_ends_at` (the challenge window has closed).
//! * `oracle.open_challenge_count == 0` — every challenged claim has been settled
//!   by `settle_challenge`; otherwise a challenged-but-unsettled proposer is not
//!   yet disqualified and would be wrongly counted as surviving
//!   ([`KassandraError::ChallengesOutstanding`]).
//!
//! # Consistency guards
//! * `tail.len() == proposer_count` and each account is distinct, program-owned,
//!   tagged [`AccountType::Proposer`], and belongs to THIS oracle — so the full
//!   proposer set is provably present.
//! * The number of non-disqualified proposers collected MUST equal
//!   `oracle.surviving_count` — a state-consistency check that also confirms no
//!   survivor was omitted. A mismatch is [`KassandraError::InvalidAccount`].
//! * A non-disqualified proposer with `claim_option == CLAIM_OPTION_NONE` is an
//!   invariant violation (a no-show is disqualified in `finalize_ai_claims`
//!   before this point), rejected as [`KassandraError::InvalidAccount`].
//!
//! # Idempotency
//! Runs exactly once: the phase becomes terminal (Resolved / InvalidDeadend), so
//! a second call fails `require_phase(Challenge)` with
//! [`KassandraError::WrongPhase`].
//!
//! # Token CPI: InvalidDeadend burn-back (emission + slashed bond_pool)
//! On the Resolved branch finalize_oracle stamps the resolution totals the S2
//! pull-claims read — `total_correct_proposer_stake` (Σ surviving-correct bonds)
//! and `reward_pool = bond_pool + reward_emission` (S3 folds the creation-time
//! emission in) — with NO token movement. On the InvalidDeadend branch it BURNS
//! BOTH the `reward_emission` AND the slashed `bond_pool` back from `stake_vault`
//! (program-signed by the oracle PDA) to the supply reservoir: a dead-end is a
//! non-outcome, so the emission funds no reward and the slashed amounts have no
//! recipient (no winner) — both are burned, leaving the vault holding EXACTLY the
//! returnable non-slashed principal (`Σ surviving bonds − flip slashes + agreed/
//! duplicate fact stakes + un-slashed approve-voter stakes`), which the S2 claims
//! drain to dust. Burning `bond_pool` is conservation-safe: it equals Σ
//! `slashed_amount` over the slashed accounts, and any `kass_fee` already paid OUT
//! to a challenger by `settle_challenge` was recorded as `bond − kass_fee` (so it
//! is NOT in `bond_pool` and is not double-burned). AiClaim-account rent
//! reclamation (the design's "close AiClaim accounts on resolution") is a
//! SEPARATE permissionless per-claim instruction ([`crate::processor::close_ai_claim`],
//! callable post-resolution): it has the same one-tx capacity concern as
//! finalize, so it is not crammed into this recompute, and finalize does not
//! block on it.
//!
//! # Accounts
//! 0. oracle        — writable, owned by this program (mutated; signs the burn-back).
//! 1. kass_mint     — writable; `== oracle.kass_mint` (the InvalidDeadend burn-back target).
//! 2. stake_vault   — writable; `== oracle.stake_vault` (emission burned from here).
//! 3. token program — `pinocchio_token::ID`.
//! 4. onward        — the FULL proposer set: exactly `proposer_count` accounts, each
//!    READ-ONLY (finalize only reads `claim_option` / `disqualified`), owned by
//!    this program, tagged Proposer, belonging to this oracle, distinct within
//!    the call. Read-only avoids write-lock contention and raises the practical
//!    per-tx account ceiling for the one-shot finalize.
//!
//! NOTE: the fixed burn accounts (1-3) are required on BOTH terminal branches
//! even though only InvalidDeadend with `reward_emission + bond_pool > 0`
//! actually burns — the account layout is fixed, and validating the canonical
//! mint/vault is cheap.
//!
//! # Instruction payload (after the 1-byte discriminant), exactly 8 bytes
//! `oracle_nonce: u64 LE` — re-derives + verifies the oracle PDA, whose seeds
//! `[b"oracle", nonce_le, bump]` program-sign the InvalidDeadend emission burn.

use pinocchio::{
    account::AccountView as AccountInfo, address::Address as Pubkey, cpi::Signer,
    error::ProgramError, ProgramResult,
};
use pinocchio_token::instructions::Burn;

use crate::{
    clock::{now, require_after_end, require_phase},
    error::KassandraError,
    plurality::{plurality, Plurality},
    processor::guards::{
        assert_key, load_oracle, load_proposer, require_distinct, verify_oracle_pda,
    },
    state::{Oracle, Phase, CLAIM_OPTION_NONE},
};

/// Exact payload length: `oracle_nonce[8]` (re-derives the oracle PDA signer for
/// the InvalidDeadend emission burn-back).
const PAYLOAD_LEN: usize = 8;

/// Upper bound on the proposer set finalize_oracle will gather votes for. Lives
/// in [`crate::config::MAX_PROPOSERS`] so `propose` (the registration cap, the
/// real liveness guarantee) and `finalize_oracle` (this defensive backstop that
/// keeps the fixed `votes` buffer from overflowing) share one constant. See the
/// config doc + module-level CONTRACT note. Task 13's fuzzer must stay within it.
const MAX_PROPOSERS: usize = crate::config::MAX_PROPOSERS as usize;

pub fn process(program_id: &Pubkey, accounts: &mut [AccountInfo], payload: &[u8]) -> ProgramResult {
    let [oracle_ai, rest @ ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // Owner + size + account_type check, then an owned copy for mutation. Done
    // BEFORE the payload/fixed-account parse so a bad-owner oracle still fails
    // with `InvalidAccount` (dispatch-routing contract).
    let mut oracle: Oracle = load_oracle(oracle_ai, program_id)?;

    require_phase(&oracle, Phase::Challenge)?;
    let now = now()?;
    require_after_end(&oracle, now)?;

    // Every challenged claim must have settled, else an unsettled (and thus
    // not-yet-disqualified) challenged proposer would be miscounted as surviving.
    if oracle.open_challenge_count != 0 {
        return Err(KassandraError::ChallengesOutstanding.into());
    }

    // Payload nonce → re-derive + verify the oracle PDA (its seeds sign the
    // InvalidDeadend emission burn-back), exactly like the S2 claims.
    if payload.len() != PAYLOAD_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let nonce = u64::from_le_bytes(payload[0..8].try_into().unwrap());
    verify_oracle_pda(program_id, oracle_ai, &oracle, nonce)?;

    // Fixed burn accounts (canonical mint + vault + token program), then the
    // FULL proposer set as the read-only tail.
    let [kass_mint_ai, stake_vault_ai, token_prog_ai, tail @ ..] = rest else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    assert_key(token_prog_ai, &pinocchio_token::ID)?;
    assert_key(kass_mint_ai, &oracle.kass_mint)?;
    assert_key(stake_vault_ai, &oracle.stake_vault)?;

    // One-shot: the FULL proposer set must be supplied in this single call.
    if tail.len() != oracle.proposer_count as usize {
        return Err(KassandraError::InvalidAccount.into());
    }
    if tail.len() > MAX_PROPOSERS {
        // Defensive backstop so the fixed votes buffer can never overflow. A
        // registration cap (future propose processor) is the real liveness
        // guarantee; see the module + MAX_PROPOSERS CONTRACT notes.
        return Err(KassandraError::InvalidAccount.into());
    }

    // Gather the surviving proposers' claim_options (one proposer = one vote)
    // and their bonds in parallel (the bond is the pro-rata weight used to stamp
    // `total_correct_proposer_stake` once the winning option is known).
    let mut votes = [0u8; MAX_PROPOSERS];
    let mut bonds = [0u64; MAX_PROPOSERS];
    let mut n = 0usize;
    for (i, p_ai) in tail.iter().enumerate() {
        require_distinct(&tail[..i], p_ai.address())?;

        let proposer = load_proposer(p_ai, program_id)?;
        if proposer.oracle != *oracle_ai.address() {
            return Err(KassandraError::InvalidAccount.into());
        }
        if proposer.is_disqualified() {
            continue;
        }
        // A surviving proposer always carries a real claim_option (no-shows were
        // disqualified in finalize_ai_claims). CLAIM_OPTION_NONE here is an
        // invariant violation, never a vote for option 0xFF.
        if proposer.claim_option == CLAIM_OPTION_NONE {
            return Err(KassandraError::InvalidAccount.into());
        }
        votes[n] = proposer.claim_option;
        bonds[n] = proposer.bond;
        n += 1;
    }

    // Consistency: the survivors counted must match the oracle's running tally.
    // This also proves no surviving proposer was omitted from the call.
    if n != oracle.surviving_count as usize {
        return Err(KassandraError::InvalidAccount.into());
    }

    match plurality(&votes[..n]) {
        Plurality::Winner(opt) => {
            // Stamp the resolution totals for the S2 pull-claims (Task S1; NO
            // token movement). `total_correct_proposer_stake` = Σ bond over the
            // SURVIVORS whose vote is the winning option (the pro-rata
            // denominator for the proposer reward bucket).
            let mut total_correct: u64 = 0;
            for i in 0..n {
                if votes[i] == opt {
                    total_correct = total_correct
                        .checked_add(bonds[i])
                        .ok_or(ProgramError::ArithmeticOverflow)?;
                }
            }
            oracle.total_correct_proposer_stake = total_correct;
            // Finalize the distributable reward pool (S3): `reward_pool =
            // bond_pool + reward_emission`. The creation-time emission is already
            // physically in `stake_vault`, so it joins the reward pool here.
            // `total_approved_fact_stake` was already accumulated incrementally by
            // finalize_facts.
            oracle.reward_pool = oracle
                .bond_pool
                .checked_add(oracle.reward_emission)
                .ok_or(ProgramError::ArithmeticOverflow)?;
            oracle.resolved_option = opt;
            oracle.set_phase(Phase::Resolved);
        }
        // A tie has no plurality winner, and zero survivors means every proposer
        // was disqualified: both are terminal dead-ends (design §7). Stamp the
        // loud sentinel so a consumer that skips the phase gate never misreads
        // the dead-end as "option 0 won." On a dead-end there is no reward
        // distribution: `reward_pool` / `total_correct_proposer_stake` stay 0
        // (their zeroed default; finalize_oracle runs once).
        Plurality::Tie | Plurality::NoSurvivors => {
            oracle.resolved_option = CLAIM_OPTION_NONE;
            oracle.set_phase(Phase::InvalidDeadend);
            // Burn BOTH the creation-time emission AND the slashed `bond_pool`
            // back to the reservoir so a dead-end strands nothing: a dead-end is
            // a non-outcome with no recipient for slashed amounts (no winner), so
            // they are burned like the creator fee, and the emission funds no
            // reward. After the burn the vault holds EXACTLY the returnable
            // non-slashed principal, which the S2 claims drain to dust. Both sit
            // in `stake_vault` (token authority == the oracle PDA), so the burn
            // is signed by the oracle seeds. `reward_pool` stays 0 (no
            // distribution); `bond_pool`/`reward_emission` are left as the durable
            // record of what was slashed/minted then burned.
            let burn_amount = oracle
                .reward_emission
                .checked_add(oracle.bond_pool)
                .ok_or(ProgramError::ArithmeticOverflow)?;
            if burn_amount > 0 {
                let nonce_le = nonce.to_le_bytes();
                let bump_seed = [oracle.bump];
                let seeds = Oracle::signer_seeds(&nonce_le, &bump_seed);
                Burn::new(stake_vault_ai, kass_mint_ai, oracle_ai, burn_amount)
                    .invoke_signed(&[Signer::from(&seeds)])?;
            }
        }
    }

    let mut data = oracle_ai.try_borrow_mut()?;
    data[..Oracle::LEN].copy_from_slice(bytemuck::bytes_of(&oracle));
    Ok(())
}

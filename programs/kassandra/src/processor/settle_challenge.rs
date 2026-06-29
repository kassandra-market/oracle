//! `settle_challenge`: settle ONE challenged claim's decision market after its
//! TWAP window, applying the slash trigger and resolving the MetaDAO question.
//!
//! Settlement is **incremental** across markets (design §6): each call settles
//! exactly one [`Market`] and updates oracle state immediately. The phase STAYS
//! [`Phase::Challenge`]; the final plurality recompute + phase transition is
//! Task 12's `finalize_oracle`, which runs after every market has settled.
//!
//! # Hard AMM verification (the deferred Task-10 requirement)
//! `open_challenge` only checked `owner == AMM_ID` on the two AMMs (the v0.4 AMM
//! layout was not yet pinned). Here we read the real `Amm` layout (see the
//! offset consts in [`crate::cpi::metadao`]) and BIND each AMM to THIS market's
//! conditional mint pair:
//! * `pass_amm.base_mint  == conditional_token_mint_pda(kass_vault, 0)` (pass-KASS)
//! * `pass_amm.quote_mint == conditional_token_mint_pda(usdc_vault, 0)` (pass-USDC)
//! * `fail_amm.base_mint  == conditional_token_mint_pda(kass_vault, 1)` (fail-KASS)
//! * `fail_amm.quote_mint == conditional_token_mint_pda(usdc_vault, 1)` (fail-USDC)
//!
//! and require `pass_amm != fail_amm`. A challenger therefore cannot point
//! settlement at an AMM they control: the TWAP read is on the canonical pass/fail
//! pools of this exact market.
//!
//! # TWAP read (no crank needed at settle)
//! The v0.4 AMM stores a slot-weighted price aggregator. `get_twap()` in the AMM
//! source is `aggregator / (last_updated_slot - created_at_slot)` — already the
//! finalized time-weighted average; we read those stored fields directly. We do
//! NOT crank here: cranking only matters to *fold in* the most recent price
//! before reading, but (a) a crank only records once per `ONE_MINUTE_IN_SLOTS`
//! and (b) the design's manipulation resistance comes precisely from NOT letting
//! a last-moment observation dominate the window average. Trading parties (or a
//! permissionless cranker) keep the observation fresh during the window; settle
//! consumes the stored average. If a market never traded (`aggregator == 0` or
//! zero slots elapsed) its TWAP reads as `0` — "challenge market with no
//! counter-trading → claim survives" (design §7).
//!
//! # Slash trigger (design §6, invariant §9.8)
//! Disqualify iff `fail_twap > pass_twap + threshold`, with the protocol-global
//! relative margin from [`crate::config`]: `fail_twap * DEN > pass_twap * (DEN +
//! NUM)`, computed in `u128` (the TWAPs are already `u128`).
//! * **Disqualified (fraud):** `proposer.disqualified = slashed = 1`; the
//!   proposer's full bond (split into conditional KASS at `open_challenge`) is
//!   forfeit to `oracle.bond_pool`; `surviving_count -= 1`. The question resolves
//!   FAIL-side (`[0, 1]`) so the fail-conditional tokens become redeemable.
//! * **Survives (honest):** no slash; the question resolves PASS-side (`[1, 0]`).
//!
//! `slashed_amount` is kept consistent with Task 7's per-proposer accounting
//! (a proposer's `bond_pool` contribution always equals its `slashed_amount`):
//! we add only `bond - already_slashed` so a previously flip-slashed proposer is
//! topped up to the full bond, never double-counted.
//!
//! # Deferred (documented)
//! settle performs the program-signed `resolve_question` (making the conditional
//! tokens redeemable per outcome) and all state/accounting updates. The PHYSICAL
//! `redeem_tokens` CPI that moves the underlying KASS out of the conditional
//! vault (fail-side → bond pool, or pass-side → returned) is a documented
//! follow-up — exactly mirroring `finalize_facts`/`finalize_ai_claims`, which
//! account slashes in the `bond_pool` counter without moving tokens. The slash
//! DECISION, the AMM binding, the TWAP read, and `surviving_count` / `bond_pool`
//! / `slashed_amount` updates are all real here.
//!
//! # Accounts
//! 0. oracle              — writable; owned by this program; the question's
//!    resolver (signs `resolve_question` via the oracle PDA seeds)
//! 1. market              — writable; the [`Market`] PDA for this claim
//! 2. ai_claim            — read-only; `== market.ai_claim`
//! 3. proposer            — writable; `== market.proposer`
//! 4. question            — writable; `== market.question` (resolved here)
//! 5. pass_amm            — read-only; `== market.pass_amm`, owned by `AMM_ID`
//! 6. fail_amm            — read-only; `== market.fail_amm`, owned by `AMM_ID`
//! 7. conditional_vault program
//! 8. cv_event_authority  — read-only; conditional_vault `#[event_cpi]` authority
//!
//! # Instruction payload (after the 1-byte discriminant)
//! `oracle_nonce: u64 LE` (exactly 8 bytes) — the oracle PDA signer seed nonce,
//! verified by re-derivation (same scheme as `open_challenge`).

use pinocchio::{
    account_info::AccountInfo,
    instruction::{AccountMeta, Seed, Signer},
    program_error::ProgramError,
    pubkey::{find_program_address, Pubkey},
    ProgramResult,
};

use crate::{
    clock::{now, require_phase},
    cpi::metadao,
    error::KassandraError,
    processor::guards::{
        assert_key, assert_owned_by_program, load_ai_claim, load_oracle, load_proposer,
    },
    state::{Market, Oracle, Phase},
};

/// Exact payload length: `oracle_nonce[8]`.
const PAYLOAD_LEN: usize = 8;

/// Verify `amm` is owned by `AMM_ID`, carries the `Amm` Anchor discriminator,
/// and is bound to `(expected_base, expected_quote)`, then return its
/// slot-weighted TWAP (`aggregator / slots_passed`, or `0` if the market never
/// produced an observation). This is the hard binding the prompt requires: the
/// AMM must be THIS market's pass/fail conditional pool.
fn verify_and_read_twap(
    amm: &AccountInfo,
    expected_base: &Pubkey,
    expected_quote: &Pubkey,
) -> Result<u128, ProgramError> {
    assert_owned_by_program(amm, &metadao::AMM_ID)?;
    let data = amm.try_borrow_data()?;
    if data.len() < metadao::AMM_MIN_LEN {
        return Err(KassandraError::InvalidAccount.into());
    }
    // Defense-in-depth: the 8-byte Anchor account discriminator must be `Amm`'s
    // (on top of the owner + conditional mint-pair binding below).
    if data[..8] != metadao::AMM_ACCOUNT_DISCRIMINATOR {
        return Err(KassandraError::InvalidAccount.into());
    }
    let base_mint = metadao::read_pubkey(&data, metadao::AMM_BASE_MINT_OFFSET)?;
    let quote_mint = metadao::read_pubkey(&data, metadao::AMM_QUOTE_MINT_OFFSET)?;
    if &base_mint != expected_base || &quote_mint != expected_quote {
        return Err(KassandraError::InvalidAccount.into());
    }

    let created_at = metadao::read_u64(&data, metadao::AMM_CREATED_AT_SLOT_OFFSET)?;
    let last_updated = metadao::read_u64(&data, metadao::AMM_LAST_UPDATED_SLOT_OFFSET)?;
    let aggregator = metadao::read_u128(&data, metadao::AMM_AGGREGATOR_OFFSET)?;
    let start_delay = metadao::read_u64(&data, metadao::AMM_START_DELAY_SLOTS_OFFSET)?;

    // Mirror the v0.4.2 AMM `get_twap()`:
    //   aggregator / (last_updated - (created_at + start_delay_slots)).
    // No observations (or no elapsed slots past the start delay) => no price
    // signal => 0 (a market with no counter-trading => claim survives, §7).
    let start_slot = created_at.saturating_add(start_delay);
    let slots = last_updated.saturating_sub(start_slot);
    if slots == 0 || aggregator == 0 {
        return Ok(0);
    }
    Ok(aggregator / slots as u128)
}

pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], payload: &[u8]) -> ProgramResult {
    if payload.len() != PAYLOAD_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let oracle_nonce = u64::from_le_bytes(payload[0..8].try_into().unwrap());

    let [oracle_ai, market_ai, ai_claim_ai, proposer_ai, question_ai, pass_amm_ai, fail_amm_ai, cv_prog_ai, cv_event_auth_ai, ..] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    assert_key(cv_prog_ai, &metadao::CONDITIONAL_VAULT_ID)?;

    // --- oracle + phase gate -----------------------------------------------
    let mut oracle: Oracle = load_oracle(oracle_ai, program_id)?;
    require_phase(&oracle, Phase::Challenge)?;

    // Re-derive the oracle PDA from the supplied nonce; it is the question's
    // resolver, signed below with `[b"oracle", nonce_le, [bump]]`.
    let (derived_oracle, derived_bump) =
        find_program_address(&[b"oracle", &oracle_nonce.to_le_bytes()], program_id);
    if &derived_oracle != oracle_ai.key() || derived_bump != oracle.bump {
        return Err(KassandraError::InvalidAccount.into());
    }

    // --- market load + binding ---------------------------------------------
    assert_owned_by_program(market_ai, program_id)?;
    if market_ai.data_len() < Market::LEN {
        return Err(KassandraError::InvalidAccount.into());
    }
    let mut market: Market = {
        let data = market_ai.try_borrow_data()?;
        bytemuck::pod_read_unaligned::<Market>(&data[..Market::LEN])
    };
    if market.account_type != crate::state::AccountType::Market.as_u8() {
        return Err(KassandraError::InvalidAccount.into());
    }
    if market.oracle != *oracle_ai.key() {
        return Err(KassandraError::InvalidAccount.into());
    }
    if market.is_settled() {
        return Err(KassandraError::AlreadySettled.into());
    }

    // --- TWAP window gate ---------------------------------------------------
    let now = now()?;
    if now < market.twap_end {
        return Err(KassandraError::TwapWindowOpen.into());
    }

    // --- bind the recorded accounts -----------------------------------------
    assert_key(ai_claim_ai, &market.ai_claim)?;
    assert_key(proposer_ai, &market.proposer)?;
    assert_key(question_ai, &market.question)?;
    assert_key(pass_amm_ai, &market.pass_amm)?;
    assert_key(fail_amm_ai, &market.fail_amm)?;
    // A challenger must not be able to alias the two pools.
    if pass_amm_ai.key() == fail_amm_ai.key() {
        return Err(KassandraError::InvalidAccount.into());
    }

    // --- HARD AMM binding: each AMM ↔ this market's conditional mint pair ----
    let (pass_kass_mint, _) = metadao::conditional_token_mint_pda(&market.kass_vault, 0);
    let (fail_kass_mint, _) = metadao::conditional_token_mint_pda(&market.kass_vault, 1);
    let (pass_usdc_mint, _) = metadao::conditional_token_mint_pda(&market.usdc_vault, 0);
    let (fail_usdc_mint, _) = metadao::conditional_token_mint_pda(&market.usdc_vault, 1);

    let pass_twap = verify_and_read_twap(pass_amm_ai, &pass_kass_mint, &pass_usdc_mint)?;
    let fail_twap = verify_and_read_twap(fail_amm_ai, &fail_kass_mint, &fail_usdc_mint)?;

    // --- claim + proposer ---------------------------------------------------
    let ai_claim = load_ai_claim(ai_claim_ai, program_id)?;
    if ai_claim.oracle != *oracle_ai.key() || ai_claim.proposer != *proposer_ai.key() {
        return Err(KassandraError::InvalidAccount.into());
    }
    let mut proposer = load_proposer(proposer_ai, program_id)?;
    if proposer.oracle != *oracle_ai.key() {
        return Err(KassandraError::InvalidAccount.into());
    }

    // --- slash trigger (u128): fail_twap * DEN > pass_twap * (DEN + NUM) -----
    // GUARD: `pass_twap == 0` ALWAYS survives. A zero pass TWAP means the pass
    // pool has no observation — i.e. NO counter-trading on the pass side — which
    // design §7 defines as "claim survives". Without this guard a challenger
    // could crank ONLY the fail pool (leaving pass un-cranked at 0) and cheaply
    // flip `fail_twap*DEN > 0` true to disqualify an honest proposer. So a
    // disqualification requires a real, non-zero pass price to beat.
    // Margin params are snapshotted on the oracle (== MARKET_THRESHOLD_* by
    // default); stored as u64, widened back to u128 for the overflow-safe math.
    let market_threshold_num = oracle.market_threshold_num as u128;
    let market_threshold_den = oracle.market_threshold_den as u128;
    let disqualify = if pass_twap == 0 {
        false
    } else {
        let lhs = fail_twap
            .checked_mul(market_threshold_den)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        let rhs = pass_twap
            .checked_mul(market_threshold_den + market_threshold_num)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        lhs > rhs
    };

    // PASS-side [1,0] survives, FAIL-side [0,1] disqualifies.
    let numerators: [u32; 2] = if disqualify { [0, 1] } else { [1, 0] };

    if disqualify && !proposer.is_disqualified() {
        // Forfeit the full bond; top up any prior (flip) slash so the proposer's
        // bond_pool contribution equals its slashed_amount == bond (no double
        // counting, never exceeds the physically escrowed bond).
        let delta = proposer
            .bond
            .checked_sub(proposer.slashed_amount)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        proposer.disqualified = 1;
        proposer.slashed = 1;
        proposer.slashed_amount = proposer.bond;
        oracle.bond_pool = oracle
            .bond_pool
            .checked_add(delta)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        oracle.surviving_count = oracle
            .surviving_count
            .checked_sub(1)
            .ok_or(ProgramError::ArithmeticOverflow)?;
    }

    // --- program-signed resolve_question (oracle PDA is the resolver) -------
    let (cv_event_auth, _) = metadao::event_authority_pda(&metadao::CONDITIONAL_VAULT_ID);
    assert_key(cv_event_auth_ai, &cv_event_auth)?;

    let resolve_data = metadao::resolve_question_data_binary(numerators);
    let resolve_metas = [
        AccountMeta::writable(question_ai.key()),
        AccountMeta::readonly_signer(oracle_ai.key()),
        AccountMeta::readonly(cv_event_auth_ai.key()),
        AccountMeta::readonly(cv_prog_ai.key()),
    ];
    let resolve_infos = [question_ai, oracle_ai, cv_event_auth_ai, cv_prog_ai];
    let nonce_le = oracle_nonce.to_le_bytes();
    let bump_seed = [oracle.bump];
    let oracle_seeds = [
        Seed::from(b"oracle".as_ref()),
        Seed::from(nonce_le.as_ref()),
        Seed::from(&bump_seed),
    ];
    let oracle_signer = Signer::from(&oracle_seeds);
    metadao::invoke_conditional_vault_signed(
        &resolve_data,
        &resolve_metas,
        &resolve_infos,
        &[oracle_signer],
    )?;

    // --- persist (oracle, proposer, market) ---------------------------------
    market.settled = 1;
    // One fewer OPEN challenge market. Task 12 gates final plurality recompute
    // on this reaching 0, so every challenged proposer is settled first.
    oracle.open_challenge_count = oracle
        .open_challenge_count
        .checked_sub(1)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    {
        let mut data = market_ai.try_borrow_mut_data()?;
        data[..Market::LEN].copy_from_slice(bytemuck::bytes_of(&market));
    }
    {
        let mut data = proposer_ai.try_borrow_mut_data()?;
        data[..crate::state::Proposer::LEN].copy_from_slice(bytemuck::bytes_of(&proposer));
    }
    {
        let mut data = oracle_ai.try_borrow_mut_data()?;
        data[..Oracle::LEN].copy_from_slice(bytemuck::bytes_of(&oracle));
    }

    Ok(())
}

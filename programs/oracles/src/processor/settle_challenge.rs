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
//! (a proposer's `bond_pool` contribution always equals its `slashed_amount`).
//! With the C2 KASS-fee carve-out (below), that contribution is `bond −
//! kass_fee`: we add only `(bond − kass_fee) − already_slashed` so a previously
//! flip-slashed proposer is topped up to exactly `bond − kass_fee`, never
//! double-counted, and the identity `slashed_amount == bond_pool contribution`
//! still holds.
//!
//! # Physical settlement + directional fees (Task C2 — implemented here)
//! After `resolve_question`, settle PHYSICALLY redeems the bond's idle pass/fail
//! conditional KASS (`market.oracle_pass_kass` + `oracle_fail_kass`) back into
//! `oracle.stake_vault` via a program-signed `redeem_tokens` CPI (winning side
//! redeems 1:1, losing side → 0, so the FULL `bond` KASS lands in `stake_vault`;
//! the bond was split into BOTH legs at `open_challenge` and never traded, so the
//! redeem is clean — recon §3/§4). Then it routes the directional fees:
//! * **Survives (pass-win, challenge FAILED):** the bond is the proposer's (no
//!   slash). `usdc_fee = challenger_usdc × challenge_fail_usdc_fee_num/den` →
//!   PROPOSER's USDC account; `challenger_usdc − usdc_fee` → CHALLENGER's USDC
//!   account. (Escrow fully accounted: fee + return == escrow.)
//! * **Disqualified (fail-win, challenge SUCCEEDED):** `kass_fee = bond ×
//!   challenge_success_kass_fee_num/den` → CHALLENGER's KASS account (from
//!   `stake_vault`); `bond − kass_fee` is the proposer's `bond_pool` contribution
//!   (== `slashed_amount`). The FULL `challenger_usdc` escrow → CHALLENGER's USDC
//!   account. (No proposer USDC fee on a successful challenge.)
//!
//! All token moves are program-signed by the oracle PDA (the SPL authority of
//! `stake_vault`, the escrow vault, and the conditional-KASS destinations).
//!
//! # Conservation
//! * KASS: redeem lands `bond` in `stake_vault`; on disqualify `kass_fee` then
//!   leaves to the challenger, so `stake_vault + kass_vault_underlying + kass_fee
//!   == total_oracle_stake`; on survive nothing leaves, so `stake_vault +
//!   kass_vault_underlying == total_oracle_stake`.
//! * USDC: `challenger_usdc == challenger_return + proposer_fee` (survive) or
//!   `== challenger_return + 0` (disqualify), exactly.
//!
//! # Accounts
//! 0.  oracle              — writable; owned by this program; the question's
//!     resolver + SPL authority of stake_vault/escrow/conditional dests
//! 1.  market              — writable; the [`Market`] PDA for this claim
//! 2.  ai_claim            — read-only; `== market.ai_claim`
//! 3.  proposer            — writable; `== market.proposer`
//! 4.  question            — writable; `== market.question` (resolved here)
//! 5.  pass_amm            — read-only; `== market.pass_amm`, owned by `AMM_ID`
//! 6.  fail_amm            — read-only; `== market.fail_amm`, owned by `AMM_ID`
//! 7.  conditional_vault program
//! 8.  cv_event_authority  — read-only; conditional_vault `#[event_cpi]` authority
//! 9.  token program
//! 10. stake_vault         — writable; `== oracle.stake_vault` (redeem dest + KASS-fee source)
//! 11. kass_vault          — writable; `== market.kass_vault` (redeem vault)
//! 12. kass_vault_underlying — writable; `== kass_vault.underlying_token_account`
//! 13. pass_kass_mint      — writable; conditional-KASS mint idx 0 of kass_vault
//! 14. fail_kass_mint      — writable; conditional-KASS mint idx 1 of kass_vault
//! 15. oracle_pass_kass    — writable; `== market.oracle_pass_kass` (pass-KASS holder)
//! 16. oracle_fail_kass    — writable; `== market.oracle_fail_kass` (fail-KASS holder)
//! 17. challenger_usdc_vault — writable; `== market.challenger_usdc_vault` (USDC escrow)
//! 18. proposer_usdc       — writable; proposer's USDC account (mint==usdc, owner==proposer.authority)
//! 19. challenger_usdc_dest — writable; challenger's USDC account (mint==usdc, owner==market.challenger)
//! 20. challenger_kass     — writable; challenger's KASS account (mint==kass, owner==market.challenger)
//!
//! # Instruction payload (after the 1-byte discriminant)
//! `oracle_nonce: u64 LE` (exactly 8 bytes) — the oracle PDA signer seed nonce,
//! verified by re-derivation (same scheme as `open_challenge`).

use pinocchio::{
    account::AccountView as AccountInfo, address::Address as Pubkey, cpi::Signer,
    error::ProgramError, instruction::InstructionAccount, ProgramResult,
};
use pinocchio_token::instructions::Transfer;

use crate::{
    clock::{now, require_phase},
    cpi::metadao,
    error::KassandraError,
    processor::guards::{
        assert_key, assert_owned_by_program, assert_token_account, load_ai_claim, load_oracle,
        load_proposer, verify_oracle_pda,
    },
    state::{Market, Oracle, Phase},
};

/// Exact payload length: `oracle_nonce[8]`.
const PAYLOAD_LEN: usize = 8;

/// `value × num / den` in u128, checked back into `u64`. `den == 0` (a malformed
/// fee config) is rejected as [`KassandraError::InvalidConfig`]. Used for both
/// directional fees (KASS fee on a successful challenge, USDC fee on a failed
/// one).
fn fee_amount(value: u64, num: u64, den: u64) -> Result<u64, ProgramError> {
    if den == 0 {
        return Err(KassandraError::InvalidConfig.into());
    }
    let scaled = (value as u128)
        .checked_mul(num as u128)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    u64::try_from(scaled / den as u128).map_err(|_| ProgramError::ArithmeticOverflow)
}

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
    // Bind the AMM to this market's conditional (base, quote) pair (owner +
    // length + `Amm` discriminator + exact mint pair). Shared with
    // `open_challenge`, which now enforces the SAME binding at open so an
    // unbindable AMM can never be recorded (see `metadao::assert_amm_bound`).
    metadao::assert_amm_bound(amm, expected_base, expected_quote)?;
    let data = amm.try_borrow()?;
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

pub fn process(program_id: &Pubkey, accounts: &mut [AccountInfo], payload: &[u8]) -> ProgramResult {
    if payload.len() != PAYLOAD_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let oracle_nonce = u64::from_le_bytes(payload[0..8].try_into().unwrap());

    let [oracle_ai, market_ai, ai_claim_ai, proposer_ai, question_ai, pass_amm_ai, fail_amm_ai, cv_prog_ai, cv_event_auth_ai, token_prog_ai, stake_vault_ai, kass_vault_ai, kass_vault_underlying_ai, pass_kass_mint_ai, fail_kass_mint_ai, oracle_pass_kass_ai, oracle_fail_kass_ai, escrow_vault_ai, proposer_usdc_ai, challenger_usdc_dest_ai, challenger_kass_ai, ..] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    assert_key(cv_prog_ai, &metadao::CONDITIONAL_VAULT_ID)?;
    assert_key(token_prog_ai, &pinocchio_token::ID)?;

    // --- oracle + phase gate -----------------------------------------------
    let mut oracle: Oracle = load_oracle(oracle_ai, program_id)?;
    require_phase(&oracle, Phase::Challenge)?;

    // The oracle PDA is the question's resolver, signed below with
    // `[b"oracle", nonce_le, [bump]]`.
    verify_oracle_pda(program_id, oracle_ai, &oracle, oracle_nonce)?;

    // --- market load + binding ---------------------------------------------
    assert_owned_by_program(market_ai, program_id)?;
    if market_ai.data_len() < Market::LEN {
        return Err(KassandraError::InvalidAccount.into());
    }
    let mut market: Market = {
        let data = market_ai.try_borrow()?;
        bytemuck::pod_read_unaligned::<Market>(&data[..Market::LEN])
    };
    if market.account_type != crate::state::AccountType::Market.as_u8() {
        return Err(KassandraError::InvalidAccount.into());
    }
    if market.oracle != *oracle_ai.address() {
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
    if pass_amm_ai.address() == fail_amm_ai.address() {
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
    if ai_claim.oracle != *oracle_ai.address() || ai_claim.proposer != *proposer_ai.address() {
        return Err(KassandraError::InvalidAccount.into());
    }
    let mut proposer = load_proposer(proposer_ai, program_id)?;
    if proposer.oracle != *oracle_ai.address() {
        return Err(KassandraError::InvalidAccount.into());
    }

    // --- bind the physical-settlement accounts ------------------------------
    // The redeem + fee CPIs below need: the stake vault (redeem dest + KASS-fee
    // source), the KASS conditional vault + its underlying ATA, the conditional
    // KASS mints + the oracle-PDA-owned holders the bond was split into, the USDC
    // escrow, and the proposer/challenger payout accounts. Bind every one to the
    // recorded `Market`/`Oracle` so a settle cranker cannot substitute accounts.
    assert_key(stake_vault_ai, &oracle.stake_vault)?;
    assert_key(kass_vault_ai, &market.kass_vault)?;
    assert_key(pass_kass_mint_ai, &pass_kass_mint)?;
    assert_key(fail_kass_mint_ai, &fail_kass_mint)?;
    assert_key(oracle_pass_kass_ai, &market.oracle_pass_kass)?;
    assert_key(oracle_fail_kass_ai, &market.oracle_fail_kass)?;
    assert_key(escrow_vault_ai, &market.challenger_usdc_vault)?;
    // The redeem vault's underlying token account must be the one the vault
    // records (the same ATA the bond was split into at open_challenge).
    assert_owned_by_program(kass_vault_ai, &metadao::CONDITIONAL_VAULT_ID)?;
    {
        let data = kass_vault_ai.try_borrow()?;
        let v_underlying_acct =
            metadao::read_pubkey(&data, metadao::VAULT_UNDERLYING_ACCOUNT_OFFSET)?;
        if &v_underlying_acct != kass_vault_underlying_ai.address() {
            return Err(KassandraError::InvalidAccount.into());
        }
    }
    // Payout destinations: pin mint + owner so the directional fees / escrow
    // return cannot be siphoned. Proposer USDC ↔ proposer.authority; challenger
    // USDC + KASS ↔ market.challenger.
    assert_token_account(proposer_usdc_ai, &oracle.usdc_mint, &proposer.authority)?;
    assert_token_account(
        challenger_usdc_dest_ai,
        &oracle.usdc_mint,
        &market.challenger,
    )?;
    assert_token_account(challenger_kass_ai, &oracle.kass_mint, &market.challenger)?;

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

    // Directional KASS fee on a SUCCESSFUL challenge (disqualify): a carve-out of
    // the bond to the challenger. The proposer's `bond_pool` contribution becomes
    // `bond − kass_fee` (NOT the full bond), keeping the per-proposer identity
    // `slashed_amount == bond_pool contribution`.
    //
    // DEFENSIVE CAP (belt-and-suspenders): cap the fee to the proposer's REMAINING
    // un-slashed bond (`bond − slashed_amount`). A proposer flip-slashed earlier in
    // finalize_ai_claims already contributed `slashed_amount` to bond_pool; the
    // carve-out tops that up to `bond − kass_fee`, which must stay ≥ the prior
    // slash. The `set_config` joint bound (`flip_slash_frac + success_kass_fee_frac
    // ≤ 1`) guarantees that for valid configs (cap is then a no-op), but capping
    // here means even a hypothetically-bad config can never underflow the carve-out
    // / brick settlement, nor transfer more KASS than is left in stake_vault. The
    // capped value drives BOTH the accounting and the KASS transfer below.
    let remaining_bond = proposer.bond.saturating_sub(proposer.slashed_amount);
    let kass_fee = fee_amount(
        proposer.bond,
        oracle.challenge_success_kass_fee_num,
        oracle.challenge_success_kass_fee_den,
    )?
    .min(remaining_bond);

    if disqualify && !proposer.is_disqualified() {
        // Net slash = bond − kass_fee (≥ slashed_amount by the cap). Top up any
        // prior (flip) slash to exactly that net (never double-counting, never
        // exceeding the escrowed bond): the kass_fee leaves to the challenger
        // below, the rest is the bond_pool contribution.
        let net_slash = proposer
            .bond
            .checked_sub(kass_fee)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        let delta = net_slash.saturating_sub(proposer.slashed_amount);
        proposer.disqualified = 1;
        proposer.slashed = 1;
        proposer.slashed_amount = net_slash;
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
        InstructionAccount::writable(question_ai.address()),
        InstructionAccount::readonly_signer(oracle_ai.address()),
        InstructionAccount::readonly(cv_event_auth_ai.address()),
        InstructionAccount::readonly(cv_prog_ai.address()),
    ];
    let resolve_infos = [&*question_ai, &*oracle_ai, &*cv_event_auth_ai, &*cv_prog_ai];
    let nonce_le = oracle_nonce.to_le_bytes();
    let bump_seed = [oracle.bump];
    let oracle_seeds = Oracle::signer_seeds(&nonce_le, &bump_seed);
    metadao::invoke_conditional_vault_signed(
        &resolve_data,
        &resolve_metas,
        &resolve_infos,
        &[Signer::from(&oracle_seeds)],
    )?;

    // --- physical redeem: bond's conditional KASS → stake_vault -------------
    // redeem_tokens (InteractWithVault, same shape as the open_challenge split):
    //   0 question  1 kass_vault(w)  2 kass_vault_underlying(w)
    //   3 authority=oracle PDA(signer)  4 stake_vault(w, user_underlying)
    //   5 token_program  6 cv_event_auth  7 cv_program
    //   remaining: pass_mint(w) fail_mint(w) oracle_pass_kass(w) oracle_fail_kass(w)
    // The question is now resolved, so the winning side redeems 1:1 and the losing
    // side → 0 — the FULL `bond` KASS the proposer split lands in `stake_vault`.
    let redeem_data = metadao::redeem_tokens_data();
    let redeem_metas = [
        InstructionAccount::readonly(question_ai.address()),
        InstructionAccount::writable(kass_vault_ai.address()),
        InstructionAccount::writable(kass_vault_underlying_ai.address()),
        InstructionAccount::readonly_signer(oracle_ai.address()), // authority (oracle PDA)
        InstructionAccount::writable(stake_vault_ai.address()),   // user_underlying (dest)
        InstructionAccount::readonly(token_prog_ai.address()),
        InstructionAccount::readonly(cv_event_auth_ai.address()),
        InstructionAccount::readonly(cv_prog_ai.address()),
        InstructionAccount::writable(pass_kass_mint_ai.address()),
        InstructionAccount::writable(fail_kass_mint_ai.address()),
        InstructionAccount::writable(oracle_pass_kass_ai.address()),
        InstructionAccount::writable(oracle_fail_kass_ai.address()),
    ];
    let redeem_infos = [
        &*question_ai,
        &*kass_vault_ai,
        &*kass_vault_underlying_ai,
        &*oracle_ai,
        &*stake_vault_ai,
        &*token_prog_ai,
        &*cv_event_auth_ai,
        &*cv_prog_ai,
        &*pass_kass_mint_ai,
        &*fail_kass_mint_ai,
        &*oracle_pass_kass_ai,
        &*oracle_fail_kass_ai,
    ];
    metadao::invoke_conditional_vault_signed(
        &redeem_data,
        &redeem_metas,
        &redeem_infos,
        &[Signer::from(&oracle_seeds)],
    )?;

    // --- directional fee routing (oracle PDA signs every move) --------------
    let challenger_usdc = market.challenger_usdc;
    if disqualify {
        // Successful challenge: KASS fee carved out of the (now-redeemed) bond in
        // stake_vault → challenger; full USDC escrow returned to the challenger.
        if kass_fee > 0 {
            Transfer::new(stake_vault_ai, challenger_kass_ai, oracle_ai, kass_fee)
                .invoke_signed(&[Signer::from(&oracle_seeds)])?;
        }
        if challenger_usdc > 0 {
            Transfer::new(
                escrow_vault_ai,
                challenger_usdc_dest_ai,
                oracle_ai,
                challenger_usdc,
            )
            .invoke_signed(&[Signer::from(&oracle_seeds)])?;
        }
    } else {
        // Failed challenge: bond stays the proposer's (redeemed into stake_vault).
        // USDC fee → proposer; the remainder of the escrow → challenger. The split
        // is exact: usdc_fee + return == challenger_usdc.
        let usdc_fee = fee_amount(
            challenger_usdc,
            oracle.challenge_fail_usdc_fee_num,
            oracle.challenge_fail_usdc_fee_den,
        )?;
        let challenger_return = challenger_usdc
            .checked_sub(usdc_fee)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if usdc_fee > 0 {
            Transfer::new(escrow_vault_ai, proposer_usdc_ai, oracle_ai, usdc_fee)
                .invoke_signed(&[Signer::from(&oracle_seeds)])?;
        }
        if challenger_return > 0 {
            Transfer::new(
                escrow_vault_ai,
                challenger_usdc_dest_ai,
                oracle_ai,
                challenger_return,
            )
            .invoke_signed(&[Signer::from(&oracle_seeds)])?;
        }
    }

    // --- persist (oracle, proposer, market) ---------------------------------
    market.settled = 1;
    // One fewer OPEN challenge market. Task 12 gates final plurality recompute
    // on this reaching 0, so every challenged proposer is settled first.
    oracle.open_challenge_count = oracle
        .open_challenge_count
        .checked_sub(1)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    {
        let mut data = market_ai.try_borrow_mut()?;
        data[..Market::LEN].copy_from_slice(bytemuck::bytes_of(&market));
    }
    {
        let mut data = proposer_ai.try_borrow_mut()?;
        data[..crate::state::Proposer::LEN].copy_from_slice(bytemuck::bytes_of(&proposer));
    }
    {
        let mut data = oracle_ai.try_borrow_mut()?;
        data[..Oracle::LEN].copy_from_slice(bytemuck::bytes_of(&oracle));
    }

    Ok(())
}

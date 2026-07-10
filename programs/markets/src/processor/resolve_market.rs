//! `resolve_market`: bridge a terminal Kassandra oracle result into the market's
//! MetaDAO `resolve_question`, so users can redeem their winning conditional
//! tokens 1:1 (or pro-rata on a void).
//!
//! Permissionless, idempotent crank. The Market PDA is the MetaDAO Question's
//! oracle-authority (`Question.oracle @40 == market PDA`), so it is the resolver
//! and must SIGN the `resolve_question` CPI. It is NOT a separate account: the
//! writable `market` account (index 0) doubles as the CPI `readonly_signer`, with
//! the same `AccountView` supplied in the CPI infos and signed via the market
//! seeds `[b"market", oracle, [outcome_index], [bump]]` (the same doubling
//! `activate` already relies on for `split_tokens`/`add_liquidity`).
//!
//! # Numerator selection (binary sub-market; YES = oracle resolves to `outcome_index`)
//! * `phase == Resolved (7)`: read `Oracle.resolved_option`;
//!   `== market.outcome_index => [1,0]` (YES pays), else `=> [0,1]` (NO pays);
//!   a `resolved_option` out of `options_count` range → `InvalidAccount`.
//!   Sets `status = Resolved`.
//! * `phase == InvalidDeadend (8)`: `[1,1]` (void — every leg redeems for half).
//!   Sets `status = Void`.
//!
//! After the CPI succeeds it sets `market.settled = 1` and the terminal status,
//! writing the Market once.
//!
//! # Instruction payload (after the 1-byte discriminant)
//! empty.
//!
//! # Accounts
//! 0. market               — writable; must be `Active`, not yet settled; the resolver
//! 1. oracle               — read-only; `== market.oracle`, terminal Kassandra oracle
//! 2. question             — writable; `== market.question`, owned by conditional_vault
//! 3. cv_event_authority   — read-only; conditional_vault `#[event_cpi]` authority
//! 4. cv_program           — read-only; the conditional_vault program

use pinocchio::{
    account::AccountView, address::Address, cpi::Signer, error::ProgramError,
    instruction::InstructionAccount, ProgramResult,
};

use crate::{
    cpi::metadao,
    error::MarketError,
    processor::guards::{
        assert_key, assert_owned_by_program, load_kassandra_oracle, load_market,
        market_signer_seeds, write_market,
    },
    state::MarketStatus,
};

pub fn process(
    program_id: &Address,
    accounts: &mut [AccountView],
    payload: &[u8],
) -> ProgramResult {
    if !payload.is_empty() {
        return Err(ProgramError::InvalidInstructionData);
    }
    let [market_ai, oracle_ai, question_ai, cv_event_auth_ai, cv_prog_ai, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // --- market gates -------------------------------------------------------
    let market = load_market(market_ai, program_id)?;
    // Idempotency FIRST: a resolved market has `settled == 1` AND a terminal
    // (non-Active) status, so this must precede the `Active` check — otherwise a
    // second resolve would report `NotActive` instead of `AlreadySettled`. A
    // never-activated market has `settled == 0` and falls through to `NotActive`.
    if market.settled != 0 {
        return Err(MarketError::AlreadySettled.into());
    }
    if market.status != MarketStatus::Active.as_u8() {
        return Err(MarketError::NotActive.into());
    }

    // --- bind the MetaDAO Question ------------------------------------------
    assert_key(question_ai, &market.question)?;
    assert_owned_by_program(question_ai, &metadao::CONDITIONAL_VAULT_ID)?;

    // --- bind the conditional_vault program + event authority ---------------
    assert_key(cv_prog_ai, &metadao::CONDITIONAL_VAULT_ID)?;
    let (cv_event_auth, _) = metadao::event_authority_pda(&metadao::CONDITIONAL_VAULT_ID);
    assert_key(cv_event_auth_ai, &cv_event_auth)?;

    // --- oracle must be terminal --------------------------------------------
    assert_key(oracle_ai, &market.oracle)?;
    let oracle = load_kassandra_oracle(oracle_ai)?;
    let resolved = oracle.phase == crate::kass_oracle::PHASE_RESOLVED;
    let deadend = oracle.phase == crate::kass_oracle::PHASE_INVALID_DEADEND;
    if !resolved && !deadend {
        return Err(MarketError::OracleNotTerminal.into());
    }

    // --- numerator selection + terminal status ------------------------------
    // Binary sub-market: YES pays iff the oracle resolved to this outcome.
    let (numerators, status) = if resolved {
        // Defensive: the oracle guarantees a valid `resolved_option`, but reject an
        // out-of-range value rather than silently mis-resolving.
        if oracle.resolved_option >= oracle.options_count {
            return Err(MarketError::InvalidAccount.into());
        }
        let numerators = if oracle.resolved_option == market.outcome_index {
            [1u32, 0] // YES pays
        } else {
            [0u32, 1] // NO pays
        };
        (numerators, MarketStatus::Resolved)
    } else {
        // InvalidDeadend → void: every conditional token redeems for half.
        ([1u32, 1], MarketStatus::Void)
    };

    // --- program-signed resolve_question (market PDA is the resolver) -------
    // metas: [question(w), market_pda(readonly_signer), cv_event_auth(ro), cv_program(ro)].
    // The market account doubles as the resolver signer (same key as the writable
    // top-level account); we supply the same `market_ai` in the infos.
    let resolve_data = metadao::resolve_question_data_binary(numerators);
    let resolve_metas = [
        InstructionAccount::writable(question_ai.address()),
        InstructionAccount::readonly_signer(market_ai.address()),
        InstructionAccount::readonly(cv_event_auth_ai.address()),
        InstructionAccount::readonly(cv_prog_ai.address()),
    ];
    let resolve_infos = [&*question_ai, &*market_ai, &*cv_event_auth_ai, &*cv_prog_ai];
    market_signer_seeds!(market, oidx, mbump, market_seeds);
    metadao::invoke_conditional_vault_signed(
        &resolve_data,
        &resolve_metas,
        &resolve_infos,
        &[Signer::from(&market_seeds)],
    )?;

    // --- persist: settled + terminal status (write once) --------------------
    let mut m = market;
    m.settled = 1;
    m.status = status.as_u8();
    // Short-circuit the `collect_fee` crank when there is nothing to collect: no
    // fee configured, or no LP position. Otherwise leave `fee_collected == 0` so
    // the permissionless crank must run before `claim_lp` opens.
    if m.fee_bps == 0 || m.lp_total == 0 {
        m.fee_collected = 1;
    }
    write_market(market_ai, &m)?;

    Ok(())
}

//! `activate`: turn a fully-funded `Funding` market into a live MetaDAO
//! cYES/cNO AMM prediction market.
//!
//! # Decomposed market (mirrors the Kassandra `open_challenge` precedent)
//! The client composes the MetaDAO accounts in its OWN prior transactions (a
//! binary `Question` whose oracle-authority == this Market PDA, a KASS
//! `conditional_vault`, and the cYES/cNO `Amm`). This instruction does NOT create
//! them — it **verifies** they are bound to this market (re-derive PDAs +
//! owner-check + read field bindings), performs the **program-signed** split of
//! the escrowed KASS into cYES/cNO, seeds the pool 50/50 via a program-signed
//! `add_liquidity`, and **records** the bindings on the [`Market`] with
//! `status = Active`.
//!
//! # Program-signed CPIs
//! Both the `conditional_vault::split_tokens` and `amm::add_liquidity` CPIs use
//! the Market PDA as authority, signed with the market seeds
//! `[b"market", market.oracle, [market.outcome_index], [market.bump]]` (the same
//! seeds `refund` uses).
//! The split's source (`user_underlying`) is `market.escrow_vault` (Market-PDA
//! authority); its cYES/cNO destinations are two transient Market-PDA-owned token
//! accounts created here (`[b"cyes"|b"cno", market]`). `add_liquidity` deposits
//! the full split amount of each side (balanced 50/50) and mints LP into a
//! Market-PDA-owned `lp_vault` (`[b"lp_vault", market]`).
//!
//! # Balanced seed (v1)
//! The pool seeds equal cYES/cNO reserves
//! (`base_amount == quote_amount == total_contributed`), so no capital is
//! stranded.
//!
//! # Instruction payload (after the 1-byte discriminant)
//! empty.

use pinocchio::{
    account::AccountView,
    address::Address,
    cpi::{Seed, Signer},
    error::ProgramError,
    instruction::InstructionAccount,
    ProgramResult,
};
use pinocchio_system::instructions::{Allocate, Assign, Transfer};
use pinocchio_token::instructions::InitializeAccount3;

use crate::{
    cpi::metadao,
    cpi::spl::{
        SPL_TOKEN_ACCOUNT_LEN, SPL_TOKEN_AMOUNT_OFFSET, SPL_TOKEN_MINT_OFFSET,
        SPL_TOKEN_OWNER_OFFSET,
    },
    error::MarketError,
    processor::guards::{
        assert_key, assert_owned_by_program, assert_signer, load_kassandra_oracle, load_market,
        market_signer_seeds, rent_exempt_lamports, write_market,
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
    let [market_ai, oracle_ai, payer_ai, question_ai, vault_ai, vault_underlying_ai, escrow_ai, yes_mint_ai, no_mint_ai, market_cyes_ai, market_cno_ai, amm_ai, lp_mint_ai, lp_vault_ai, amm_vault_base_ai, amm_vault_quote_ai, cv_event_auth_ai, cv_prog_ai, amm_event_auth_ai, amm_prog_ai, token_prog_ai, system_prog_ai, ..] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // --- signer + program ids ----------------------------------------------
    assert_signer(payer_ai)?;
    assert_key(cv_prog_ai, &metadao::CONDITIONAL_VAULT_ID)?;
    assert_key(amm_prog_ai, &metadao::AMM_ID)?;
    assert_key(token_prog_ai, &pinocchio_token::ID)?;
    assert_key(system_prog_ai, &pinocchio_system::ID)?;

    // --- market state gates -------------------------------------------------
    let market = load_market(market_ai, program_id)?;
    if market.status != MarketStatus::Funding.as_u8() {
        return Err(MarketError::NotFunding.into());
    }
    if market.total_contributed < market.min_liquidity {
        return Err(MarketError::NotFunded.into());
    }
    assert_key(oracle_ai, &market.oracle)?;
    assert_key(escrow_ai, &market.escrow_vault)?;

    // --- oracle must be NON-terminal (a resolved oracle can't be activated) --
    // Terminal == the same check `cancel` uses; a terminal oracle must take the
    // cancel/refund exit, not activate.
    let oracle = load_kassandra_oracle(oracle_ai)?;
    let terminal = oracle.phase == crate::kass_oracle::PHASE_RESOLVED
        || oracle.phase == crate::kass_oracle::PHASE_INVALID_DEADEND;
    if terminal {
        return Err(MarketError::OracleResolved.into());
    }

    // --- verify the composed MetaDAO market (re-derive + owner + bindings) ---
    // question_id == the kassandra oracle address; oracle_authority == market PDA.
    assert_owned_by_program(question_ai, &metadao::CONDITIONAL_VAULT_ID)?;
    let (expect_question, _) =
        metadao::question_pda(market.oracle.as_array(), market_ai.address(), 2);
    assert_key(question_ai, &expect_question)?;
    {
        let data = question_ai.try_borrow()?;
        let q_oracle = metadao::read_pubkey(&data, metadao::QUESTION_ORACLE_OFFSET)?;
        let num_outcomes = metadao::read_u32(&data, metadao::QUESTION_NUM_OUTCOMES_LEN_OFFSET)?;
        if &q_oracle != market_ai.address() || num_outcomes != 2 {
            return Err(MarketError::InvalidAccount.into());
        }
    }

    // KASS conditional vault: question@8, underlying_mint@40, underlying_account@72.
    assert_owned_by_program(vault_ai, &metadao::CONDITIONAL_VAULT_ID)?;
    let (expect_vault, _) = metadao::vault_pda(question_ai.address(), &market.kass_mint);
    assert_key(vault_ai, &expect_vault)?;
    {
        let data = vault_ai.try_borrow()?;
        let v_question = metadao::read_pubkey(&data, metadao::VAULT_QUESTION_OFFSET)?;
        let v_underlying = metadao::read_pubkey(&data, metadao::VAULT_UNDERLYING_MINT_OFFSET)?;
        let v_underlying_acct =
            metadao::read_pubkey(&data, metadao::VAULT_UNDERLYING_ACCOUNT_OFFSET)?;
        if &v_question != question_ai.address()
            || v_underlying != market.kass_mint
            || &v_underlying_acct != vault_underlying_ai.address()
        {
            return Err(MarketError::InvalidAccount.into());
        }
    }

    // Conditional-KASS mints derive from the vault (idx 0 = cYES, idx 1 = cNO).
    let (expect_yes, _) = metadao::conditional_token_mint_pda(vault_ai.address(), 0);
    let (expect_no, _) = metadao::conditional_token_mint_pda(vault_ai.address(), 1);
    assert_key(yes_mint_ai, &expect_yes)?;
    assert_key(no_mint_ai, &expect_no)?;

    // AMM: owned by amm program, re-derived, disc + base/quote mint bound.
    assert_owned_by_program(amm_ai, &metadao::AMM_ID)?;
    let (expect_amm, _) = metadao::amm_pda(yes_mint_ai.address(), no_mint_ai.address());
    assert_key(amm_ai, &expect_amm)?;
    {
        let data = amm_ai.try_borrow()?;
        if data.len() < 8 || data[..8] != metadao::AMM_ACCOUNT_DISCRIMINATOR {
            return Err(MarketError::InvalidAccount.into());
        }
        let base = metadao::read_pubkey(&data, metadao::AMM_BASE_MINT_OFFSET)?;
        let quote = metadao::read_pubkey(&data, metadao::AMM_QUOTE_MINT_OFFSET)?;
        if &base != yes_mint_ai.address() || &quote != no_mint_ai.address() {
            return Err(MarketError::InvalidAccount.into());
        }
        // The pool MUST be empty: we seed it with `add_liquidity(min_lp_tokens=0)`
        // and want the clean 50/50 opening ratio. A front-runner adding liquidity
        // between `create_amm` and `activate` would otherwise set the ratio and
        // strand our split cYES/cNO. RESIDUAL (v1): front-running this is possible
        // but costs the front-runner real split capital, so a safe revert here is
        // acceptable (revisit alongside the uneven-prior refinement).
        let base_amount = metadao::read_u64(&data, metadao::AMM_BASE_AMOUNT_OFFSET)?;
        let quote_amount = metadao::read_u64(&data, metadao::AMM_QUOTE_AMOUNT_OFFSET)?;
        if base_amount != 0 || quote_amount != 0 {
            return Err(MarketError::PoolNotEmpty.into());
        }
    }

    // LP mint derives from the amm.
    let (expect_lp_mint, _) = metadao::amm_lp_mint_pda(amm_ai.address());
    assert_key(lp_mint_ai, &expect_lp_mint)?;

    // The AMM's per-mint vault ATAs (symmetry with the rest of the verification;
    // the AMM callee also enforces these).
    let (expect_vault_base, _) =
        metadao::associated_token_address(amm_ai.address(), yes_mint_ai.address());
    let (expect_vault_quote, _) =
        metadao::associated_token_address(amm_ai.address(), no_mint_ai.address());
    assert_key(amm_vault_base_ai, &expect_vault_base)?;
    assert_key(amm_vault_quote_ai, &expect_vault_quote)?;

    // Event authorities (single-seed PDAs on each MetaDAO program).
    let (cv_event_auth, _) = metadao::event_authority_pda(&metadao::CONDITIONAL_VAULT_ID);
    assert_key(cv_event_auth_ai, &cv_event_auth)?;
    let (amm_event_auth, _) = metadao::event_authority_pda(&metadao::AMM_ID);
    assert_key(amm_event_auth_ai, &amm_event_auth)?;

    // --- create the three Market-PDA-owned token accounts -------------------
    let rent = rent_exempt_lamports(SPL_TOKEN_ACCOUNT_LEN)?;
    create_market_token_account(
        payer_ai,
        market_cyes_ai,
        yes_mint_ai,
        market_ai,
        b"cyes",
        program_id,
        rent,
    )?;
    create_market_token_account(
        payer_ai,
        market_cno_ai,
        no_mint_ai,
        market_ai,
        b"cno",
        program_id,
        rent,
    )?;
    create_market_token_account(
        payer_ai,
        lp_vault_ai,
        lp_mint_ai,
        market_ai,
        b"lp_vault",
        program_id,
        rent,
    )?;

    // --- market-PDA signer seeds (shared by both CPIs) ----------------------
    market_signer_seeds!(market, oidx, mbump, market_seeds);
    let amount = market.total_contributed;

    // --- program-signed split: escrow KASS -> cYES/cNO ----------------------
    let split_data = metadao::split_tokens_data(amount);
    let split_metas = [
        InstructionAccount::readonly(question_ai.address()),
        InstructionAccount::writable(vault_ai.address()),
        InstructionAccount::writable(vault_underlying_ai.address()),
        InstructionAccount::readonly_signer(market_ai.address()), // authority (market PDA)
        InstructionAccount::writable(escrow_ai.address()),        // user_underlying (split source)
        InstructionAccount::readonly(token_prog_ai.address()),
        InstructionAccount::readonly(cv_event_auth_ai.address()),
        InstructionAccount::readonly(cv_prog_ai.address()),
        InstructionAccount::writable(yes_mint_ai.address()),
        InstructionAccount::writable(no_mint_ai.address()),
        InstructionAccount::writable(market_cyes_ai.address()),
        InstructionAccount::writable(market_cno_ai.address()),
    ];
    let split_infos = [
        &*question_ai,
        &*vault_ai,
        &*vault_underlying_ai,
        &*market_ai,
        &*escrow_ai,
        &*token_prog_ai,
        &*cv_event_auth_ai,
        &*cv_prog_ai,
        &*yes_mint_ai,
        &*no_mint_ai,
        &*market_cyes_ai,
        &*market_cno_ai,
    ];
    metadao::invoke_conditional_vault_signed(
        &split_data,
        &split_metas,
        &split_infos,
        &[Signer::from(&market_seeds)],
    )?;

    // --- program-signed add_liquidity: seed the pool 50/50 ------------------
    // quote_amount == max_base_amount == the full split amount (empty pool → both
    // sides deposited in full, no ratio constraint); min_lp_tokens == 0.
    let add_data = metadao::add_liquidity_data(amount, amount, 0);
    let add_metas = [
        InstructionAccount::writable_signer(market_ai.address()), // authority (market PDA)
        InstructionAccount::writable(amm_ai.address()),
        InstructionAccount::writable(lp_mint_ai.address()),
        InstructionAccount::writable(lp_vault_ai.address()), // user_lp
        InstructionAccount::writable(market_cyes_ai.address()), // user_base
        InstructionAccount::writable(market_cno_ai.address()), // user_quote
        InstructionAccount::writable(amm_vault_base_ai.address()),
        InstructionAccount::writable(amm_vault_quote_ai.address()),
        InstructionAccount::readonly(token_prog_ai.address()),
        InstructionAccount::readonly(amm_event_auth_ai.address()),
        InstructionAccount::readonly(amm_prog_ai.address()),
    ];
    let add_infos = [
        &*market_ai,
        &*amm_ai,
        &*lp_mint_ai,
        &*lp_vault_ai,
        &*market_cyes_ai,
        &*market_cno_ai,
        &*amm_vault_base_ai,
        &*amm_vault_quote_ai,
        &*token_prog_ai,
        &*amm_event_auth_ai,
        &*amm_prog_ai,
    ];
    metadao::invoke_amm_signed(
        &add_data,
        &add_metas,
        &add_infos,
        &[Signer::from(&market_seeds)],
    )?;

    // --- measure the LP tokens now in lp_vault ------------------------------
    let lp_total = {
        let data = lp_vault_ai.try_borrow()?;
        metadao::read_u64(&data, SPL_TOKEN_AMOUNT_OFFSET)?
    };

    // --- record bindings + flip to Active -----------------------------------
    let mut m = market; // preserve the Phase-1 fields
    m.question = *question_ai.address();
    m.vault = *vault_ai.address();
    m.yes_mint = *yes_mint_ai.address();
    m.no_mint = *no_mint_ai.address();
    m.amm = *amm_ai.address();
    m.lp_mint = *lp_mint_ai.address();
    m.lp_vault = *lp_vault_ai.address();
    m.lp_total = lp_total;
    m.status = MarketStatus::Active.as_u8();
    write_market(market_ai, &m)?;

    Ok(())
}

/// Create-or-adopt a Market-PDA-owned SPL token account at PDA `[seed, market]`
/// on `mint`, paid by `payer`, token authority == the Market PDA.
///
/// # DoS resistance (create-or-adopt, mirroring `init_config`)
/// The three token-account PDAs are deterministic, so a plain system
/// `CreateAccount` would let anyone brick `activate` permanently by pre-funding
/// the address with 1 lamport (`CreateAccount` fails `AccountAlreadyInUse` on any
/// funded account). Instead we ADOPT: if the account isn't already the live
/// token account we want, top the balance up to rent-exempt (only if short),
/// `Allocate` the space + `Assign` it to the SPL token program (both signed by
/// the PDA seeds), then `InitializeAccount3`. A system-owned attacker-pre-funded
/// account carries no data and can only be `Allocate`d by the PDA signer, so
/// adoption always succeeds.
fn create_market_token_account(
    payer_ai: &AccountView,
    account_ai: &AccountView,
    mint_ai: &AccountView,
    market_ai: &AccountView,
    seed: &[u8],
    program_id: &Address,
    rent: u64,
) -> ProgramResult {
    let (expected, bump) =
        Address::find_program_address(&[seed, market_ai.address().as_ref()], program_id);
    assert_key(account_ai, &expected)?;

    // Re-entry / already-live guard: if this is ALREADY our initialized token
    // account (token-program owned, correct mint + authority), there is nothing
    // to do. This tolerates a partially-completed prior attempt and prevents an
    // `InitializeAccount3`-on-initialized failure.
    if account_ai.owned_by(&pinocchio_token::ID) {
        let data = account_ai.try_borrow()?;
        if data.len() >= SPL_TOKEN_ACCOUNT_LEN {
            let mint = metadao::read_pubkey(&data, SPL_TOKEN_MINT_OFFSET)?;
            let owner = metadao::read_pubkey(&data, SPL_TOKEN_OWNER_OFFSET)?;
            if &mint == mint_ai.address() && &owner == market_ai.address() {
                return Ok(());
            }
            // Owned by the token program but not our account → reject rather than
            // trample someone else's token account at this address.
            return Err(MarketError::InvalidAccount.into());
        }
    }

    let bump_seed = [bump];
    let seeds = [
        Seed::from(seed),
        Seed::from(market_ai.address().as_ref()),
        Seed::from(&bump_seed),
    ];

    // Top up to rent-exempt only if the (possibly pre-funded) account is short.
    let current = account_ai.lamports();
    if current < rent {
        Transfer {
            from: payer_ai,
            to: account_ai,
            lamports: rent - current,
        }
        .invoke()?;
    }
    Allocate {
        account: account_ai,
        space: SPL_TOKEN_ACCOUNT_LEN as u64,
    }
    .invoke_signed(&[Signer::from(&seeds)])?;
    Assign {
        account: account_ai,
        owner: &pinocchio_token::ID,
    }
    .invoke_signed(&[Signer::from(&seeds)])?;
    InitializeAccount3 {
        account: account_ai,
        mint: mint_ai,
        owner: market_ai.address(),
    }
    .invoke()?;
    Ok(())
}

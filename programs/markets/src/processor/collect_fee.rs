//! `collect_fee` (Ix 9): permissionless crank that cuts the protocol's `fee_bps`
//! share of a resolved market's **accrued** LP earnings and routes it, denominated
//! in KASS, to the futarchy-governed `Config.fee_destination`.
//!
//! # Why a separate crank (not part of `resolve_market`)
//! Keeps resolve lean and isolates the heavy program-signed CPIs
//! (`amm::remove_liquidity` → `conditional_vault::redeem_tokens` → SPL `transfer`)
//! plus the accrued math and the big account list. `claim_lp` gates on
//! `market.fee_collected == 1`, so ordering is forced: **resolve → collect_fee →
//! claim_lp**, and `lp_total` is final (reduced by the fee cut) before any
//! pro-rata LP is distributed. Idempotent: a second call sees `fee_collected == 1`
//! and rejects.
//!
//! # Accrued math (u128, floor, conservative — under-charges on rounding)
//! Reading only the resolved `Question` numerators, the `Amm` reserves, and the
//! AMM LP-mint supply (no full-pool unwind):
//! 1. `(num0, num1, denom)` = the resolved payout numerators / denominator.
//! 2. `(base, quote)` = the pool's cYES / cNO reserves.
//! 3. `supply` = the AMM LP-mint total supply.
//! 4. `pool_value = (base·num0 + quote·num1) / denom`  (full-pool KASS value).
//! 5. `realized_full = lp_total · pool_value / supply`  (this market's LP value).
//! 6. `accrued = realized_full.saturating_sub(total_contributed)`  (0 ⇒ no fee;
//!    impermanent-loss / no-profit case → just set the flag and return).
//! 7. `accrued_lp = lp_total · accrued / realized_full`  (LP tokens ≙ the accrued).
//! 8. `fee_lp = accrued_lp · fee_bps / 10000`  (LP tokens to realize as the fee).
//!
//! # Realize the fee slice (all program-signed with the market seeds)
//! 9. `remove_liquidity(fee_lp, 0, 0)` burns `fee_lp` LP out of `lp_vault`,
//!    returning pro-rata cYES/cNO into the market-PDA-owned `market_cyes`/`_cno`.
//! 10. `redeem_tokens()` against the resolved Question burns those cYES/cNO and
//!     pays the resolved KASS into `escrow_vault` (empty since `activate`).
//! 11. SPL `transfer` the redeemed KASS `escrow_vault → fee_destination`.
//! 12. `lp_total -= fee_lp`; `fee_collected = 1`. Market written once.
//!
//! # Instruction payload (after the 1-byte discriminant)
//! empty.
//!
//! # Accounts
//!  0. market                (w)  — Resolved/Void, `fee_collected == 0`; the CPI signer
//!  1. config                (ro) — the Config PDA (source of `fee_destination` + `kass_mint`)
//!  2. fee_destination       (w)  — `config.fee_destination`; KASS token account
//!  3. question              (ro) — `market.question`; resolved binary Question
//!  4. vault                 (w)  — `market.vault`; KASS conditional vault
//!  5. vault_underlying_ata  (w)  — the vault's KASS ATA
//!  6. escrow_vault          (w)  — `market.escrow_vault`; redeem dest + transfer source
//!  7. yes_mint              (w)  — `market.yes_mint` (cYES)
//!  8. no_mint               (w)  — `market.no_mint` (cNO)
//!  9. market_cyes           (w)  — `[b"cyes", market]`
//! 10. market_cno            (w)  — `[b"cno", market]`
//! 11. amm                   (w)  — `market.amm`
//! 12. lp_mint               (w)  — `market.lp_mint`
//! 13. lp_vault              (w)  — `market.lp_vault`
//! 14. amm_vault_base        (w)  — amm's cYES ATA
//! 15. amm_vault_quote       (w)  — amm's cNO ATA
//! 16. cv_event_authority    (ro)
//! 17. cv_program            (ro)
//! 18. amm_event_authority   (ro)
//! 19. amm_program           (ro)
//! 20. token program         (ro)

use pinocchio::{
    account::AccountView, address::Address, cpi::Signer, error::ProgramError,
    instruction::InstructionAccount, ProgramResult,
};
use pinocchio_token::instructions::Transfer;

use crate::{
    cpi::metadao,
    cpi::spl::SPL_TOKEN_AMOUNT_OFFSET,
    error::MarketError,
    processor::guards::{
        assert_key, assert_owned_by_program, load_config, load_market, market_signer_seeds,
        read_token_mint, write_market,
    },
    state::{Market, MarketStatus},
};

pub fn process(
    program_id: &Address,
    accounts: &mut [AccountView],
    payload: &[u8],
) -> ProgramResult {
    if !payload.is_empty() {
        return Err(ProgramError::InvalidInstructionData);
    }
    let [market_ai, config_ai, fee_dest_ai, question_ai, vault_ai, vault_underlying_ai, escrow_ai, yes_mint_ai, no_mint_ai, market_cyes_ai, market_cno_ai, amm_ai, lp_mint_ai, lp_vault_ai, amm_vault_base_ai, amm_vault_quote_ai, cv_event_auth_ai, cv_prog_ai, amm_event_auth_ai, amm_prog_ai, token_prog_ai, ..] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // --- program ids --------------------------------------------------------
    assert_key(cv_prog_ai, &metadao::CONDITIONAL_VAULT_ID)?;
    assert_key(amm_prog_ai, &metadao::AMM_ID)?;
    assert_key(token_prog_ai, &pinocchio_token::ID)?;

    // --- market state gates -------------------------------------------------
    let market = load_market(market_ai, program_id)?;
    // Idempotency / "nothing to collect" FIRST: `resolve_market` already stamps
    // `fee_collected = 1` for fee-free / no-LP markets, and a prior successful
    // crank stamps it too. Either way there is nothing left to do.
    if market.fee_collected != 0 {
        return Err(MarketError::AlreadySettled.into());
    }
    // Must be terminal (settled). A still-`Active` (or Funding/Cancelled) market's
    // Question is not resolved, so there is nothing to realize. `NotActive` is the
    // closest existing reject (the market is not in a collectable terminal state).
    let terminal = market.status == MarketStatus::Resolved.as_u8()
        || market.status == MarketStatus::Void.as_u8();
    if !terminal {
        return Err(MarketError::NotActive.into());
    }
    // Defensive: `resolve_market` guarantees `fee_collected == 1` whenever
    // `fee_bps == 0 || lp_total == 0`, so reaching here with `fee_collected == 0`
    // implies both are non-zero. Re-check and short-circuit rather than trust it.
    if market.fee_bps == 0 || market.lp_total == 0 {
        return write_collected(market_ai, market);
    }

    // --- config + fee destination -------------------------------------------
    let (config_pda, _) = Address::find_program_address(&[b"config"], program_id);
    assert_key(config_ai, &config_pda)?;
    let config = load_config(config_ai, program_id)?;
    assert_key(fee_dest_ai, &config.fee_destination)?;
    // The destination must be a live SPL token account on the KASS mint.
    if read_token_mint(fee_dest_ai)? != config.kass_mint {
        return Err(MarketError::WrongMint.into());
    }

    // --- verify the recorded MetaDAO bindings (mirror `activate`) ------------
    assert_key(question_ai, &market.question)?;
    assert_owned_by_program(question_ai, &metadao::CONDITIONAL_VAULT_ID)?;
    assert_key(vault_ai, &market.vault)?;
    assert_owned_by_program(vault_ai, &metadao::CONDITIONAL_VAULT_ID)?;
    assert_key(amm_ai, &market.amm)?;
    assert_owned_by_program(amm_ai, &metadao::AMM_ID)?;
    assert_key(lp_mint_ai, &market.lp_mint)?;
    assert_key(lp_vault_ai, &market.lp_vault)?;
    assert_key(yes_mint_ai, &market.yes_mint)?;
    assert_key(no_mint_ai, &market.no_mint)?;
    assert_key(escrow_ai, &market.escrow_vault)?;

    // The two transient cYES/cNO holders (`[b"cyes"|b"cno", market]`, created at
    // `activate`; empty since add_liquidity consumed the split).
    let (expect_cyes, _) =
        Address::find_program_address(&[b"cyes", market_ai.address().as_ref()], program_id);
    let (expect_cno, _) =
        Address::find_program_address(&[b"cno", market_ai.address().as_ref()], program_id);
    assert_key(market_cyes_ai, &expect_cyes)?;
    assert_key(market_cno_ai, &expect_cno)?;

    // The vault's underlying (KASS) ATA + mint binding.
    {
        let d = vault_ai.try_borrow()?;
        let v_underlying = metadao::read_pubkey(&d, metadao::VAULT_UNDERLYING_MINT_OFFSET)?;
        let v_underlying_acct = metadao::read_pubkey(&d, metadao::VAULT_UNDERLYING_ACCOUNT_OFFSET)?;
        if v_underlying != market.kass_mint || &v_underlying_acct != vault_underlying_ai.address() {
            return Err(MarketError::InvalidAccount.into());
        }
    }

    // AMM per-mint vault ATAs + event authorities.
    let (expect_vault_base, _) =
        metadao::associated_token_address(amm_ai.address(), yes_mint_ai.address());
    let (expect_vault_quote, _) =
        metadao::associated_token_address(amm_ai.address(), no_mint_ai.address());
    assert_key(amm_vault_base_ai, &expect_vault_base)?;
    assert_key(amm_vault_quote_ai, &expect_vault_quote)?;
    let (cv_event_auth, _) = metadao::event_authority_pda(&metadao::CONDITIONAL_VAULT_ID);
    assert_key(cv_event_auth_ai, &cv_event_auth)?;
    let (amm_event_auth, _) = metadao::event_authority_pda(&metadao::AMM_ID);
    assert_key(amm_event_auth_ai, &amm_event_auth)?;

    // --- accrued math (u128, floor, saturating) -----------------------------
    let (num0, num1, denom) = {
        let d = question_ai.try_borrow()?;
        (
            metadao::read_u32(&d, metadao::QUESTION_NUM0_OFFSET)? as u128,
            metadao::read_u32(&d, metadao::QUESTION_NUM1_OFFSET)? as u128,
            metadao::read_u32(&d, metadao::QUESTION_DENOMINATOR_OFFSET)? as u128,
        )
    };
    if denom == 0 {
        // Question not resolved (shouldn't happen for a terminal market) — refuse
        // rather than divide by zero.
        return Err(MarketError::InvalidAccount.into());
    }
    let (base, quote) = {
        let d = amm_ai.try_borrow()?;
        (
            metadao::read_u64(&d, metadao::AMM_BASE_AMOUNT_OFFSET)? as u128,
            metadao::read_u64(&d, metadao::AMM_QUOTE_AMOUNT_OFFSET)? as u128,
        )
    };
    let supply = {
        let d = lp_mint_ai.try_borrow()?;
        metadao::read_u64(&d, metadao::MINT_SUPPLY_OFFSET)? as u128
    };
    if supply == 0 {
        return Err(MarketError::InvalidAccount.into());
    }

    let lp_total = market.lp_total as u128;
    let total_contributed = market.total_contributed as u128;
    let fee_bps = market.fee_bps as u128;

    // Full-pool KASS value at resolution, then this market's LP share of it.
    let pool_value = base
        .checked_mul(num0)
        .and_then(|x| x.checked_add(quote.checked_mul(num1)?))
        .ok_or(ProgramError::ArithmeticOverflow)?
        / denom;
    // `lp_total · pool_value` is the only multiplication that can grow large: the
    // intermediate is on the order of `total_contributed²` (both factors are bounded
    // by the KASS the market split in), so it only nears `u128::MAX` (~3.4e38) when
    // `total_contributed` approaches ~u64::MAX (~1.8e19 base units ≈ 18B KASS at
    // 9 dp) — astronomically beyond any real market. Should it ever overflow,
    // collect_fee reverts; because claim_lp gates on `fee_collected`, that would
    // brick LP withdrawal — an acknowledged, unreachable bound (no mul_div needed).
    let realized_full = lp_total
        .checked_mul(pool_value)
        .ok_or(ProgramError::ArithmeticOverflow)?
        / supply;

    // Accrued (profit over contributed). Zero on impermanent loss / no gain.
    let accrued = realized_full.saturating_sub(total_contributed);
    if accrued == 0 {
        return write_collected(market_ai, market);
    }
    // `realized_full >= accrued > 0`, so the division is safe.
    let accrued_lp = lp_total
        .checked_mul(accrued)
        .ok_or(ProgramError::ArithmeticOverflow)?
        / realized_full;
    let fee_lp_u128 = accrued_lp
        .checked_mul(fee_bps)
        .ok_or(ProgramError::ArithmeticOverflow)?
        / 10_000;
    let fee_lp = u64::try_from(fee_lp_u128).map_err(|_| ProgramError::ArithmeticOverflow)?;
    if fee_lp == 0 {
        return write_collected(market_ai, market);
    }

    // --- market-PDA signer seeds (shared by all three CPIs) -----------------
    market_signer_seeds!(market, oidx, mbump, market_seeds);

    // --- (9) program-signed remove_liquidity: fee_lp LP → cYES/cNO ----------
    // Mirrors `activate`'s add_liquidity account order (the AMM uses the same
    // 11-account add/remove context); min_base == min_quote == 0.
    let remove_data = metadao::remove_liquidity_data(fee_lp, 0, 0);
    let remove_metas = [
        InstructionAccount::writable_signer(market_ai.address()), // authority (market PDA)
        InstructionAccount::writable(amm_ai.address()),
        InstructionAccount::writable(lp_mint_ai.address()),
        InstructionAccount::writable(lp_vault_ai.address()), // user_lp (LP burned from here)
        InstructionAccount::writable(market_cyes_ai.address()), // user_base
        InstructionAccount::writable(market_cno_ai.address()), // user_quote
        InstructionAccount::writable(amm_vault_base_ai.address()),
        InstructionAccount::writable(amm_vault_quote_ai.address()),
        InstructionAccount::readonly(token_prog_ai.address()),
        InstructionAccount::readonly(amm_event_auth_ai.address()),
        InstructionAccount::readonly(amm_prog_ai.address()),
    ];
    let remove_infos = [
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
        &remove_data,
        &remove_metas,
        &remove_infos,
        &[Signer::from(&market_seeds)],
    )?;

    // --- (10) program-signed redeem_tokens: cYES/cNO → KASS into escrow -----
    // Mirrors `activate`'s split_tokens InteractWithVault order (redeem shares it);
    // authority = market PDA, user_underlying = escrow_vault (empty pre-redeem).
    let redeem_data = metadao::redeem_tokens_data();
    let redeem_metas = [
        InstructionAccount::readonly(question_ai.address()),
        InstructionAccount::writable(vault_ai.address()),
        InstructionAccount::writable(vault_underlying_ai.address()),
        InstructionAccount::readonly_signer(market_ai.address()), // authority (market PDA)
        InstructionAccount::writable(escrow_ai.address()),        // user_underlying (redeem dest)
        InstructionAccount::readonly(token_prog_ai.address()),
        InstructionAccount::readonly(cv_event_auth_ai.address()),
        InstructionAccount::readonly(cv_prog_ai.address()),
        InstructionAccount::writable(yes_mint_ai.address()),
        InstructionAccount::writable(no_mint_ai.address()),
        InstructionAccount::writable(market_cyes_ai.address()), // user_yes (burned)
        InstructionAccount::writable(market_cno_ai.address()),  // user_no (burned)
    ];
    let redeem_infos = [
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
        &redeem_data,
        &redeem_metas,
        &redeem_infos,
        &[Signer::from(&market_seeds)],
    )?;

    // --- (11) program-signed transfer: redeemed KASS escrow → fee_destination
    // Escrow was drained to empty at `activate` (split consumed it in full) and
    // nothing refills it over the market's life, so its post-redeem balance is the
    // redeemed fee slice PLUS any residual dust — e.g. KASS a griefer donated into
    // `escrow_vault`, or rounding dust the redeem paid out. Sweeping all of it to
    // `fee_destination` is harmless: a donor only forfeits their own funds to the
    // protocol, and LP holders are unaffected (their claim is off `lp_total`, which
    // this ix reduces by exactly `fee_lp`). Mirrors `activate`'s drain-to-empty
    // residual convention. `redeem_tokens` likewise burns the FULL cyes/cno holder
    // balances, which are `remove_liquidity`'s proceeds plus any donated dust there.
    let fee_kass = {
        let d = escrow_ai.try_borrow()?;
        metadao::read_u64(&d, SPL_TOKEN_AMOUNT_OFFSET)?
    };
    if fee_kass > 0 {
        Transfer::new(escrow_ai, fee_dest_ai, market_ai, fee_kass)
            .invoke_signed(&[Signer::from(&market_seeds)])?;
    }

    // --- (12) persist: reduced lp_total + fee_collected (write once) --------
    let mut m = market;
    m.lp_total = m
        .lp_total
        .checked_sub(fee_lp)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    m.fee_collected = 1;
    write_market(market_ai, &m)?;
    Ok(())
}

/// Mark `fee_collected` without moving any value (the no-fee / nothing-to-collect
/// cases: no LP, no profit, or a floored-to-zero fee). Writes the market once.
fn write_collected(market_ai: &mut AccountView, market: Market) -> ProgramResult {
    let mut m = market;
    m.fee_collected = 1;
    write_market(market_ai, &m)
}

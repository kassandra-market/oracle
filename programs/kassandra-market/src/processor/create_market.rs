//! `create_market`: stand up a binary [`Market`] for a Kassandra oracle, its KASS
//! escrow token account, and the creator's [`Contribution`], transferring the
//! creator's `seed_amount` KASS into escrow.
//!
//! # PDA seeds (CONTRACT)
//! * Market:  `[b"market", oracle, [outcome_index]]`, program = [`crate::ID`] (one
//!   sub-market per outcome per oracle).
//! * Escrow:  `[b"escrow", market]`, an SPL token account on KASS whose token
//!   authority is the market PDA.
//! * Contribution: `[b"contribution", market, creator]`.
//!
//! # Instruction payload (after the 1-byte discriminant), exactly 9 bytes
//! `seed_amount: u64 LE` ++ `outcome_index: u8`.
//!
//! # Accounts
//! 0. config          — read-only; pins the canonical KASS mint + `min_liquidity`
//! 1. oracle          — read-only Kassandra oracle (owned by the Kassandra program)
//! 2. market PDA      — writable, uninitialized (created here)
//! 3. escrow PDA      — writable, uninitialized (created + initialized here)
//! 4. kass_mint       — read-only; must equal `config.kass_mint`
//! 5. creator         — signer, writable; pays rent, seeds the market
//! 6. creator_kass_ata — writable; KASS source, authority == creator
//! 7. contribution PDA — writable, uninitialized (created here)
//! 8. token program
//! 9. system program

use bytemuck::Zeroable;
use pinocchio::{
    account::AccountView, address::Address, cpi::Seed, error::ProgramError, ProgramResult,
};
use pinocchio_token::instructions::InitializeAccount3;

use crate::{
    cpi::spl::SPL_TOKEN_ACCOUNT_LEN,
    error::MarketError,
    processor::{
        contribution::record_contribution,
        guards::{
            assert_key, assert_signer, create_pda, load_config, load_kassandra_oracle,
            rent_exempt_lamports,
        },
    },
    state::{AccountType, Market, MarketStatus},
};

/// Exact payload length: seed_amount[8] ++ outcome_index[1].
const PAYLOAD_LEN: usize = 9;

pub fn process(
    program_id: &Address,
    accounts: &mut [AccountView],
    payload: &[u8],
) -> ProgramResult {
    if payload.len() != PAYLOAD_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let seed_amount = u64::from_le_bytes(payload[0..8].try_into().unwrap());
    let outcome_index = payload[8];

    let [config_ai, oracle_ai, market_ai, escrow_ai, kass_mint_ai, creator_ai, creator_ata_ai, contribution_ai, token_prog_ai, system_prog_ai, ..] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    assert_signer(creator_ai)?;
    assert_key(token_prog_ai, &pinocchio_token::ID)?;
    assert_key(system_prog_ai, &pinocchio_system::ID)?;

    let config = load_config(config_ai, program_id)?;
    if kass_mint_ai.address() != &config.kass_mint {
        return Err(MarketError::WrongMint.into());
    }
    // Early guard: the AUTHORITATIVE zero-amount invariant lives in
    // `record_contribution` (below). This mirror is only a fail-fast so we do not
    // allocate the market/escrow for a doomed tx — do not remove the one in
    // `record_contribution` in its favor.
    if seed_amount == 0 {
        return Err(MarketError::ZeroAmount.into());
    }

    let oracle = load_kassandra_oracle(oracle_ai)?;
    // The oracle guarantees `options_count >= 2`; this sub-market binds to one of
    // its outcomes, so `outcome_index` must index a real option.
    if outcome_index >= oracle.options_count {
        return Err(MarketError::InvalidOutcome.into());
    }
    if oracle.phase >= crate::kass_oracle::PHASE_RESOLVED {
        return Err(MarketError::OracleResolved.into());
    }

    let oidx = [outcome_index];
    let (market_key, market_bump) = Address::find_program_address(
        &[b"market", oracle_ai.address().as_ref(), &oidx],
        program_id,
    );
    assert_key(market_ai, &market_key)?;
    if market_ai.lamports() != 0 || !market_ai.is_data_empty() {
        return Err(MarketError::InvalidAccount.into()); // one sub-market per (oracle, outcome)
    }
    let (escrow_key, escrow_bump) =
        Address::find_program_address(&[b"escrow", market_ai.address().as_ref()], program_id);
    assert_key(escrow_ai, &escrow_key)?;

    // Allocate the market account (state written last).
    let market_rent = rent_exempt_lamports(Market::LEN)?;
    let mbump = [market_bump];
    let market_seeds = [
        Seed::from(b"market".as_ref()),
        Seed::from(oracle_ai.address().as_ref()),
        Seed::from(&oidx),
        Seed::from(&mbump),
    ];
    create_pda(
        creator_ai,
        market_ai,
        &market_seeds,
        market_rent,
        Market::LEN,
        program_id,
    )?;

    // Create the escrow token account owned by the market PDA.
    let vault_rent = rent_exempt_lamports(SPL_TOKEN_ACCOUNT_LEN)?;
    let ebump = [escrow_bump];
    let escrow_seeds = [
        Seed::from(b"escrow".as_ref()),
        Seed::from(market_ai.address().as_ref()),
        Seed::from(&ebump),
    ];
    create_pda(
        creator_ai,
        escrow_ai,
        &escrow_seeds,
        vault_rent,
        SPL_TOKEN_ACCOUNT_LEN,
        &pinocchio_token::ID,
    )?;
    InitializeAccount3 {
        account: escrow_ai,
        mint: kass_mint_ai,
        owner: market_ai.address(),
    }
    .invoke()?;

    // Transfer the creator's seed into escrow and record their Contribution.
    record_contribution(
        program_id,
        market_ai.address(),
        creator_ai,
        creator_ata_ai,
        escrow_ai,
        contribution_ai,
        token_prog_ai,
        creator_ai,
        seed_amount,
    )?;

    // Write the market state once.
    let mut market = Market::zeroed();
    market.account_type = AccountType::Market.as_u8();
    market.oracle = *oracle_ai.address();
    market.creator = *creator_ai.address();
    market.kass_mint = config.kass_mint;
    market.escrow_vault = *escrow_ai.address();
    market.min_liquidity = config.min_liquidity;
    market.fee_bps = config.fee_bps; // snapshot governance fee, immune to later changes
    market.total_contributed = seed_amount;
    market.open_contributions = 1; // the creator's Contribution (created by record_contribution above)
    market.status = MarketStatus::Funding.as_u8();
    market.bump = market_bump;
    market.escrow_bump = escrow_bump;
    market.outcome_index = outcome_index;
    {
        let mut d = market_ai.try_borrow_mut()?;
        d.copy_from_slice(bytemuck::bytes_of(&market));
    }
    Ok(())
}

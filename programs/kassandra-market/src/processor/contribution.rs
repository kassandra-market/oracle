//! Shared `record_contribution`: move KASS from a contributor's ATA into a
//! market's escrow, then create (or increment) that contributor's `Contribution`
//! PDA. Used by both `create_market` (the creator's seed) and `contribute`.

use bytemuck::Zeroable;
use pinocchio::{account::AccountView, address::Address, cpi::Seed, error::ProgramError};
use pinocchio_token::instructions::Transfer;

use crate::{
    error::MarketError,
    processor::guards::{
        assert_key, create_pda, load_contribution, rent_exempt_lamports, write_contribution,
    },
    state::{AccountType, Contribution},
};

/// Transfer `amount` KASS from `src_ata` (authority = the contributor signer)
/// into `escrow`, then create-or-increment the contributor's Contribution.
#[allow(clippy::too_many_arguments)]
pub fn record_contribution(
    program_id: &Address,
    market_key: &Address,
    contributor_ai: &AccountView,
    src_ata_ai: &AccountView,
    escrow_ai: &AccountView,
    contribution_ai: &mut AccountView,
    token_prog_ai: &AccountView,
    payer_ai: &AccountView,
    amount: u64,
) -> Result<(), ProgramError> {
    if amount == 0 {
        return Err(MarketError::ZeroAmount.into());
    }
    assert_key(token_prog_ai, &pinocchio_token::ID)?;

    let (expected, bump) = Address::find_program_address(
        &[
            b"contribution",
            market_key.as_ref(),
            contributor_ai.address().as_ref(),
        ],
        program_id,
    );
    assert_key(contribution_ai, &expected)?;

    // Move the KASS first (authority is the contributor signer).
    Transfer::new(src_ata_ai, escrow_ai, contributor_ai, amount).invoke()?;

    if contribution_ai.lamports() == 0 && contribution_ai.is_data_empty() {
        let rent = rent_exempt_lamports(Contribution::LEN)?;
        let bump_seed = [bump];
        let seeds = [
            Seed::from(b"contribution".as_ref()),
            Seed::from(market_key.as_ref()),
            Seed::from(contributor_ai.address().as_ref()),
            Seed::from(&bump_seed),
        ];
        create_pda(
            payer_ai,
            contribution_ai,
            &seeds,
            rent,
            Contribution::LEN,
            program_id,
        )?;
        let mut c = Contribution::zeroed();
        c.account_type = AccountType::Contribution.as_u8();
        c.market = *market_key;
        c.contributor = *contributor_ai.address();
        c.amount = amount;
        c.bump = bump;
        write_contribution(contribution_ai, &c)?;
    } else {
        let mut c = load_contribution(contribution_ai, program_id)?;
        c.amount = c
            .amount
            .checked_add(amount)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        write_contribution(contribution_ai, &c)?;
    }
    Ok(())
}

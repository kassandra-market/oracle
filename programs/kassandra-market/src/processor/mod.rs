use pinocchio::{account::AccountView, address::Address, error::ProgramError, ProgramResult};

use crate::instruction::Ix;

pub mod activate;
pub mod cancel;
pub mod claim_lp;
pub mod close_market;
pub mod collect_fee;
pub mod contribute;
pub mod contribution;
pub mod create_market;
pub mod guards;
pub mod init_config;
pub mod refund;
pub mod resolve_market;
pub mod update_config;

pub fn process(program_id: &Address, accounts: &mut [AccountView], data: &[u8]) -> ProgramResult {
    let (&disc, payload) = data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;
    match Ix::from_u8(disc).ok_or(ProgramError::InvalidInstructionData)? {
        Ix::InitConfig => init_config::process(program_id, accounts, payload),
        Ix::UpdateConfig => update_config::process(program_id, accounts, payload),
        Ix::CreateMarket => create_market::process(program_id, accounts, payload),
        Ix::Contribute => contribute::process(program_id, accounts, payload),
        Ix::Cancel => cancel::process(program_id, accounts, payload),
        Ix::Refund => refund::process(program_id, accounts, payload),
        Ix::Activate => activate::process(program_id, accounts, payload),
        Ix::ClaimLp => claim_lp::process(program_id, accounts, payload),
        Ix::ResolveMarket => resolve_market::process(program_id, accounts, payload),
        Ix::CollectFee => collect_fee::process(program_id, accounts, payload),
        Ix::CloseMarket => close_market::process(program_id, accounts, payload),
    }
}

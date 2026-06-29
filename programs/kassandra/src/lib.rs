#![allow(unexpected_cfgs)]
use pinocchio::{account_info::AccountInfo, pubkey::Pubkey, ProgramResult};

// `entrypoint!` also installs the default allocator and panic handler.
// Gated so the crate can be reused as a plain library (CPI helpers,
// discriminators, etc.) without emitting a second program entrypoint.
#[cfg(not(feature = "no-entrypoint"))]
use pinocchio::entrypoint;

#[cfg(not(feature = "no-entrypoint"))]
entrypoint!(process_instruction);

pub mod clock;
pub mod error;
pub mod instruction;
pub mod processor;
pub mod state;

pub const ID: Pubkey = pinocchio_pubkey::pubkey!("KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY");

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    processor::process(program_id, accounts, instruction_data)
}

use pinocchio::{account::AccountView, address::Address, error::ProgramError, ProgramResult};

use crate::{
    error::MarketError,
    processor::guards::{assert_key, assert_signer, load_config, read_token_mint, write_config},
    state::MAX_FEE_BPS,
};

/// min_liquidity[8] ++ fee_bps[2] ++ fee_destination[32]. The single
/// futarchy-gated setter updates all three at once.
const PAYLOAD_LEN: usize = 42;

pub fn process(
    program_id: &Address,
    accounts: &mut [AccountView],
    payload: &[u8],
) -> ProgramResult {
    if payload.len() != PAYLOAD_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let min_liquidity = u64::from_le_bytes(payload[0..8].try_into().unwrap());
    let fee_bps = u16::from_le_bytes(payload[8..10].try_into().unwrap());
    let mut fee_destination = [0u8; 32];
    fee_destination.copy_from_slice(&payload[10..42]);

    let [config_ai, authority_ai, fee_destination_ai, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    assert_signer(authority_ai)?;
    assert_key(fee_destination_ai, &Address::from(fee_destination))?;
    let config = load_config(config_ai, program_id)?;
    if authority_ai.address() != &config.authority {
        return Err(MarketError::Unauthorized.into());
    }

    // Governance guardrail: the protocol fee may not exceed MAX_FEE_BPS.
    if fee_bps > MAX_FEE_BPS {
        return Err(MarketError::InvalidFee.into());
    }
    // The fee destination must be an SPL token account on the canonical KASS mint.
    if read_token_mint(fee_destination_ai)? != config.kass_mint {
        return Err(MarketError::InvalidAccount.into());
    }

    let mut updated = config;
    updated.min_liquidity = min_liquidity;
    updated.fee_bps = fee_bps;
    updated.fee_destination = fee_destination.into();
    write_config(config_ai, &updated)?;
    Ok(())
}

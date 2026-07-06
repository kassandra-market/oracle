//! `init_config`: one-time creation of the [`Config`] singleton at `[b"config"]`.
//!
//! # Bootstrap-DoS resistance: create-or-adopt (Allocate + Assign)
//! The `[b"config"]` address is deterministic and known at deploy time, so an
//! attacker could pre-fund it with 1 lamport before the first `init_config`. A
//! plain system `CreateAccount` FAILS on any already-funded account, which would
//! brick genesis permanently. To tolerate a pre-funded singleton we instead
//! **adopt** it: top the balance up to rent-exempt with a system `Transfer`
//! (only if short), then system `Allocate` the space and `Assign` ownership to
//! this program — both signed by the config PDA seeds. A system-owned,
//! attacker-pre-funded account carries no data and can only be `Allocate`d by
//! the PDA signer, so adoption always succeeds.
//!
//! Idempotency is enforced by the **account-type tag, not lamports**: a real
//! second init finds an account already owned by this program AND stamped
//! [`AccountType::Config`], and fails [`MarketError::AlreadyInitialized`].

use bytemuck::Zeroable;
use pinocchio::{
    account::AccountView,
    address::Address,
    cpi::{Seed, Signer},
    error::ProgramError,
    ProgramResult,
};
use pinocchio_system::instructions::{Allocate, Assign, Transfer};

use crate::{
    error::MarketError,
    processor::guards::{
        assert_key, assert_owned_by_program, assert_signer, assert_upgrade_authority,
        read_token_mint, rent_exempt_lamports, write_config,
    },
    state::{AccountType, Config, MAX_FEE_BPS},
};

/// authority[32] ++ min_liquidity[8] ++ fee_bps[2] ++ fee_destination[32].
const PAYLOAD_LEN: usize = 74;

pub fn process(
    program_id: &Address,
    accounts: &mut [AccountView],
    payload: &[u8],
) -> ProgramResult {
    if payload.len() != PAYLOAD_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let mut authority = [0u8; 32];
    authority.copy_from_slice(&payload[0..32]);
    let min_liquidity = u64::from_le_bytes(payload[32..40].try_into().unwrap());
    let fee_bps = u16::from_le_bytes(payload[40..42].try_into().unwrap());
    let mut fee_destination = [0u8; 32];
    fee_destination.copy_from_slice(&payload[42..74]);

    let [config_ai, payer_ai, kass_mint_ai, fee_destination_ai, system_prog_ai, program_data_ai, ..] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    assert_signer(payer_ai)?;
    // Bootstrap authorization: the caller MUST be the program's on-chain upgrade
    // authority. This gates the otherwise-permissionless singleton init to the
    // deployer, so an attacker cannot front-run genesis and seize `Config.authority`
    // (the futarchy governance key, set from the payload below).
    assert_upgrade_authority(program_data_ai, payer_ai, program_id)?;
    assert_key(system_prog_ai, &pinocchio_system::ID)?;
    assert_key(fee_destination_ai, &Address::from(fee_destination))?;

    // Governance guardrail: the protocol fee may not exceed MAX_FEE_BPS.
    if fee_bps > MAX_FEE_BPS {
        return Err(MarketError::InvalidFee.into());
    }

    let (expected, bump) = Address::find_program_address(&[b"config"], program_id);
    assert_key(config_ai, &expected)?;

    // Re-init guard via the account-type TAG (not lamports): a genuine second
    // init finds the account already owned by this program AND stamped `Config`.
    // A freshly-created or attacker-pre-funded-but-system-owned account is not
    // yet program-owned (or carries a zero/`Uninitialized` tag), so adoption
    // below proceeds.
    if config_ai.owned_by(program_id) {
        let data = config_ai.try_borrow()?;
        if data.len() >= Config::LEN && data[0] == AccountType::Config.as_u8() {
            return Err(MarketError::AlreadyInitialized.into());
        }
    }

    // Cheap defense-in-depth: the recorded KASS mint must be an SPL token-program
    // account (not an arbitrary key), so downstream fee/escrow logic can trust it.
    assert_owned_by_program(kass_mint_ai, &pinocchio_token::ID)?;

    // The fee destination must be an SPL token account (owned by the token program)
    // whose mint (bytes 0..32) is the canonical KASS mint, so fees route to KASS.
    if read_token_mint(fee_destination_ai)? != *kass_mint_ai.address() {
        return Err(MarketError::InvalidAccount.into());
    }

    // --- create-or-adopt the Config account (program-signed) ----------------
    let rent = rent_exempt_lamports(Config::LEN)?;
    let bump_seed = [bump];
    let signer_seeds = [Seed::from(b"config".as_ref()), Seed::from(&bump_seed)];

    // Top the (possibly pre-funded) account up to rent-exempt, only if short.
    let current = config_ai.lamports();
    if current < rent {
        Transfer {
            from: payer_ai,
            to: config_ai,
            lamports: rent - current,
        }
        .invoke()?;
    }
    // Allocate the data and take ownership — both signed by the PDA. Tolerates a
    // pre-funded account where a plain CreateAccount would fail.
    Allocate {
        account: config_ai,
        space: Config::LEN as u64,
    }
    .invoke_signed(&[Signer::from(&signer_seeds)])?;
    Assign {
        account: config_ai,
        owner: program_id,
    }
    .invoke_signed(&[Signer::from(&signer_seeds)])?;

    let mut config = Config::zeroed();
    config.account_type = AccountType::Config.as_u8();
    config.authority = authority.into();
    config.kass_mint = *kass_mint_ai.address();
    config.min_liquidity = min_liquidity;
    config.bump = bump;
    config.fee_bps = fee_bps;
    config.fee_destination = fee_destination.into();
    write_config(config_ai, &config)?;
    Ok(())
}

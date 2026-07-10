use pinocchio::{
    account::AccountView,
    address::Address,
    cpi::{Seed, Signer},
    error::ProgramError,
    sysvars::{rent::Rent, Sysvar},
    ProgramResult,
};
use pinocchio_system::instructions::CreateAccount;

/// Rent-exempt minimum balance for an account of `len` bytes.
///
/// DEVIATION (pinocchio 0.11 Rent): `Rent::try_minimum_balance(len)` computes
/// `(ACCOUNT_STORAGE_OVERHEAD + len) * lamports_per_byte`, but pinocchio 0.11
/// loads only the Rent sysvar's first `u64` (`lamports_per_byte_year`) into its
/// single `lamports_per_byte` field and drops the `exemption_threshold` factor —
/// so it returns HALF the balance the runtime (and pinocchio 0.8's
/// `Rent::minimum_balance`) require, which fails `InsufficientFundsForRent`.
/// `exemption_threshold` is the chain-wide genesis constant `2.0` (pinocchio 0.8
/// multiplies by `DEFAULT_EXEMPTION_THRESHOLD_AS_U64 == 2` on this default path),
/// so doubling restores the pre-migration program's byte-identical rent value on
/// every cluster (the Rent sysvar's on-wire bincode layout is fixed).
pub fn rent_exempt_lamports(len: usize) -> Result<u64, ProgramError> {
    Rent::get()?
        .try_minimum_balance(len)?
        .checked_mul(2)
        .ok_or(ProgramError::ArithmeticOverflow)
}

use crate::cpi::metadao;
use crate::cpi::spl::{SPL_TOKEN_MINT_OFFSET, SPL_TOKEN_OWNER_OFFSET};
use crate::error::MarketError;
use crate::kass_oracle::{KassOracle, KASSANDRA_PROGRAM_ID, ORACLE_ACCOUNT_TYPE, ORACLE_LEN};
use crate::state::{AccountType, Config, Contribution, Market};

pub fn assert_owned_by_program(a: &AccountView, program_id: &Address) -> ProgramResult {
    if !a.owned_by(program_id) {
        return Err(MarketError::InvalidAccount.into());
    }
    Ok(())
}
pub fn assert_signer(a: &AccountView) -> ProgramResult {
    if !a.is_signer() {
        return Err(MarketError::Unauthorized.into());
    }
    Ok(())
}
pub fn assert_key(a: &AccountView, expected: &Address) -> ProgramResult {
    if a.address() != expected {
        return Err(MarketError::InvalidAccount.into());
    }
    Ok(())
}

/// The BPF Upgradeable Loader — owns every upgradeable program's `ProgramData`.
pub const BPF_UPGRADEABLE_LOADER_ID: Address =
    Address::from_str_const("BPFLoaderUpgradeab1e11111111111111111111111");

// `UpgradeableLoaderState::ProgramData` bincode layout: variant `u32 @0` (== 3),
// `slot: u64 @4`, `upgrade_authority: Option<Pubkey>` = tag `@12` (1 = Some) then
// the 32-byte key `@13..45`. `size_of_programdata_metadata() == 45`.
const PROGRAMDATA_VARIANT_PROGRAMDATA: u32 = 3;
const PROGRAMDATA_OPTION_OFFSET: usize = 12;
const PROGRAMDATA_AUTHORITY_OFFSET: usize = 13;
const PROGRAMDATA_METADATA_LEN: usize = 45;

/// Assert `signer` is THIS program's on-chain **upgrade authority**, proven via the
/// BPF-Upgradeable-Loader `ProgramData` account.
///
/// Pins `program_data` to the canonical `ProgramData` PDA of `program_id`
/// (`find_program_address([program_id], loader)`), checks it is loader-owned, then
/// reads its `Some(upgrade_authority)` and requires it equals `signer`. Rejects an
/// immutable program (authority `None`) — there is no authority to match. This gates
/// the permissionless `init_config` bootstrap to the deployer, so an attacker cannot
/// front-run genesis to seize `Config.authority`.
pub fn assert_upgrade_authority(
    program_data: &AccountView,
    signer: &AccountView,
    program_id: &Address,
) -> ProgramResult {
    let (expected, _) =
        Address::find_program_address(&[program_id.as_ref()], &BPF_UPGRADEABLE_LOADER_ID);
    if program_data.address() != &expected {
        return Err(MarketError::InvalidAccount.into());
    }
    if !program_data.owned_by(&BPF_UPGRADEABLE_LOADER_ID) {
        return Err(MarketError::InvalidAccount.into());
    }
    let data = program_data.try_borrow()?;
    if data.len() < PROGRAMDATA_METADATA_LEN {
        return Err(MarketError::InvalidAccount.into());
    }
    let variant = u32::from_le_bytes(data[0..4].try_into().unwrap());
    if variant != PROGRAMDATA_VARIANT_PROGRAMDATA {
        return Err(MarketError::InvalidAccount.into());
    }
    // `Option<Pubkey>`: 1 == Some. An immutable program (None/0) has no authority.
    if data[PROGRAMDATA_OPTION_OFFSET] != 1 {
        return Err(MarketError::NotUpgradeAuthority.into());
    }
    let authority = &data[PROGRAMDATA_AUTHORITY_OFFSET..PROGRAMDATA_AUTHORITY_OFFSET + 32];
    if !signer.is_signer() || signer.address().as_ref() != authority {
        return Err(MarketError::NotUpgradeAuthority.into());
    }
    Ok(())
}

/// Close a program-owned DATA account: drain ALL its lamports to `recipient`,
/// then `close()` it. Mirrors the sibling Kassandra program's PDA-close idiom
/// (`sweep_oracle` / `close_market`). The account MUST be owned by this program (so
/// `close()` is permitted) and MUST NOT be a token account (rent → recipient, no SPL
/// CPI). Leaves NO lamport dust — every lamport moves to `recipient`.
///
/// NOTE (pinocchio semantics): `close()` zeroes the account's OWNER, LAMPORTS, and
/// DATA-LENGTH, but does NOT wipe the data BODY (the runtime reclaims it at tx end /
/// next CPI). So "reaped / no dust" means the account is emptied and de-owned — it is
/// NOT "data scrubbed / reinit-safe": re-creating the PDA still goes through the
/// status-gated create paths (`create_market` / `activate`), never a bare re-adopt.
pub fn close_data_account(account: &mut AccountView, recipient: &mut AccountView) -> ProgramResult {
    let amt = account.lamports();
    recipient.set_lamports(
        recipient
            .lamports()
            .checked_add(amt)
            .ok_or(ProgramError::ArithmeticOverflow)?,
    );
    account.set_lamports(0);
    account.close()
}

pub fn create_pda(
    payer: &AccountView,
    pda: &AccountView,
    seeds: &[Seed],
    lamports: u64,
    space: usize,
    owner: &Address,
) -> ProgramResult {
    CreateAccount {
        from: payer,
        to: pda,
        lamports,
        space: space as u64,
        owner,
    }
    .invoke_signed(&[Signer::from(seeds)])
}

/// Overwrite the [`Market`] state of a program-owned market account. Writes the
/// first `Market::LEN` bytes only (the account may be larger) — the shared idiom
/// behind every processor that persists a mutated market.
pub fn write_market(market_ai: &mut AccountView, m: &Market) -> ProgramResult {
    let mut d = market_ai.try_borrow_mut()?;
    d[..Market::LEN].copy_from_slice(bytemuck::bytes_of(m));
    Ok(())
}

/// Overwrite the first `Config::LEN` bytes of a program-owned config account
/// (sibling of [`write_market`]).
pub fn write_config(config_ai: &mut AccountView, c: &Config) -> ProgramResult {
    let mut d = config_ai.try_borrow_mut()?;
    d[..Config::LEN].copy_from_slice(bytemuck::bytes_of(c));
    Ok(())
}

/// Overwrite the first `Contribution::LEN` bytes of a program-owned contribution
/// account (sibling of [`write_market`]).
pub fn write_contribution(contribution_ai: &mut AccountView, c: &Contribution) -> ProgramResult {
    let mut d = contribution_ai.try_borrow_mut()?;
    d[..Contribution::LEN].copy_from_slice(bytemuck::bytes_of(c));
    Ok(())
}

/// Assert `ai` is owned by the SPL token program, then read its `mint` field
/// (bytes `0..32`). Both a wrong owner and a too-short buffer yield
/// [`MarketError::InvalidAccount`]; the CALLER compares the returned mint against
/// its expected value and chooses the mismatch error (e.g. `InvalidAccount` vs
/// `WrongMint`). Shared by every processor that must confirm a passed token
/// account sits on a specific mint.
pub fn read_token_mint(ai: &AccountView) -> Result<Address, ProgramError> {
    assert_owned_by_program(ai, &pinocchio_token::ID)?;
    let d = ai.try_borrow()?;
    metadao::read_pubkey(&d, SPL_TOKEN_MINT_OFFSET)
}

/// Assert `ai` is owned by the SPL token program, then read its `owner` field
/// (bytes `32..64`). Wrong owner / too-short buffer → [`MarketError::InvalidAccount`];
/// the CALLER compares the returned owner and chooses the mismatch error. Sibling
/// of [`read_token_mint`].
pub fn read_token_owner(ai: &AccountView) -> Result<Address, ProgramError> {
    assert_owned_by_program(ai, &pinocchio_token::ID)?;
    let d = ai.try_borrow()?;
    metadao::read_pubkey(&d, SPL_TOKEN_OWNER_OFFSET)
}

/// Build the Market-PDA signer seeds (`[b"market", oracle, [outcome_index], [bump]]`)
/// from a loaded `Market`, for a program-signed CPI.
///
/// Each `Seed` borrows a byte array (`[outcome_index]` / `[bump]`), so those arrays
/// must outlive the returned `Signer`. A plain function cannot express that — the
/// arrays would be dropped on return. This macro instead binds the two byte arrays
/// (`$oidx`, `$bump`) AND the seeds array (`$seeds`) in the CALLER's scope, so the
/// borrows live as long as the caller needs them.
///
/// Usage:
/// ```ignore
/// market_signer_seeds!(market, oidx, mbump, market_seeds);
/// // ... market_seeds valid here ...
/// Signer::from(&market_seeds)
/// ```
macro_rules! market_signer_seeds {
    ($market:expr, $oidx:ident, $bump:ident, $seeds:ident) => {
        let $oidx = [$market.outcome_index];
        let $bump = [$market.bump];
        let $seeds = [
            ::pinocchio::cpi::Seed::from(b"market".as_ref()),
            ::pinocchio::cpi::Seed::from($market.oracle.as_ref()),
            ::pinocchio::cpi::Seed::from(&$oidx),
            ::pinocchio::cpi::Seed::from(&$bump),
        ];
    };
}
pub(crate) use market_signer_seeds;

macro_rules! loader {
    ($name:ident, $ty:ident, $tag:expr) => {
        pub fn $name(a: &AccountView, program_id: &Address) -> Result<$ty, ProgramError> {
            assert_owned_by_program(a, program_id)?;
            if a.data_len() < $ty::LEN {
                return Err(MarketError::InvalidAccount.into());
            }
            let v: $ty = {
                let d = a.try_borrow()?;
                bytemuck::pod_read_unaligned::<$ty>(&d[..$ty::LEN])
            };
            if v.account_type != $tag.as_u8() {
                return Err(MarketError::InvalidAccount.into());
            }
            Ok(v)
        }
    };
}
loader!(load_config, Config, AccountType::Config);
loader!(load_market, Market, AccountType::Market);
loader!(load_contribution, Contribution, AccountType::Contribution);

/// Read a Kassandra `Oracle` account: it must be owned by the Kassandra program,
/// large enough, and tagged `AccountType::Oracle` (reject type-confusion).
pub fn load_kassandra_oracle(a: &AccountView) -> Result<KassOracle, ProgramError> {
    if !a.owned_by(&KASSANDRA_PROGRAM_ID) {
        return Err(MarketError::InvalidAccount.into());
    }
    if a.data_len() < ORACLE_LEN {
        return Err(MarketError::InvalidAccount.into());
    }
    let o = {
        let d = a.try_borrow()?;
        KassOracle::read(&d[..ORACLE_LEN])
    };
    if o.account_type != ORACLE_ACCOUNT_TYPE {
        return Err(MarketError::InvalidAccount.into());
    }
    Ok(o)
}

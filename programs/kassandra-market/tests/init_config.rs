mod common;
use common::*;
use kassandra_market_program::error::MarketError;
use kassandra_market_program::state::{AccountType, Config, MAX_FEE_BPS};
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

#[test]
fn init_config_happy_path() {
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new().pubkey();
    let fee_destination = ctx.create_token_account(kass, authority, 0);
    let (cfg_pda, res) = ctx.init_config_full(authority, kass, 1_000_000_000, 250, fee_destination);
    assert!(res.is_ok(), "{res:?}");
    let c: Config = ctx.read_pod(cfg_pda);
    assert_eq!(c.account_type, AccountType::Config.as_u8());
    assert_eq!(c.authority.to_bytes(), authority.to_bytes());
    assert_eq!(c.kass_mint.to_bytes(), kass.to_bytes());
    assert_eq!(c.min_liquidity, 1_000_000_000);
    assert_eq!(c.fee_bps, 250);
    assert_eq!(c.fee_destination.to_bytes(), fee_destination.to_bytes());
}

#[test]
fn init_config_rejects_fee_over_max() {
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new().pubkey();
    let fee_destination = ctx.create_token_account(kass, authority, 0);
    let (_pda, res) = ctx.init_config_full(
        authority,
        kass,
        1_000_000_000,
        MAX_FEE_BPS + 1,
        fee_destination,
    );
    assert_eq!(custom_code(&res), Some(MarketError::InvalidFee as u32));
}

#[test]
fn init_config_rejects_wrong_fee_mint() {
    // A fee_destination token account on a mint other than KASS must be rejected.
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let other = ctx.create_mint(9);
    let authority = Keypair::new().pubkey();
    let bad_destination = ctx.create_token_account(other, authority, 0);
    let (_pda, res) = ctx.init_config_full(authority, kass, 1_000_000_000, 100, bad_destination);
    assert_eq!(custom_code(&res), Some(MarketError::InvalidAccount as u32));
}

#[test]
fn init_config_twice_fails() {
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new().pubkey();
    let (_pda, res1) = ctx.init_config(authority, kass, 1);
    assert!(res1.is_ok());
    // The re-init guard is tag-based: the existing account is already stamped
    // `Config`, so the second init rejects with `AlreadyInitialized`.
    let (_pda, res2) = ctx.init_config(authority, kass, 2);
    assert_eq!(
        custom_code(&res2),
        Some(MarketError::AlreadyInitialized as u32)
    );
}

#[test]
fn init_config_rejects_non_mint_kass() {
    // A `kass_mint` that is not an SPL mint (here: a nonexistent, non-token-owned
    // key) must be rejected — the config must record a real KASS mint.
    let mut ctx = TestCtx::new();
    let fake_mint = Pubkey::new_unique();
    let authority = Keypair::new().pubkey();
    let (_pda, res) = ctx.init_config(authority, fake_mint, 1_000_000_000);
    assert_eq!(custom_code(&res), Some(MarketError::InvalidAccount as u32));
}

#[test]
fn init_config_rejects_non_upgrade_authority() {
    // Front-run defense: a caller who is NOT the program's on-chain upgrade
    // authority cannot bootstrap the Config singleton. The harness fabricates the
    // ProgramData with `ctx.payer` as the upgrade authority; an attacker signing
    // with a different key must be rejected with `NotUpgradeAuthority`, so it
    // cannot seize `Config.authority` by racing genesis.
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new().pubkey();
    let fee_destination = ctx.create_token_account(kass, authority, 0);
    let attacker = Keypair::new();
    let res = ctx.init_config_signed_by(
        &attacker,
        authority,
        kass,
        1_000_000_000,
        100,
        fee_destination,
    );
    assert_eq!(
        custom_code(&res),
        Some(MarketError::NotUpgradeAuthority as u32)
    );
    // And the canonical upgrade authority (the harness payer) still succeeds.
    let (_pda, ok) = ctx.init_config_full(authority, kass, 1_000_000_000, 100, fee_destination);
    assert!(ok.is_ok(), "{ok:?}");
}

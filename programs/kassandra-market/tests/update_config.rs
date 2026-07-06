mod common;
use common::*;
use kassandra_market_program::error::MarketError;
use kassandra_market_program::state::{Config, MAX_FEE_BPS};
use solana_sdk::signature::{Keypair, Signer};

#[test]
fn update_config_by_authority_ok() {
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    let (cfg, _) = ctx.init_config(authority.pubkey(), kass, 100);
    // The futarchy setter updates min_liquidity + fee_bps + fee_destination together.
    let fee_destination = ctx.create_token_account(kass, authority.pubkey(), 0);
    let res = ctx.update_config_full(&authority, 500, 250, fee_destination);
    assert!(res.is_ok(), "{res:?}");
    let c: Config = ctx.read_pod(cfg);
    assert_eq!(c.min_liquidity, 500);
    assert_eq!(c.fee_bps, 250);
    assert_eq!(c.fee_destination.to_bytes(), fee_destination.to_bytes());
}

#[test]
fn update_config_by_stranger_fails() {
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    let _ = ctx.init_config(authority.pubkey(), kass, 100);
    let stranger = Keypair::new();
    ctx.svm_airdrop(&stranger.pubkey());
    let res = ctx.update_config(&stranger, kass, 999);
    assert_eq!(custom_code(&res), Some(MarketError::Unauthorized as u32));
}

#[test]
fn update_config_rejects_fee_over_max() {
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    let _ = ctx.init_config(authority.pubkey(), kass, 100);
    let fee_destination = ctx.create_token_account(kass, authority.pubkey(), 0);
    let res = ctx.update_config_full(&authority, 500, MAX_FEE_BPS + 1, fee_destination);
    assert_eq!(custom_code(&res), Some(MarketError::InvalidFee as u32));
}

#[test]
fn update_config_rejects_wrong_fee_mint() {
    // A fee_destination on a mint other than the config's KASS mint is rejected.
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let other = ctx.create_mint(9);
    let authority = Keypair::new();
    let _ = ctx.init_config(authority.pubkey(), kass, 100);
    let bad_destination = ctx.create_token_account(other, authority.pubkey(), 0);
    let res = ctx.update_config_full(&authority, 500, 100, bad_destination);
    assert_eq!(custom_code(&res), Some(MarketError::InvalidAccount as u32));
}

//! Integration tests for `cancel` (marks an under-funded market `Cancelled`
//! once its underlying Kassandra oracle is terminal).

mod common;
use common::*;
use kassandra_markets_program::error::MarketError;
use kassandra_markets_program::state::{Market, MarketStatus};
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

const PROPOSAL: u8 = 1; // kassandra Phase::Proposal
const RESOLVED: u8 = 7; // kassandra Phase::Resolved
const INVALID_DEADEND: u8 = 8; // kassandra Phase::InvalidDeadend

const MIN_LIQUIDITY: u64 = 1_000_000_000;

/// Stand up an under-funded `Funding` market (seed 200M < min 1B). Returns the
/// context, the oracle pubkey, and the market PDA.
fn setup_underfunded_market() -> (TestCtx, Pubkey /*oracle*/, Pubkey /*market*/) {
    setup_market_with_seed(200_000_000)
}

/// Stand up a `Funding` market seeding `seed` KASS against a fresh oracle in the
/// (non-terminal) Proposal phase.
fn setup_market_with_seed(seed: u64) -> (TestCtx, Pubkey /*oracle*/, Pubkey /*market*/) {
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    let (_config, res) = ctx.init_config(authority.pubkey(), kass, MIN_LIQUIDITY);
    assert!(res.is_ok(), "{res:?}");

    let oracle = ctx.seed_kass_oracle(2, PROPOSAL);
    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 2_000_000_000);
    let (market, res) = ctx.create_market(&creator, oracle, kass, creator_ata, seed);
    assert!(res.is_ok(), "{res:?}");
    (ctx, oracle, market)
}

#[test]
fn cancel_happy_resolved() {
    let (mut ctx, oracle, market) = setup_underfunded_market();
    ctx.set_oracle_phase(oracle, RESOLVED);

    let res = ctx.cancel(market, oracle);
    assert!(res.is_ok(), "{res:?}");

    let m: Market = ctx.read_pod(market);
    assert_eq!(m.status, MarketStatus::Cancelled.as_u8());
}

#[test]
fn cancel_happy_invalid_deadend() {
    let (mut ctx, oracle, market) = setup_underfunded_market();
    ctx.set_oracle_phase(oracle, INVALID_DEADEND);

    let res = ctx.cancel(market, oracle);
    assert!(res.is_ok(), "{res:?}");

    let m: Market = ctx.read_pod(market);
    assert_eq!(m.status, MarketStatus::Cancelled.as_u8());
}

#[test]
fn cancel_rejects_oracle_not_terminal() {
    // Oracle left in the non-terminal Proposal phase.
    let (mut ctx, oracle, market) = setup_underfunded_market();

    let res = ctx.cancel(market, oracle);
    assert_eq!(
        custom_code(&res),
        Some(MarketError::OracleNotTerminal as u32)
    );
}

#[test]
fn cancel_succeeds_even_when_fully_funded() {
    // Seed == min: a terminal oracle makes Phase-2 `activate` impossible, so
    // cancel+refund must remain available even for a fully-funded market —
    // otherwise its contributions would be stranded forever.
    let (mut ctx, oracle, market) = setup_market_with_seed(MIN_LIQUIDITY);
    ctx.set_oracle_phase(oracle, RESOLVED);

    let res = ctx.cancel(market, oracle);
    assert!(res.is_ok(), "{res:?}");

    let m: Market = ctx.read_pod(market);
    assert_eq!(m.status, MarketStatus::Cancelled.as_u8());
}

#[test]
fn cancel_rejects_not_funding_idempotent() {
    let (mut ctx, oracle, market) = setup_underfunded_market();
    ctx.set_oracle_phase(oracle, RESOLVED);

    let res = ctx.cancel(market, oracle);
    assert!(res.is_ok(), "{res:?}");

    // A second cancel finds the market already Cancelled.
    let res = ctx.cancel(market, oracle);
    assert_eq!(custom_code(&res), Some(MarketError::NotFunding as u32));
}

//! Integration tests for `create_market` (+ the shared `record_contribution`).

mod common;
use common::*;
use kassandra_market_program::error::MarketError;
use kassandra_market_program::state::{Market, MarketStatus};
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

const PROPOSAL: u8 = 1; // kassandra Phase::Proposal
const RESOLVED: u8 = 7; // kassandra Phase::Resolved
const INVALID_DEADEND: u8 = 8; // kassandra Phase::InvalidDeadend (other terminal phase)

fn setup() -> (TestCtx, Pubkey /*kass*/, Keypair /*authority*/) {
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    let (_config, res) = ctx.init_config(authority.pubkey(), kass, 1_000_000_000);
    assert!(res.is_ok(), "{res:?}");
    (ctx, kass, authority)
}

#[test]
fn create_market_happy_path() {
    let (mut ctx, kass, _auth) = setup();
    let oracle = ctx.seed_kass_oracle(2, PROPOSAL);
    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 500_000_000);
    let (market, res) = ctx.create_market(&creator, oracle, kass, creator_ata, 200_000_000);
    assert!(res.is_ok(), "{res:?}");
    let m: Market = ctx.read_pod(market);
    assert_eq!(m.status, MarketStatus::Funding.as_u8());
    assert_eq!(m.total_contributed, 200_000_000);
    // The creator's Contribution is the first live contribution.
    assert_eq!(m.open_contributions, 1, "counter starts at 1 (creator)");
    assert_eq!(m.min_liquidity, 1_000_000_000);
    // The default harness init_config sets fee_bps = 100; the market snapshots it.
    assert_eq!(m.fee_bps, 100);
    assert_eq!(m.oracle.to_bytes(), oracle.to_bytes());
    assert_eq!(m.creator.to_bytes(), creator.pubkey().to_bytes());
    assert_eq!(m.kass_mint.to_bytes(), kass.to_bytes());
    assert_eq!(
        ctx.token_balance(Pubkey::new_from_array(m.escrow_vault.to_bytes())),
        200_000_000
    );
    assert_eq!(ctx.token_balance(creator_ata), 300_000_000);

    // The creator's Contribution PDA records the seed.
    let (contribution, _) = kassandra_markets_sdk::pda::contribution(&market, &creator.pubkey());
    let c: kassandra_market_program::state::Contribution = ctx.read_pod(contribution);
    assert_eq!(c.amount, 200_000_000);
    assert_eq!(c.contributor.to_bytes(), creator.pubkey().to_bytes());
    assert_eq!(c.market.to_bytes(), market.to_bytes());
}

#[test]
fn create_market_snapshots_configured_fee() {
    // A market snapshots the Config's fee_bps at creation (config-as-state), so it
    // is immune to later governance changes.
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    let fee_destination = ctx.create_token_account(kass, authority.pubkey(), 0);
    let (_config, res) = ctx.init_config_full(
        authority.pubkey(),
        kass,
        1_000_000_000,
        750,
        fee_destination,
    );
    assert!(res.is_ok(), "{res:?}");
    let oracle = ctx.seed_kass_oracle(2, PROPOSAL);
    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 500_000_000);
    let (market, res) = ctx.create_market(&creator, oracle, kass, creator_ata, 200_000_000);
    assert!(res.is_ok(), "{res:?}");
    let m: Market = ctx.read_pod(market);
    assert_eq!(m.fee_bps, 750);
}

#[test]
fn create_market_categorical_sub_markets_per_outcome() {
    // A 3-option oracle is modeled as three independent binary sub-markets, one
    // per outcome_index; each lands at a DISTINCT market PDA and all succeed.
    let (mut ctx, kass, _auth) = setup();
    let oracle = ctx.seed_kass_oracle(3, PROPOSAL);
    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 3_000_000_000);

    let mut markets = Vec::new();
    for outcome_index in 0u8..3 {
        let (market, res) = ctx.create_market_full(
            &creator,
            oracle,
            kass,
            creator_ata,
            200_000_000,
            outcome_index,
        );
        assert!(res.is_ok(), "outcome {outcome_index}: {res:?}");
        let m: Market = ctx.read_pod(market);
        assert_eq!(m.outcome_index, outcome_index, "outcome_index stored");
        assert_eq!(m.oracle.to_bytes(), oracle.to_bytes());
        assert_eq!(m.status, MarketStatus::Funding.as_u8());
        markets.push(market);
    }
    // Distinct PDAs per outcome.
    assert_ne!(markets[0], markets[1]);
    assert_ne!(markets[1], markets[2]);
    assert_ne!(markets[0], markets[2]);
}

#[test]
fn create_market_rejects_outcome_index_out_of_range() {
    // outcome_index must be < options_count; option 3 does not exist on a 3-option
    // oracle (valid indices 0/1/2).
    let (mut ctx, kass, _auth) = setup();
    let oracle = ctx.seed_kass_oracle(3, PROPOSAL);
    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 500_000_000);
    let (_market, res) =
        ctx.create_market_full(&creator, oracle, kass, creator_ata, 200_000_000, 3);
    assert_eq!(custom_code(&res), Some(MarketError::InvalidOutcome as u32));
}

#[test]
fn create_market_rejects_duplicate_outcome() {
    // Two creates against the same (oracle, outcome_index) collide at one PDA.
    let (mut ctx, kass, _auth) = setup();
    let oracle = ctx.seed_kass_oracle(3, PROPOSAL);
    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 500_000_000);
    let (_m, res) = ctx.create_market_full(&creator, oracle, kass, creator_ata, 100_000_000, 1);
    assert!(res.is_ok(), "{res:?}");
    let (_m, res2) = ctx.create_market_full(&creator, oracle, kass, creator_ata, 100_000_000, 1);
    assert_eq!(custom_code(&res2), Some(MarketError::InvalidAccount as u32));
}

#[test]
fn create_market_rejects_zero_amount() {
    let (mut ctx, kass, _auth) = setup();
    let oracle = ctx.seed_kass_oracle(2, PROPOSAL);
    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 500_000_000);
    let (_market, res) = ctx.create_market(&creator, oracle, kass, creator_ata, 0);
    assert_eq!(custom_code(&res), Some(MarketError::ZeroAmount as u32));
}

#[test]
fn create_market_rejects_resolved_oracle() {
    let (mut ctx, kass, _auth) = setup();
    let oracle = ctx.seed_kass_oracle(2, RESOLVED);
    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 500_000_000);
    let (_market, res) = ctx.create_market(&creator, oracle, kass, creator_ata, 200_000_000);
    assert_eq!(custom_code(&res), Some(MarketError::OracleResolved as u32));
}

#[test]
fn create_market_rejects_wrong_mint() {
    let (mut ctx, _kass, _auth) = setup();
    let other_mint = ctx.create_mint(9);
    let oracle = ctx.seed_kass_oracle(2, PROPOSAL);
    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(other_mint, creator.pubkey(), 500_000_000);
    let (_market, res) = ctx.create_market(&creator, oracle, other_mint, creator_ata, 200_000_000);
    assert_eq!(custom_code(&res), Some(MarketError::WrongMint as u32));
}

#[test]
fn create_market_rejects_duplicate() {
    let (mut ctx, kass, _auth) = setup();
    let oracle = ctx.seed_kass_oracle(2, PROPOSAL);
    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 500_000_000);
    let (_market, res) = ctx.create_market(&creator, oracle, kass, creator_ata, 100_000_000);
    assert!(res.is_ok(), "{res:?}");
    // A second create against the same oracle must fail (one market per oracle).
    let (_market, res2) = ctx.create_market(&creator, oracle, kass, creator_ata, 100_000_000);
    assert_eq!(custom_code(&res2), Some(MarketError::InvalidAccount as u32));
}

#[test]
fn create_market_rejects_invalid_deadend_oracle() {
    // Phase 8 (InvalidDeadend) is the other terminal phase; `phase >= Resolved(7)`
    // must reject it too, not just phase 7.
    let (mut ctx, kass, _auth) = setup();
    let oracle = ctx.seed_kass_oracle(2, INVALID_DEADEND);
    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 500_000_000);
    let (_market, res) = ctx.create_market(&creator, oracle, kass, creator_ata, 200_000_000);
    assert_eq!(custom_code(&res), Some(MarketError::OracleResolved as u32));
}

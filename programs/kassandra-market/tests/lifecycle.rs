//! End-to-end integration test for the Phase 1 crowdfunding lifecycle:
//! `create_market` → `contribute` → `cancel` → `refund`, with three
//! participants (creator + 2 contributors) all staying under `min_liquidity`
//! so the market is genuinely cancellable and everyone is refunded in full.

mod common;
use common::*;
use kassandra_market_program::state::{Market, MarketStatus};
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

const PROPOSAL: u8 = 1;
const RESOLVED: u8 = 7;

#[test]
fn full_crowdfunding_lifecycle_cancel_and_refund() {
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    // min_liquidity high enough that creator+2 contributors stay under it.
    let _ = ctx.init_config(authority.pubkey(), kass, 10_000_000_000);

    let oracle = ctx.seed_kass_oracle(2, PROPOSAL);

    // Creator seeds the market (below min).
    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 1_000_000_000);
    let (market, res) = ctx.create_market(&creator, oracle, kass, creator_ata, 400_000_000);
    assert!(res.is_ok(), "create_market: {res:?}");

    // Two more contributors (still under min).
    let c1 = Keypair::new();
    ctx.svm_airdrop(&c1.pubkey());
    let c1_ata = ctx.create_token_account(kass, c1.pubkey(), 1_000_000_000);
    assert!(ctx.contribute(&c1, market, c1_ata, 300_000_000).is_ok());

    let c2 = Keypair::new();
    ctx.svm_airdrop(&c2.pubkey());
    let c2_ata = ctx.create_token_account(kass, c2.pubkey(), 1_000_000_000);
    assert!(ctx.contribute(&c2, market, c2_ata, 250_000_000).is_ok());

    // Total contributed = 950M < 10B min.
    let m: Market = ctx.read_pod(market);
    assert_eq!(m.total_contributed, 950_000_000);
    assert_eq!(m.status, MarketStatus::Funding.as_u8());
    let escrow = Pubkey::new_from_array(m.escrow_vault.to_bytes());
    assert_eq!(ctx.token_balance(escrow), 950_000_000);

    // Oracle resolves while still under-funded → cancel.
    ctx.set_oracle_phase(oracle, RESOLVED);
    assert!(ctx.cancel(market, oracle).is_ok());
    let m: Market = ctx.read_pod(market);
    assert_eq!(m.status, MarketStatus::Cancelled.as_u8());

    // Everyone refunded, in full.
    assert!(ctx.refund(market, creator.pubkey(), creator_ata).is_ok());
    assert!(ctx.refund(market, c1.pubkey(), c1_ata).is_ok());
    assert!(ctx.refund(market, c2.pubkey(), c2_ata).is_ok());

    assert_eq!(ctx.token_balance(creator_ata), 1_000_000_000);
    assert_eq!(ctx.token_balance(c1_ata), 1_000_000_000);
    assert_eq!(ctx.token_balance(c2_ata), 1_000_000_000);
    assert_eq!(ctx.token_balance(escrow), 0);

    // Each refund CLOSED its Contribution (reaped, rent → contributor); the counter
    // drained to 0.
    for who in [creator.pubkey(), c1.pubkey(), c2.pubkey()] {
        let (cpda, _) = kassandra_markets_sdk::pda::contribution(&market, &who);
        assert_eq!(ctx.lamports(cpda), 0, "Contribution closed by refund");
    }
    assert_eq!(
        ctx.read_pod::<Market>(market).open_contributions,
        0,
        "counter drained"
    );

    // close_market reclaims the escrow + Market rent to the creator (never-activated
    // Cancelled path: no cyes/cno/lp_vault).
    let reclaimable = ctx.lamports(market) + ctx.lamports(escrow);
    let creator_before = ctx.lamports(creator.pubkey());
    assert!(ctx.close_market(oracle, creator.pubkey()).is_ok());
    assert_eq!(ctx.lamports(market), 0, "market reaped");
    assert_eq!(ctx.lamports(escrow), 0, "escrow reaped");
    assert_eq!(
        ctx.lamports(creator.pubkey()),
        creator_before + reclaimable,
        "escrow + market rent → creator"
    );
}

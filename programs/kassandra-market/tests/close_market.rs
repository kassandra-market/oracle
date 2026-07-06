//! Integration tests for `close_market` (Ix 10): permissionless rent reclaim for
//! a fully-settled market. SPL-closes the Market-PDA-owned token accounts (escrow
//! always; cyes/cno/lp_vault iff activated) and the `Market` PDA, routing all rent
//! to the creator.

mod common;
use common::*;
use kassandra_market_program::error::MarketError;
use kassandra_market_program::state::{AccountType, Market, MarketStatus};
use solana_sdk::{
    account::Account,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

const PROPOSAL: u8 = 1; // kassandra Phase::Proposal (non-terminal)
const RESOLVED: u8 = 7; // kassandra Phase::Resolved (terminal)
const MIN_LIQ: u64 = 1_000_000_000; // 1 KASS (9 dp)
const SEED_A: u64 = 600_000_000; // creator's stake
const SEED_B: u64 = 400_000_000; // second contributor's stake (A + B == MIN_LIQ)

/// Fabricate a program-owned Market at its canonical PDA with the given fields set,
/// for exercising `close_market`'s guard order without the full lifecycle.
#[allow(clippy::too_many_arguments)]
fn seed_market(
    ctx: &mut TestCtx,
    oracle: Pubkey,
    creator: Pubkey,
    status: u8,
    fee_collected: u8,
    open_contributions: u16,
    lp_vault: Pubkey,
) -> Pubkey {
    use bytemuck::Zeroable;
    let (market, bump) = kassandra_market_sdk::pda::market(&oracle, 0);
    let (escrow, _) = kassandra_market_sdk::pda::escrow(&market);
    let mut m = Market::zeroed();
    m.account_type = AccountType::Market.as_u8();
    m.oracle = oracle.to_bytes().into();

    m.creator = creator.to_bytes().into();

    m.escrow_vault = escrow.to_bytes().into();

    m.status = status;
    m.fee_collected = fee_collected;
    m.open_contributions = open_contributions;
    m.lp_vault = lp_vault.to_bytes().into();

    m.bump = bump;
    let data = bytemuck::bytes_of(&m).to_vec();
    let lamports = ctx.svm.minimum_balance_for_rent_exemption(data.len());
    ctx.svm
        .set_account(
            market,
            Account {
                lamports,
                data,
                owner: ctx.program_id,
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();
    market
}

/// Stand up a fully-settled ACTIVATED market with two contributors, both of whom
/// have already claimed their LP (so `open_contributions == 0`, lp_vault swept to
/// 0). Fee-free config so `resolve_market` stamps `fee_collected` directly. Returns
/// the context + the addresses `close_market` will reap.
struct Activated {
    ctx: TestCtx,
    oracle: Pubkey,
    market: Pubkey,
    creator: Keypair,
}

fn setup_settled_activated_all_claimed() -> Activated {
    let mut ctx = TestCtx::new();
    ctx.load_metadao();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    let fee_dest = ctx.create_token_account(kass, authority.pubkey(), 0);
    let (_cfg, res) = ctx.init_config_full(authority.pubkey(), kass, MIN_LIQ, 0, fee_dest);
    assert!(res.is_ok(), "{res:?}");

    let oracle = ctx.seed_kass_oracle(2, PROPOSAL);
    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 5_000_000_000);
    let (market, res) = ctx.create_market(&creator, oracle, kass, creator_ata, SEED_A);
    assert!(res.is_ok(), "{res:?}");

    let c2 = Keypair::new();
    ctx.svm_airdrop(&c2.pubkey());
    let c2_ata = ctx.create_token_account(kass, c2.pubkey(), 5_000_000_000);
    let res = ctx.contribute(&c2, market, c2_ata, SEED_B);
    assert!(res.is_ok(), "{res:?}");

    let refs = ctx.compose_metadao_market(market, oracle, kass);
    let res = ctx.activate(oracle, kass);
    assert!(res.is_ok(), "activate: {res:?}");
    ctx.set_oracle_resolved(oracle, 0);
    let res = ctx.resolve_market(market, oracle, refs.question);
    assert!(res.is_ok(), "resolve: {res:?}");

    // Both contributors claim → open_contributions == 0, lp_vault swept to 0.
    let a_lp = ctx.create_token_account(refs.lp_mint, creator.pubkey(), 0);
    assert!(ctx.claim_lp(market, creator.pubkey(), a_lp).is_ok());
    let b_lp = ctx.create_token_account(refs.lp_mint, c2.pubkey(), 0);
    assert!(ctx.claim_lp(market, c2.pubkey(), b_lp).is_ok());

    let m: Market = ctx.read_pod(market);
    assert_eq!(m.open_contributions, 0, "all claimed");
    assert_eq!(m.status, MarketStatus::Resolved.as_u8());
    assert_eq!(m.fee_collected, 1);

    Activated {
        ctx,
        oracle,
        market,
        creator,
    }
}

#[test]
fn close_market_tolerates_dust_donation_into_escrow() {
    // F3 regression: a griefer who transfers 1 token unit into the DERIVABLE escrow
    // PDA after settlement must NOT be able to permanently brick `close_market`.
    // SPL `CloseAccount` refuses a funded account, so an unconditional close would
    // revert forever (nothing can drain escrow post-settle) and strand ALL of the
    // creator's rent. The fix skips a non-empty account: the Market PDA (and the
    // other 0-balance accounts) still close; only the dusted escrow is left behind.
    let mut a = setup_settled_activated_all_claimed();
    let market = a.market;
    let (escrow, _) = kassandra_market_sdk::pda::escrow(&market);
    let (cyes, _) = kassandra_market_sdk::pda::market_cyes(&market);
    let (cno, _) = kassandra_market_sdk::pda::market_cno(&market);
    let (lp_vault, _) = kassandra_market_sdk::pda::lp_vault(&market);

    // The griefer's dust donation into escrow.
    a.ctx.set_token_amount(escrow, 1);
    assert_eq!(a.ctx.token_balance(escrow), 1, "escrow dusted");

    // Close must still succeed (the headline: NOT permanently bricked).
    let res = a.ctx.close_market(a.oracle, a.creator.pubkey());
    assert!(
        res.is_ok(),
        "close_market must tolerate escrow dust: {res:?}"
    );

    // The Market PDA is reaped (no ghost) and every EMPTY account is closed; only
    // the dusted escrow is left in place (its rent stranded — the griefer's cost).
    for reaped in [market, cyes, cno, lp_vault] {
        assert_eq!(a.ctx.lamports(reaped), 0, "empty account reaped");
    }
    assert_eq!(
        a.ctx.token_balance(escrow),
        1,
        "dusted escrow left in place"
    );
}

#[test]
fn close_market_activated_happy_reclaims_all_rent() {
    let mut a = setup_settled_activated_all_claimed();
    let market = a.market;
    let (escrow, _) = kassandra_market_sdk::pda::escrow(&market);
    let (cyes, _) = kassandra_market_sdk::pda::market_cyes(&market);
    let (cno, _) = kassandra_market_sdk::pda::market_cno(&market);
    let (lp_vault, _) = kassandra_market_sdk::pda::lp_vault(&market);

    // All four token accounts are 0-balance (activate/collect drained cyes/cno &
    // escrow; the last claimer swept lp_vault).
    for ta in [escrow, cyes, cno, lp_vault] {
        assert_eq!(a.ctx.token_balance(ta), 0, "token account empty pre-close");
    }
    let reclaimable = a.ctx.lamports(market)
        + a.ctx.lamports(escrow)
        + a.ctx.lamports(cyes)
        + a.ctx.lamports(cno)
        + a.ctx.lamports(lp_vault);
    assert!(reclaimable > 0);
    let creator_before = a.ctx.lamports(a.creator.pubkey());

    let res = a.ctx.close_market(a.oracle, a.creator.pubkey());
    assert!(res.is_ok(), "close_market: {res:?}");

    // Market + all four token accounts reaped; every lamport → creator.
    for gone in [market, escrow, cyes, cno, lp_vault] {
        assert_eq!(a.ctx.lamports(gone), 0, "account reaped");
    }
    assert_eq!(
        a.ctx.lamports(a.creator.pubkey()),
        creator_before + reclaimable,
        "all reclaimed rent → creator"
    );
}

#[test]
fn close_market_rejects_contributions_open() {
    let mut a = setup_settled_activated_all_claimed();
    // Re-open the counter to simulate a contributor who hasn't claimed: close must
    // refuse so their still-stranded LP/refund is not destroyed.
    let mut m: Market = a.ctx.read_pod(a.market);
    m.open_contributions = 1;
    let data = bytemuck::bytes_of(&m).to_vec();
    let lamports = a.ctx.lamports(a.market);
    let owner = a.ctx.program_id;
    a.ctx
        .svm
        .set_account(
            a.market,
            Account {
                lamports,
                data,
                owner,
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();

    let res = a.ctx.close_market(a.oracle, a.creator.pubkey());
    assert_eq!(
        custom_code(&res),
        Some(MarketError::ContributionsOpen as u32)
    );
    // Market untouched (not reaped).
    assert!(a.ctx.lamports(a.market) > 0, "market still live");
}

#[test]
fn close_market_rejects_before_fee_collected() {
    // Fabricated activated market (lp_vault set), Resolved, fee_collected == 0.
    let mut ctx = TestCtx::new();
    let creator = Keypair::new();
    let oracle = Pubkey::new_unique();
    let lp_vault = Pubkey::new_unique(); // non-default → "activated"
    let market = seed_market(
        &mut ctx,
        oracle,
        creator.pubkey(),
        MarketStatus::Resolved.as_u8(),
        0, // fee NOT collected
        0,
        lp_vault,
    );
    let res = ctx.close_market(oracle, creator.pubkey());
    assert_eq!(custom_code(&res), Some(MarketError::FeeNotCollected as u32));
    assert!(ctx.lamports(market) > 0, "market untouched");
}

#[test]
fn close_market_rejects_non_terminal() {
    // A still-Active market is not settled — close must reject.
    let mut ctx = TestCtx::new();
    let creator = Keypair::new();
    let oracle = Pubkey::new_unique();
    let market = seed_market(
        &mut ctx,
        oracle,
        creator.pubkey(),
        MarketStatus::Active.as_u8(),
        1,
        0,
        Pubkey::new_unique(),
    );
    let res = ctx.close_market(oracle, creator.pubkey());
    assert_eq!(custom_code(&res), Some(MarketError::NotSettled as u32));
    assert!(ctx.lamports(market) > 0, "market untouched");
}

#[test]
fn close_market_rejects_wrong_creator() {
    // The creator is the rent recipient; a stranger cannot redirect the rent.
    let mut ctx = TestCtx::new();
    let creator = Keypair::new();
    let oracle = Pubkey::new_unique();
    let _market = seed_market(
        &mut ctx,
        oracle,
        creator.pubkey(),
        MarketStatus::Cancelled.as_u8(),
        0,
        0,
        Pubkey::default(),
    );
    let stranger = Keypair::new();
    let res = ctx.close_market(oracle, stranger.pubkey());
    assert_eq!(custom_code(&res), Some(MarketError::InvalidAccount as u32));
}

#[test]
fn close_market_cancelled_closes_escrow_and_market_only() {
    // A never-activated (Cancelled) market: only escrow + Market are closed (no
    // cyes/cno/lp_vault ever existed). Drive the real Cancelled path so escrow is a
    // genuine 0-balance token account.
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    let (_cfg, res) = ctx.init_config(authority.pubkey(), kass, MIN_LIQ);
    assert!(res.is_ok(), "{res:?}");
    let oracle = ctx.seed_kass_oracle(2, PROPOSAL);
    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 500_000_000);
    let (market, res) = ctx.create_market(&creator, oracle, kass, creator_ata, 200_000_000);
    assert!(res.is_ok(), "{res:?}");
    let (escrow, _) = kassandra_market_sdk::pda::escrow(&market);

    // Cancel (oracle terminal), then refund the sole contributor → escrow drained,
    // Contribution closed, open_contributions == 0.
    ctx.set_oracle_phase(oracle, RESOLVED);
    assert!(ctx.cancel(market, oracle).is_ok());
    assert!(ctx.refund(market, creator.pubkey(), creator_ata).is_ok());
    let m: Market = ctx.read_pod(market);
    assert_eq!(m.open_contributions, 0, "sole contributor refunded");
    assert_eq!(m.status, MarketStatus::Cancelled.as_u8());
    assert_eq!(
        m.lp_vault.to_bytes(),
        Pubkey::default().to_bytes(),
        "never activated"
    );
    assert_eq!(ctx.token_balance(escrow), 0, "escrow drained");

    let reclaimable = ctx.lamports(market) + ctx.lamports(escrow);
    let creator_before = ctx.lamports(creator.pubkey());
    let res = ctx.close_market(oracle, creator.pubkey());
    assert!(res.is_ok(), "close_market cancelled: {res:?}");
    assert_eq!(ctx.lamports(market), 0, "market reaped");
    assert_eq!(ctx.lamports(escrow), 0, "escrow reaped");
    assert_eq!(
        ctx.lamports(creator.pubkey()),
        creator_before + reclaimable,
        "escrow + market rent → creator"
    );

    // Double close: the Market is gone, so a second call fails to load it.
    let res = ctx.close_market(oracle, creator.pubkey());
    assert_eq!(custom_code(&res), Some(MarketError::InvalidAccount as u32));
}

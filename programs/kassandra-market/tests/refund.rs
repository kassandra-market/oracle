//! Integration tests for `refund` (permissionless per-contributor refund of
//! staked KASS out of a `Cancelled` market's escrow, program-signed by the
//! market PDA).

mod common;
use common::*;
use kassandra_market_program::error::MarketError;
use kassandra_market_program::state::Contribution;
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

const PROPOSAL: u8 = 1; // kassandra Phase::Proposal
const RESOLVED: u8 = 7; // kassandra Phase::Resolved

const MIN_LIQUIDITY: u64 = 1_000_000_000;

/// Stand up an under-funded `Funding` market with two contributors:
///
/// - creator: ata funded 500M, seeds 200M (drained to 300M),
/// - c2:      ata funded 400M, contributes 300M (drained to 100M).
///
/// Total contributed = 500M < 1B min, so the market is genuinely under-funded.
/// Returns everything a refund test needs.
struct Setup {
    ctx: TestCtx,
    oracle: Pubkey,
    market: Pubkey,
    kass: Pubkey,
    creator: Keypair,
    creator_ata: Pubkey,
    c2: Keypair,
    c2_ata: Pubkey,
}

fn setup_two_contributors() -> Setup {
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    let (_config, res) = ctx.init_config(authority.pubkey(), kass, MIN_LIQUIDITY);
    assert!(res.is_ok(), "{res:?}");

    let oracle = ctx.seed_kass_oracle(2, PROPOSAL);

    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 500_000_000);
    let (market, res) = ctx.create_market(&creator, oracle, kass, creator_ata, 200_000_000);
    assert!(res.is_ok(), "{res:?}");
    assert_eq!(ctx.token_balance(creator_ata), 300_000_000);

    let c2 = Keypair::new();
    ctx.svm_airdrop(&c2.pubkey());
    let c2_ata = ctx.create_token_account(kass, c2.pubkey(), 400_000_000);
    let res = ctx.contribute(&c2, market, c2_ata, 300_000_000);
    assert!(res.is_ok(), "{res:?}");
    assert_eq!(ctx.token_balance(c2_ata), 100_000_000);

    Setup {
        ctx,
        oracle,
        market,
        kass,
        creator,
        creator_ata,
        c2,
        c2_ata,
    }
}

#[test]
fn refund_happy_two_contributors_made_whole() {
    let mut s = setup_two_contributors();
    let (escrow, _) = kassandra_market_sdk::pda::escrow(&s.market);
    assert_eq!(s.ctx.token_balance(escrow), 500_000_000);

    s.ctx.set_oracle_phase(s.oracle, RESOLVED);
    let res = s.ctx.cancel(s.market, s.oracle);
    assert!(res.is_ok(), "{res:?}");

    // Refund the creator — its Contribution rent goes back to the creator's wallet.
    let (creator_contrib, _) =
        kassandra_market_sdk::pda::contribution(&s.market, &s.creator.pubkey());
    let creator_contrib_rent = s.ctx.lamports(creator_contrib);
    let creator_wallet_before = s.ctx.lamports(s.creator.pubkey());
    let res = s.ctx.refund(s.market, s.creator.pubkey(), s.creator_ata);
    assert!(res.is_ok(), "{res:?}");
    assert_eq!(s.ctx.token_balance(s.creator_ata), 500_000_000);
    assert_eq!(
        s.ctx.lamports(creator_contrib),
        0,
        "creator Contribution closed"
    );
    assert_eq!(
        s.ctx.lamports(s.creator.pubkey()),
        creator_wallet_before + creator_contrib_rent,
        "Contribution rent returned to contributor"
    );
    assert_eq!(
        s.ctx
            .read_pod::<kassandra_market_program::state::Market>(s.market)
            .open_contributions,
        1,
        "counter decremented after first refund"
    );

    // Refund the second contributor.
    let res = s.ctx.refund(s.market, s.c2.pubkey(), s.c2_ata);
    assert!(res.is_ok(), "{res:?}");
    assert_eq!(s.ctx.token_balance(s.c2_ata), 400_000_000);

    // Escrow fully drained.
    assert_eq!(s.ctx.token_balance(escrow), 0);

    // Both contributions CLOSED (reaped); counter drained to 0.
    let (c2_contrib, _) = kassandra_market_sdk::pda::contribution(&s.market, &s.c2.pubkey());
    assert_eq!(
        s.ctx.lamports(creator_contrib),
        0,
        "creator Contribution closed"
    );
    assert_eq!(s.ctx.lamports(c2_contrib), 0, "c2 Contribution closed");
    assert_eq!(
        s.ctx
            .read_pod::<kassandra_market_program::state::Market>(s.market)
            .open_contributions,
        0,
        "counter drained to 0"
    );
}

#[test]
fn refund_rejects_second_refund_contribution_closed() {
    let mut s = setup_two_contributors();
    s.ctx.set_oracle_phase(s.oracle, RESOLVED);
    let res = s.ctx.cancel(s.market, s.oracle);
    assert!(res.is_ok(), "{res:?}");

    let res = s.ctx.refund(s.market, s.creator.pubkey(), s.creator_ata);
    assert!(res.is_ok(), "{res:?}");

    // The refund CLOSED the Contribution, so a second refund can't load it — the
    // account's absence is the idempotency (fails the load guard → InvalidAccount).
    let (creator_contrib, _) =
        kassandra_market_sdk::pda::contribution(&s.market, &s.creator.pubkey());
    assert_eq!(
        s.ctx.lamports(creator_contrib),
        0,
        "Contribution closed by first refund"
    );
    let res = s.ctx.refund(s.market, s.creator.pubkey(), s.creator_ata);
    assert_eq!(custom_code(&res), Some(MarketError::InvalidAccount as u32));
}

#[test]
fn refund_rejects_not_cancelled() {
    // Fresh, still-`Funding` market: refund must be rejected.
    let mut s = setup_two_contributors();
    let res = s.ctx.refund(s.market, s.creator.pubkey(), s.creator_ata);
    assert_eq!(custom_code(&res), Some(MarketError::NotCancelled as u32));
}

#[test]
fn refund_rejects_cross_market_contribution() {
    // Market A (from the shared setup) plus an independent market B in the same
    // context. Refunding against A with B's `Contribution` PDA must be rejected
    // by the `contribution.market != market` guard.
    let mut s = setup_two_contributors();

    let oracle_b = s.ctx.seed_kass_oracle(2, PROPOSAL);
    let creator_b = Keypair::new();
    s.ctx.svm_airdrop(&creator_b.pubkey());
    let creator_b_ata = s
        .ctx
        .create_token_account(s.kass, creator_b.pubkey(), 500_000_000);
    let (market_b, res) =
        s.ctx
            .create_market(&creator_b, oracle_b, s.kass, creator_b_ata, 200_000_000);
    assert!(res.is_ok(), "{res:?}");

    // Cancel market A so the status guard passes and we reach the market check.
    s.ctx.set_oracle_phase(s.oracle, RESOLVED);
    let res = s.ctx.cancel(s.market, s.oracle);
    assert!(res.is_ok(), "{res:?}");

    // market A account paired with market B's contribution PDA → InvalidAccount.
    let (contribution_b, _) =
        kassandra_market_sdk::pda::contribution(&market_b, &creator_b.pubkey());
    let res = s
        .ctx
        .refund_with_contribution(s.market, contribution_b, creator_b_ata);
    assert_eq!(custom_code(&res), Some(MarketError::InvalidAccount as u32));
}

#[test]
fn refund_rejects_wrong_destination_owner() {
    let mut s = setup_two_contributors();
    s.ctx.set_oracle_phase(s.oracle, RESOLVED);
    let res = s.ctx.cancel(s.market, s.oracle);
    assert!(res.is_ok(), "{res:?}");

    // A cranker tries to redirect the creator's refund to a token account owned
    // by a stranger. The destination's SPL owner != contribution.contributor.
    let stranger = Keypair::new();
    let stranger_ata = s.ctx.create_token_account(s.kass, stranger.pubkey(), 0);
    let res = s.ctx.refund_to(s.market, s.creator.pubkey(), stranger_ata);
    assert_eq!(custom_code(&res), Some(MarketError::InvalidAccount as u32));

    // And the creator's stake is untouched.
    let (creator_contrib, _) =
        kassandra_market_sdk::pda::contribution(&s.market, &s.creator.pubkey());
    let cc: Contribution = s.ctx.read_pod(creator_contrib);
    assert_eq!(cc.claimed, 0);
}

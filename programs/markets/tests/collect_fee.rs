//! Integration tests for `collect_fee` (Ix 9): the permissionless crank that cuts
//! the protocol `fee_bps` share of a resolved market's **accrued** LP earnings and
//! routes it (as KASS) to `Config.fee_destination`, via program-signed
//! `amm::remove_liquidity` → `conditional_vault::redeem_tokens` → SPL `transfer`.
//!
//! Drives the REAL deployed MetaDAO v0.4 `conditional_vault` + `amm` binaries in
//! LiteSVM (via `ctx.load_metadao()`) through the full lifecycle, and grows the
//! pool with a REAL `amm::swap` so the accrued value (and therefore the fee) is
//! genuine — computed from the on-chain reserves, not fabricated.
//!
//! The `#[test]`s are split across sibling modules (`basic`, `guards`); this file
//! keeps the shared constants, `Active` handle, and setup/analysis helpers.

mod common;
use common::*;
use kassandra_markets_program::error::MarketError;
use kassandra_markets_program::state::{Market, MarketStatus};
use kassandra_markets_sdk::metadao::SwapType;
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

#[path = "collect_fee/basic.rs"]
mod basic;
#[path = "collect_fee/guards.rs"]
mod guards;

pub(crate) const PROPOSAL: u8 = 1; // kassandra Phase::Proposal (non-terminal)
pub(crate) const INVALID_DEADEND: u8 = 8; // kassandra Phase::InvalidDeadend (terminal void)
pub(crate) const MIN_LIQ: u64 = 1_000_000_000; // 1 KASS (9 dp) — the seeded pool depth
pub(crate) const SWAP_KASS: u64 = 3_000_000_000; // KASS the swapper splits for a trading position
pub(crate) const SWAP_IN: u64 = 1_500_000_000; // cYES sold into the pool to move price + accrue fees

/// Account byte offsets (absolute, incl. any 8-byte Anchor disc).
pub(crate) const Q_NUM0: usize = 76;
pub(crate) const Q_NUM1: usize = 80;
pub(crate) const Q_DENOM: usize = 84;
pub(crate) const AMM_BASE: usize = 115;
pub(crate) const AMM_QUOTE: usize = 123;
pub(crate) const MINT_SUPPLY: usize = 36;

pub(crate) fn read_u32_at(ctx: &TestCtx, key: Pubkey, off: usize) -> u128 {
    let acc = ctx.svm.get_account(&key).expect("account exists");
    u32::from_le_bytes(acc.data[off..off + 4].try_into().unwrap()) as u128
}
pub(crate) fn read_u64_at(ctx: &TestCtx, key: Pubkey, off: usize) -> u128 {
    let acc = ctx.svm.get_account(&key).expect("account exists");
    u64::from_le_bytes(acc.data[off..off + 8].try_into().unwrap()) as u128
}

/// Everything a collect_fee test needs after fund → activate.
pub(crate) struct Active {
    pub(crate) ctx: TestCtx,
    pub(crate) kass: Pubkey,
    pub(crate) market: Pubkey,
    pub(crate) oracle: Pubkey,
    pub(crate) refs: MetaDaoRefs,
    pub(crate) fee_dest: Pubkey,
    pub(crate) creator: Keypair,
}

/// Fund a single-contributor market to `MIN_LIQ`, compose the MetaDAO market, and
/// `activate`, with an explicit `fee_bps`. Returns the live `Active` handle.
pub(crate) fn setup_active(fee_bps: u16) -> Active {
    let mut ctx = TestCtx::new();
    ctx.load_metadao();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    let fee_dest = ctx.create_token_account(kass, authority.pubkey(), 0);
    let (_cfg, res) = ctx.init_config_full(authority.pubkey(), kass, MIN_LIQ, fee_bps, fee_dest);
    assert!(res.is_ok(), "init_config: {res:?}");

    let oracle = ctx.seed_kass_oracle(2, PROPOSAL);
    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 10_000_000_000);
    let (market, res) = ctx.create_market(&creator, oracle, kass, creator_ata, MIN_LIQ);
    assert!(res.is_ok(), "create_market: {res:?}");

    let refs = ctx.compose_metadao_market(market, oracle, kass);
    let res = ctx.activate(oracle, kass);
    assert!(res.is_ok(), "activate: {res:?}");
    assert_eq!(
        ctx.read_pod::<Market>(market).fee_bps,
        fee_bps,
        "fee_bps snapshot"
    );

    Active {
        ctx,
        kass,
        market,
        oracle,
        refs,
        fee_dest,
        creator,
    }
}

/// Grow the pool with a REAL swap: a fresh user splits `SWAP_KASS` KASS into
/// cYES+cNO, then SELLS `SWAP_IN` cYES into the pool (cYES in, cNO out). The swap
/// fee accrues to the reserves and the pool ends up cYES-heavy, so a subsequent
/// YES resolution realizes genuine LP earnings.
pub(crate) fn grow_pool_sell_cyes(a: &mut Active) {
    let user = Keypair::new();
    a.ctx.svm_airdrop(&user.pubkey());
    let u_kass = a.ctx.create_token_account(a.kass, user.pubkey(), SWAP_KASS);
    let u_cyes = a
        .ctx
        .create_token_account(a.refs.yes_mint, user.pubkey(), 0);
    let u_cno = a.ctx.create_token_account(a.refs.no_mint, user.pubkey(), 0);
    let res = a
        .ctx
        .user_split(&user, &a.refs, u_kass, u_cyes, u_cno, SWAP_KASS);
    assert!(res.is_ok(), "swapper split: {res:?}");

    let res = a
        .ctx
        .user_swap(&user, &a.refs, u_cyes, u_cno, SwapType::Sell, SWAP_IN, 0);
    assert!(res.is_ok(), "swap (sell cYES): {res:?}");
}

/// Analytically recompute the on-chain fee math from the resolved `Question` +
/// `Amm` reserves + LP-mint supply. Returns `(fee_lp, expected_fee_kass)` where
/// `expected_fee_kass` is the double-floored KASS the crank should deliver.
pub(crate) fn expected_fee(a: &Active, fee_bps: u128) -> (u64, u128) {
    let num0 = read_u32_at(&a.ctx, a.refs.question, Q_NUM0);
    let num1 = read_u32_at(&a.ctx, a.refs.question, Q_NUM1);
    let denom = read_u32_at(&a.ctx, a.refs.question, Q_DENOM);
    assert!(denom > 0, "question resolved");
    let base = read_u64_at(&a.ctx, a.refs.amm, AMM_BASE);
    let quote = read_u64_at(&a.ctx, a.refs.amm, AMM_QUOTE);
    let supply = read_u64_at(&a.ctx, a.refs.lp_mint, MINT_SUPPLY);
    assert!(supply > 0, "lp supply > 0");

    let m: Market = a.ctx.read_pod(a.market);
    let lp_total = m.lp_total as u128;
    let total_contributed = m.total_contributed as u128;

    let pool_value = (base * num0 + quote * num1) / denom;
    let realized_full = lp_total * pool_value / supply;
    let accrued = realized_full.saturating_sub(total_contributed);
    let fee_lp = if accrued == 0 {
        0
    } else {
        let accrued_lp = lp_total * accrued / realized_full;
        accrued_lp * fee_bps / 10_000
    };
    // Double-floor delivered KASS: remove_liquidity floors each reserve share,
    // redeem_tokens floors the payout.
    let base_out = base * fee_lp / supply;
    let quote_out = quote * fee_lp / supply;
    let expected_kass = (base_out * num0 + quote_out * num1) / denom;
    (u64::try_from(fee_lp).unwrap(), expected_kass)
}

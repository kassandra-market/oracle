//! Integration tests for `claim_lp` (Ix 7): permissionless per-contributor
//! pro-rata claim of the AMM LP tokens seeded at `activate`, program-signed out
//! of the Market-PDA-owned `lp_vault`.
//!
//! Drives the REAL deployed MetaDAO v0.4 binaries in LiteSVM (via
//! `ctx.load_metadao()`) through the full activate flow before claiming.
//!
//! The `#[test]`s are split across sibling modules (`happy`, `guards`); this file
//! keeps the shared constants, `Setup` handle, and fabrication helpers.

mod common;
use common::*;
use kassandra_markets_program::error::MarketError;
use kassandra_markets_program::state::{AccountType, Contribution, Market, MarketStatus};
use solana_sdk::{
    account::Account,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

#[path = "claim_lp/happy.rs"]
mod happy;
#[path = "claim_lp/guards.rs"]
mod guards;

/// Fabricate a program-owned account holding the `Pod` value `v` at `key`.
pub(crate) fn set_pod<T: bytemuck::Pod>(ctx: &mut TestCtx, key: Pubkey, v: &T) {
    let data = bytemuck::bytes_of(v).to_vec();
    let lamports = ctx.svm.minimum_balance_for_rent_exemption(data.len());
    let owner = ctx.program_id;
    ctx.svm
        .set_account(
            key,
            Account {
                lamports,
                data,
                owner,
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();
}

/// Place an initialized SPL token account (mint/owner/amount) at a SPECIFIC
/// address — unlike `create_token_account`, which fabricates at a random key.
pub(crate) fn set_token_at(ctx: &mut TestCtx, addr: Pubkey, mint: Pubkey, owner: Pubkey, amount: u64) {
    use spl_token::solana_program::{program_option::COption, program_pack::Pack};
    use spl_token::state::{Account as TokenAccount, AccountState};
    let state = TokenAccount {
        mint,
        owner,
        amount,
        delegate: COption::None,
        state: AccountState::Initialized,
        is_native: COption::None,
        delegated_amount: 0,
        close_authority: COption::None,
    };
    let mut data = vec![0u8; TokenAccount::LEN];
    state.pack_into_slice(&mut data);
    let lamports = ctx
        .svm
        .minimum_balance_for_rent_exemption(TokenAccount::LEN);
    ctx.svm
        .set_account(
            addr,
            Account {
                lamports,
                data,
                owner: spl_token::ID,
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();
}

pub(crate) const PROPOSAL: u8 = 1; // kassandra Phase::Proposal (non-terminal)
pub(crate) const MIN_LIQ: u64 = 1_000_000_000; // 1 KASS (9 dp)
pub(crate) const SEED_A: u64 = 600_000_000; // creator's stake
pub(crate) const SEED_B: u64 = 400_000_000; // second contributor's stake

/// Floor pro-rata share, mirroring the on-chain u128-intermediate helper.
pub(crate) fn expected_share(lp_total: u64, amount: u64, total: u64) -> u64 {
    u64::try_from((lp_total as u128) * (amount as u128) / (total as u128)).unwrap()
}

pub(crate) struct Setup {
    pub(crate) ctx: TestCtx,
    pub(crate) oracle: Pubkey,
    pub(crate) market: Pubkey,
    pub(crate) kass: Pubkey,
    pub(crate) creator: Keypair,
    pub(crate) c2: Keypair,
    pub(crate) lp_mint: Pubkey,
    pub(crate) lp_total: u64,
    pub(crate) total_contributed: u64,
}

/// Stand up a fully-funded market with two contributors (creator seeds A, c2
/// adds B, A+B == MIN_LIQ), compose the MetaDAO market, `activate` it, then
/// resolve it YES. `claim_lp` gates on `market.fee_collected == 1`; this setup
/// uses a **fee-free** config (`fee_bps == 0`) so `resolve_market` stamps the flag
/// directly (no `collect_fee` crank needed), keeping these tests focused on the
/// claim mechanics. The dedicated `collect_fee.rs` suite covers the `fee_bps > 0`
/// collect-then-claim flow. Returns everything needed to exercise `claim_lp`.
pub(crate) fn setup_active_two_contributors() -> Setup {
    let mut ctx = TestCtx::new();
    ctx.load_metadao();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    // fee_bps == 0 → resolve_market sets fee_collected without a collect_fee crank.
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

    // Resolve YES so the fee-free market stamps `fee_collected` and `claim_lp` opens.
    ctx.set_oracle_resolved(oracle, 0);
    let res = ctx.resolve_market(market, oracle, refs.question);
    assert!(res.is_ok(), "resolve: {res:?}");

    let m: Market = ctx.read_pod(market);
    assert_eq!(m.total_contributed, MIN_LIQ, "total contributed");
    assert!(m.lp_total > 0, "lp_total > 0");
    assert_eq!(
        m.fee_collected, 1,
        "fee-free market: fee_collected stamped at resolve"
    );

    Setup {
        ctx,
        oracle,
        market,
        kass,
        creator,
        c2,
        lp_mint: refs.lp_mint,
        lp_total: m.lp_total,
        total_contributed: m.total_contributed,
    }
}

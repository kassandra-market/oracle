//! Integration tests for `activate` (Ix 6): verify a client-composed MetaDAO
//! market, program-signed split of the escrowed KASS into cYES/cNO, seed the AMM
//! pool 50/50, and record the bindings on the `Market` (status → Active).
//!
//! Drives the REAL deployed MetaDAO v0.4 `conditional_vault` + `amm` binaries in
//! LiteSVM (via `ctx.load_metadao()`).

mod common;
use common::*;
use kassandra_market_program::error::MarketError;
use kassandra_market_program::state::{Market, MarketStatus};
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

const PROPOSAL: u8 = 1; // kassandra Phase::Proposal (non-terminal)
const RESOLVED: u8 = 7; // kassandra Phase::Resolved (terminal)
const MIN_LIQ: u64 = 1_000_000_000; // 1 KASS (9 dp)

/// Stand up a fully-funded `Funding` market (creator seeds exactly `MIN_LIQ`) and
/// its live Kassandra oracle. Returns the context, KASS mint, market PDA, oracle.
fn setup_funded() -> (TestCtx, Pubkey, Pubkey, Pubkey) {
    let mut ctx = TestCtx::new();
    ctx.load_metadao();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    let (_cfg, res) = ctx.init_config(authority.pubkey(), kass, MIN_LIQ);
    assert!(res.is_ok(), "{res:?}");

    let oracle = ctx.seed_kass_oracle(2, PROPOSAL);
    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 5_000_000_000);
    let (market, res) = ctx.create_market(&creator, oracle, kass, creator_ata, MIN_LIQ);
    assert!(res.is_ok(), "{res:?}");
    (ctx, kass, market, oracle)
}

#[test]
fn activate_happy_path() {
    let (mut ctx, kass, market, oracle) = setup_funded();
    let refs = ctx.compose_metadao_market(market, oracle, kass);

    let escrow = Pubkey::new_from_array(ctx.read_pod::<Market>(market).escrow_vault.to_bytes());
    assert_eq!(ctx.token_balance(escrow), MIN_LIQ, "escrow pre-loaded");

    let res = ctx.activate(oracle, kass);
    assert!(res.is_ok(), "activate: {res:?}");

    let m: Market = ctx.read_pod(market);
    assert_eq!(m.status, MarketStatus::Active.as_u8(), "status → Active");

    // Escrow KASS drained into the conditional vault's underlying account.
    assert_eq!(ctx.token_balance(escrow), 0, "escrow drained");
    assert_eq!(
        ctx.token_balance(refs.vault_underlying_ata),
        MIN_LIQ,
        "underlying moved into the vault"
    );

    // Pool holds the balanced cYES/cNO reserves (50/50).
    assert_eq!(
        ctx.token_balance(refs.amm_vault_base),
        MIN_LIQ,
        "cYES reserve"
    );
    assert_eq!(
        ctx.token_balance(refs.amm_vault_quote),
        MIN_LIQ,
        "cNO reserve"
    );

    // The transient split holders are emptied into the pool.
    let (market_cyes, _) = kassandra_market_sdk::pda::market_cyes(&market);
    let (market_cno, _) = kassandra_market_sdk::pda::market_cno(&market);
    assert_eq!(ctx.token_balance(market_cyes), 0, "cYES holder drained");
    assert_eq!(ctx.token_balance(market_cno), 0, "cNO holder drained");

    // lp_vault holds lp_total > 0.
    let (lp_vault, _) = kassandra_market_sdk::pda::lp_vault(&market);
    assert_eq!(
        Pubkey::new_from_array(m.lp_vault.to_bytes()),
        lp_vault,
        "lp_vault recorded"
    );
    assert!(m.lp_total > 0, "lp_total > 0");
    assert_eq!(
        ctx.token_balance(lp_vault),
        m.lp_total,
        "lp_vault == lp_total"
    );

    // All bindings recorded on the Market.
    assert_eq!(m.question.to_bytes(), refs.question.to_bytes());
    assert_eq!(m.vault.to_bytes(), refs.vault.to_bytes());
    assert_eq!(m.yes_mint.to_bytes(), refs.yes_mint.to_bytes());
    assert_eq!(m.no_mint.to_bytes(), refs.no_mint.to_bytes());
    assert_eq!(m.amm.to_bytes(), refs.amm.to_bytes());
    assert_eq!(m.lp_mint.to_bytes(), refs.lp_mint.to_bytes());
}

#[test]
fn activate_rejects_underfunded() {
    // Fund BELOW min_liquidity: the NotFunded guard fires before any MetaDAO work.
    let mut ctx = TestCtx::new();
    ctx.load_metadao();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    let (_cfg, res) = ctx.init_config(authority.pubkey(), kass, MIN_LIQ);
    assert!(res.is_ok(), "{res:?}");

    let oracle = ctx.seed_kass_oracle(2, PROPOSAL);
    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 5_000_000_000);
    let (_market, res) = ctx.create_market(&creator, oracle, kass, creator_ata, MIN_LIQ / 2);
    assert!(res.is_ok(), "{res:?}");

    let res = ctx.activate(oracle, kass);
    assert_eq!(custom_code(&res), Some(MarketError::NotFunded as u32));
}

#[test]
fn activate_rejects_terminal_oracle() {
    let (mut ctx, kass, _market, oracle) = setup_funded();
    let _refs = ctx.compose_metadao_market(_market, oracle, kass);
    // Push the oracle to a terminal phase after composition.
    ctx.set_oracle_phase(oracle, RESOLVED);

    let res = ctx.activate(oracle, kass);
    assert_eq!(custom_code(&res), Some(MarketError::OracleResolved as u32));
}

#[test]
fn activate_rejects_double_activate() {
    let (mut ctx, kass, _market, oracle) = setup_funded();
    let _refs = ctx.compose_metadao_market(_market, oracle, kass);
    let res = ctx.activate(oracle, kass);
    assert!(res.is_ok(), "first activate: {res:?}");
    // Second activate: the market is now Active → NotFunding.
    let res = ctx.activate(oracle, kass);
    assert_eq!(custom_code(&res), Some(MarketError::NotFunding as u32));
}

#[test]
fn activate_rejects_nonempty_pool() {
    let (mut ctx, kass, market, oracle) = setup_funded();
    let refs = ctx.compose_metadao_market(market, oracle, kass);

    // Simulate a front-runner having seeded the pool between `create_amm` and
    // `activate`: give the Amm a nonzero base reserve (`base_amount` @115), which
    // is exactly the field the empty-pool guard reads. Owner stays the AMM program.
    let mut acc = ctx.svm.get_account(&refs.amm).expect("amm exists");
    acc.data[115..123].copy_from_slice(&1_000_000u64.to_le_bytes());
    ctx.svm.set_account(refs.amm, acc).unwrap();

    let res = ctx.activate(oracle, kass);
    assert_eq!(custom_code(&res), Some(MarketError::PoolNotEmpty as u32));
}

#[test]
fn activate_rejects_tampered_question_oracle() {
    let (mut ctx, kass, market, oracle) = setup_funded();
    let refs = ctx.compose_metadao_market(market, oracle, kass);

    // Tamper the composed Question's `oracle` field (@40) so it no longer points
    // at the Market PDA, keeping it owned by the conditional_vault program. The
    // address (a PDA) is unchanged, so `assert_key` passes but the field binding
    // check must reject it.
    let mut acc = ctx
        .svm
        .get_account(&refs.question)
        .expect("question exists");
    acc.data[40..72].copy_from_slice(Pubkey::new_unique().as_ref());
    ctx.svm.set_account(refs.question, acc).unwrap();

    let res = ctx.activate(oracle, kass);
    assert_eq!(custom_code(&res), Some(MarketError::InvalidAccount as u32));
}

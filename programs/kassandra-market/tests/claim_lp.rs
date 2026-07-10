//! Integration tests for `claim_lp` (Ix 7): permissionless per-contributor
//! pro-rata claim of the AMM LP tokens seeded at `activate`, program-signed out
//! of the Market-PDA-owned `lp_vault`.
//!
//! Drives the REAL deployed MetaDAO v0.4 binaries in LiteSVM (via
//! `ctx.load_metadao()`) through the full activate flow before claiming.

mod common;
use common::*;
use kassandra_market_program::error::MarketError;
use kassandra_market_program::state::{AccountType, Contribution, Market, MarketStatus};
use solana_sdk::{
    account::Account,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

/// Fabricate a program-owned account holding the `Pod` value `v` at `key`.
fn set_pod<T: bytemuck::Pod>(ctx: &mut TestCtx, key: Pubkey, v: &T) {
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
fn set_token_at(ctx: &mut TestCtx, addr: Pubkey, mint: Pubkey, owner: Pubkey, amount: u64) {
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

const PROPOSAL: u8 = 1; // kassandra Phase::Proposal (non-terminal)
const MIN_LIQ: u64 = 1_000_000_000; // 1 KASS (9 dp)
const SEED_A: u64 = 600_000_000; // creator's stake
const SEED_B: u64 = 400_000_000; // second contributor's stake

/// Floor pro-rata share, mirroring the on-chain u128-intermediate helper.
fn expected_share(lp_total: u64, amount: u64, total: u64) -> u64 {
    u64::try_from((lp_total as u128) * (amount as u128) / (total as u128)).unwrap()
}

struct Setup {
    ctx: TestCtx,
    oracle: Pubkey,
    market: Pubkey,
    kass: Pubkey,
    creator: Keypair,
    c2: Keypair,
    lp_mint: Pubkey,
    lp_total: u64,
    total_contributed: u64,
}

/// Stand up a fully-funded market with two contributors (creator seeds A, c2
/// adds B, A+B == MIN_LIQ), compose the MetaDAO market, `activate` it, then
/// resolve it YES. `claim_lp` gates on `market.fee_collected == 1`; this setup
/// uses a **fee-free** config (`fee_bps == 0`) so `resolve_market` stamps the flag
/// directly (no `collect_fee` crank needed), keeping these tests focused on the
/// claim mechanics. The dedicated `collect_fee.rs` suite covers the `fee_bps > 0`
/// collect-then-claim flow. Returns everything needed to exercise `claim_lp`.
fn setup_active_two_contributors() -> Setup {
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

#[test]
fn claim_lp_happy_two_contributors_pro_rata() {
    let mut s = setup_active_two_contributors();
    let (lp_vault, _) = kassandra_markets_sdk::pda::lp_vault(&s.market);
    assert_eq!(s.ctx.token_balance(lp_vault), s.lp_total, "lp_vault seeded");

    let a_share = expected_share(s.lp_total, SEED_A, s.total_contributed);
    assert!(a_share > 0, "A share nonzero");

    // Contributor A (creator) claims first — NOT the last claimer (counter 2 → 1),
    // so takes the floor pro-rata share. Their Contribution is then closed and its
    // rent returned to the creator.
    let a_contrib_rent = s
        .ctx
        .lamports(kassandra_markets_sdk::pda::contribution(&s.market, &s.creator.pubkey()).0);
    assert!(a_contrib_rent > 0, "A Contribution funded pre-claim");
    let a_wallet_before = s.ctx.lamports(s.creator.pubkey());
    let a_lp_ata = s.ctx.create_token_account(s.lp_mint, s.creator.pubkey(), 0);
    let res = s.ctx.claim_lp(s.market, s.creator.pubkey(), a_lp_ata);
    assert!(res.is_ok(), "{res:?}");
    assert_eq!(s.ctx.token_balance(a_lp_ata), a_share, "A pro-rata LP");

    // A's Contribution is CLOSED (reaped) and its rent went back to A's wallet.
    let (a_contrib, _) = kassandra_markets_sdk::pda::contribution(&s.market, &s.creator.pubkey());
    assert_eq!(s.ctx.lamports(a_contrib), 0, "A Contribution closed");
    assert_eq!(
        s.ctx.lamports(s.creator.pubkey()),
        a_wallet_before + a_contrib_rent,
        "A Contribution rent returned to contributor"
    );
    let m: Market = s.ctx.read_pod(s.market);
    assert_eq!(m.open_contributions, 1, "counter decremented after A");

    // vault still holds the un-swept remainder before B's (last) claim.
    let vault_before_b = s.ctx.token_balance(lp_vault);
    assert_eq!(
        vault_before_b,
        s.lp_total - a_share,
        "vault holds remainder"
    );

    // Contributor B claims LAST (counter == 1) → sweeps the ENTIRE remaining vault
    // (their pro-rata share PLUS A's floor dust), so lp_vault ends at exactly 0.
    let b_lp_ata = s.ctx.create_token_account(s.lp_mint, s.c2.pubkey(), 0);
    let res = s.ctx.claim_lp(s.market, s.c2.pubkey(), b_lp_ata);
    assert!(res.is_ok(), "{res:?}");
    assert_eq!(
        s.ctx.token_balance(b_lp_ata),
        vault_before_b,
        "B sweeps the remainder"
    );

    // lp_vault ends at EXACTLY 0 — no un-closeable dust left for close_market.
    assert_eq!(
        s.ctx.token_balance(lp_vault),
        0,
        "last claimer sweeps vault to 0"
    );

    // Both contributions closed; counter at 0.
    let (b_contrib, _) = kassandra_markets_sdk::pda::contribution(&s.market, &s.c2.pubkey());
    assert_eq!(s.ctx.lamports(a_contrib), 0, "A Contribution closed");
    assert_eq!(s.ctx.lamports(b_contrib), 0, "B Contribution closed");
    let m: Market = s.ctx.read_pod(s.market);
    assert_eq!(m.open_contributions, 0, "counter drained to 0");
}

#[test]
fn claim_lp_rejects_second_claim_contribution_closed() {
    let mut s = setup_active_two_contributors();
    let a_lp_ata = s.ctx.create_token_account(s.lp_mint, s.creator.pubkey(), 0);
    let res = s.ctx.claim_lp(s.market, s.creator.pubkey(), a_lp_ata);
    assert!(res.is_ok(), "{res:?}");

    // The claim CLOSED the Contribution, so a second claim can't even load it —
    // the absence of the account is the idempotency (fails the load guard).
    let (a_contrib, _) = kassandra_markets_sdk::pda::contribution(&s.market, &s.creator.pubkey());
    assert_eq!(
        s.ctx.lamports(a_contrib),
        0,
        "Contribution closed by first claim"
    );
    let res = s.ctx.claim_lp(s.market, s.creator.pubkey(), a_lp_ata);
    assert_eq!(custom_code(&res), Some(MarketError::InvalidAccount as u32));
}

#[test]
fn claim_lp_rejects_not_active() {
    // A still-`Funding` market (never activated) has no LP to claim.
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    let (_cfg, res) = ctx.init_config(authority.pubkey(), kass, MIN_LIQ);
    assert!(res.is_ok(), "{res:?}");
    let oracle = ctx.seed_kass_oracle(2, PROPOSAL);
    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 5_000_000_000);
    let (market, res) = ctx.create_market(&creator, oracle, kass, creator_ata, SEED_A);
    assert!(res.is_ok(), "{res:?}");

    // No lp_mint yet; any token account will do — the status guard fires first.
    let dest = ctx.create_token_account(kass, creator.pubkey(), 0);
    let res = ctx.claim_lp(market, creator.pubkey(), dest);
    assert_eq!(custom_code(&res), Some(MarketError::NotActive as u32));
}

#[test]
fn claim_lp_rejects_wrong_dest_owner() {
    let mut s = setup_active_two_contributors();
    // Dest LP ata owned by a stranger, not the recorded contributor.
    let stranger = Keypair::new();
    let stranger_lp_ata = s.ctx.create_token_account(s.lp_mint, stranger.pubkey(), 0);
    let res = s
        .ctx
        .claim_lp_to(s.market, s.creator.pubkey(), stranger_lp_ata);
    assert_eq!(custom_code(&res), Some(MarketError::InvalidAccount as u32));

    // Stake untouched.
    let (a_contrib, _) = kassandra_markets_sdk::pda::contribution(&s.market, &s.creator.pubkey());
    assert_eq!(s.ctx.read_pod::<Contribution>(a_contrib).claimed, 0);
}

#[test]
fn claim_lp_rejects_wrong_mint() {
    let mut s = setup_active_two_contributors();
    // Dest owned by the contributor but on the WRONG mint (KASS, not lp_mint).
    let wrong_mint_ata = s.ctx.create_token_account(s.kass, s.creator.pubkey(), 0);
    let res = s.ctx.claim_lp(s.market, s.creator.pubkey(), wrong_mint_ata);
    assert_eq!(custom_code(&res), Some(MarketError::InvalidAccount as u32));
}

#[test]
fn claim_lp_succeeds_after_resolution() {
    // FUND-LOSS REGRESSION: `resolve_market` is a permissionless crank that flips
    // status to Resolved the moment the oracle is terminal. claim_lp is the ONLY
    // path out of `lp_vault`, so it MUST stay open post-resolution (once the fee is
    // collected) or a contributor who hadn't claimed yet loses their LP forever.
    // The setup already resolved a fee-free market (fee_collected stamped).
    let mut s = setup_active_two_contributors();
    let m: Market = s.ctx.read_pod(s.market);
    assert_eq!(
        m.status,
        MarketStatus::Resolved.as_u8(),
        "status → Resolved"
    );
    assert_eq!(m.fee_collected, 1, "fee flag stamped");

    // A contributor who hadn't claimed before resolution still gets their LP.
    let a_share = expected_share(s.lp_total, SEED_A, s.total_contributed);
    assert!(a_share > 0);
    let a_lp_ata = s.ctx.create_token_account(s.lp_mint, s.creator.pubkey(), 0);
    let res = s.ctx.claim_lp(s.market, s.creator.pubkey(), a_lp_ata);
    assert!(res.is_ok(), "post-resolution claim: {res:?}");
    assert_eq!(
        s.ctx.token_balance(a_lp_ata),
        a_share,
        "still gets pro-rata LP"
    );
    // A claimed first (not the last claimer) → its Contribution is closed (reaped).
    let (a_contrib, _) = kassandra_markets_sdk::pda::contribution(&s.market, &s.creator.pubkey());
    assert_eq!(
        s.ctx.lamports(a_contrib),
        0,
        "Contribution closed after claim"
    );
}

#[test]
fn claim_lp_rejects_cross_market_contribution() {
    use bytemuck::Zeroable;
    let mut s = setup_active_two_contributors();

    // A well-formed Contribution whose `market` field points at a DIFFERENT
    // market. Passed explicitly, it must be rejected by the cross-market guard
    // (before any token movement).
    let foreign_addr = Pubkey::new_unique();
    let mut c = Contribution::zeroed();
    c.account_type = AccountType::Contribution.as_u8();
    c.market = Pubkey::new_unique().to_bytes().into(); // NOT s.market
    c.contributor = s.creator.pubkey().to_bytes().into();

    c.amount = SEED_A;
    set_pod(&mut s.ctx, foreign_addr, &c);

    let dest = s.ctx.create_token_account(s.lp_mint, s.creator.pubkey(), 0);
    let res = s
        .ctx
        .claim_lp_with_contribution(s.market, foreign_addr, dest);
    assert_eq!(custom_code(&res), Some(MarketError::InvalidAccount as u32));
}

#[test]
fn claim_lp_dust_share_zero_still_closes_contribution() {
    use bytemuck::Zeroable;
    // The floor pro-rata share can round to 0 for a dust contribution (when
    // `lp_total * amount < total_contributed`). The processor must skip the
    // transfer but STILL close the contribution, so a dust contributor cannot
    // wedge in a retry loop. `open_contributions == 2` keeps this on the floor
    // pro-rata branch (NOT the last-claimer sweep). In the real flow
    // `lp_total == total_contributed` so this never triggers; we fabricate an
    // inconsistent state to cover the branch.
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let lp_mint = ctx.create_mint(9);
    let oracle = Pubkey::new_unique();
    let (market, mbump) = kassandra_markets_sdk::pda::market(&oracle, 0);
    let contributor = Keypair::new();
    let (contribution, cbump) =
        kassandra_markets_sdk::pda::contribution(&market, &contributor.pubkey());

    // A market-PDA-owned LP token account holding a tiny LP total, placed at the
    // canonical `lp_vault` PDA (the harness `claim_lp` derives that address).
    let (lp_vault, _) = kassandra_markets_sdk::pda::lp_vault(&market);
    set_token_at(&mut ctx, lp_vault, lp_mint, market, 1);

    let mut m = Market::zeroed();
    m.account_type = AccountType::Market.as_u8();
    m.oracle = oracle.to_bytes().into();

    m.kass_mint = kass.to_bytes().into();

    m.status = MarketStatus::Active.as_u8();
    m.bump = mbump;
    m.lp_mint = lp_mint.to_bytes().into();

    m.lp_vault = lp_vault.to_bytes().into();

    m.lp_total = 1;
    m.total_contributed = 1_000_000; // floor(1 * 1 / 1_000_000) == 0
    m.fee_collected = 1; // fee gate satisfied (this fabricated market skips collect_fee)
    m.open_contributions = 2; // > 1 → floor pro-rata branch (not the last-claimer sweep)
    set_pod(&mut ctx, market, &m);

    let mut c = Contribution::zeroed();
    c.account_type = AccountType::Contribution.as_u8();
    c.market = market.to_bytes().into();

    c.contributor = contributor.pubkey().to_bytes().into();

    c.amount = 1;
    c.bump = cbump;
    set_pod(&mut ctx, contribution, &c);

    let contrib_rent = ctx.lamports(contribution);
    assert!(contrib_rent > 0, "Contribution funded pre-claim");
    let dest = ctx.create_token_account(lp_mint, contributor.pubkey(), 0);
    let res = ctx.claim_lp(market, contributor.pubkey(), dest);
    assert!(res.is_ok(), "dust claim ok: {res:?}");
    assert_eq!(
        ctx.token_balance(dest),
        0,
        "no LP transferred (share floored to 0)"
    );
    assert_eq!(ctx.token_balance(lp_vault), 1, "lp_vault untouched");
    // The Contribution is CLOSED (reaped) even though no LP moved — no retry wedge.
    assert_eq!(ctx.lamports(contribution), 0, "Contribution closed");
    assert_eq!(
        ctx.lamports(contributor.pubkey()),
        contrib_rent,
        "Contribution rent returned to contributor"
    );
    assert_eq!(
        ctx.read_pod::<Market>(market).open_contributions,
        1,
        "counter decremented (2 → 1)"
    );
}

#[test]
fn claim_lp_and_refund_mutually_exclusive() {
    // On a Resolved market, `refund` must fail (status != Cancelled) — proving a
    // contribution cannot be both LP-claimed and refunded.
    let mut s = setup_active_two_contributors();
    let a_lp_ata = s.ctx.create_token_account(s.lp_mint, s.creator.pubkey(), 0);
    let res = s.ctx.claim_lp(s.market, s.creator.pubkey(), a_lp_ata);
    assert!(res.is_ok(), "{res:?}");

    // Attempt to refund the same contribution on the Active market.
    let refund_dest = s.ctx.create_token_account(s.kass, s.creator.pubkey(), 0);
    let res = s.ctx.refund(s.market, s.creator.pubkey(), refund_dest);
    assert_eq!(custom_code(&res), Some(MarketError::NotCancelled as u32));

    // Silence unused-field warning on oracle in this test path.
    let _ = s.oracle;
}

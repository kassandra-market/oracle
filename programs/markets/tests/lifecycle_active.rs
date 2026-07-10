//! End-to-end integration test for the FULL active-market lifecycle:
//! `init_config → create_market → contribute → activate → claim_lp (all
//! contributors) → trade (split) → oracle resolves → resolve_market → redeem`,
//! with conservation assertions across the whole flow.
//!
//! Drives the REAL deployed MetaDAO v0.4 `conditional_vault` + `amm` binaries in
//! LiteSVM (via `ctx.load_metadao()`). This composes only already-reviewed
//! instructions; it is the capstone that proves a binary market is live from
//! funding all the way through redemption, and that no KASS is created out of
//! thin air (redemptions ≤ deposits, shortfall bounded by acknowledged dust +
//! the losers' forfeited stakes).
//!
//! Substitutions vs. the plan sketch: the "trade" leg is a client `split_tokens`
//! (not a MetaDAO `swap`) — the resolve_market tests already exercise this path
//! and it is sufficient to hand a user a net single-leg (winner/loser) position.
//! The LP `remove_liquidity` back to underlying is intentionally omitted (heavy
//! to wire; noted): the contributed KASS stays locked in the pool reserves, so
//! it simply never leaves the system, which only strengthens `out ≤ in`. The
//! split → resolve → redeem conservation for the traded portion is exact and is
//! asserted directly.

mod common;
use common::*;
use kassandra_markets_program::state::{Market, MarketStatus};
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

const PROPOSAL: u8 = 1; // kassandra Phase::Proposal (non-terminal)
const MIN_LIQ: u64 = 1_000_000_000; // 1 KASS (9 dp) — reachable by creator + 1 contributor
const SEED_A: u64 = 600_000_000; // creator's stake
const SEED_B: u64 = 400_000_000; // second contributor's stake (A + B == MIN_LIQ)
const SPLIT_AMT: u64 = 2_000_000_000; // 2 KASS each traded user splits for a position

/// `Question` field byte offsets (after the 8-byte Anchor disc).
const Q_NUM0_OFFSET: usize = 76;
const Q_NUM1_OFFSET: usize = 80;
const Q_DENOMINATOR_OFFSET: usize = 84;

/// Floor pro-rata share, mirroring the on-chain u128-intermediate LP helper.
fn expected_share(lp_total: u64, amount: u64, total: u64) -> u64 {
    u64::try_from((lp_total as u128) * (amount as u128) / (total as u128)).unwrap()
}

/// Read a little-endian `u32` from the `Question` account at `off`.
fn question_u32(ctx: &TestCtx, question: Pubkey, off: usize) -> u32 {
    let acc = ctx.svm.get_account(&question).expect("question exists");
    u32::from_le_bytes(acc.data[off..off + 4].try_into().unwrap())
}

/// Split `amount` KASS into cYES+cNO for a fresh user, then drain the leg named
/// by `drain_yes` to a sink so the user is left holding ONLY the other leg.
/// `drain_yes == true` leaves a cNO-only (losing, when YES wins) holder;
/// `drain_yes == false` leaves a cYES-only (winning) holder. Returns the user
/// and its (kass_ata, cyes, cno) accounts.
fn single_leg_holder(
    ctx: &mut TestCtx,
    kass: Pubkey,
    refs: &MetaDaoRefs,
    amount: u64,
    drain_yes: bool,
) -> (Keypair, Pubkey, Pubkey, Pubkey) {
    let user = Keypair::new();
    ctx.svm_airdrop(&user.pubkey());
    let user_kass = ctx.create_token_account(kass, user.pubkey(), amount);
    let user_cyes = ctx.create_token_account(refs.yes_mint, user.pubkey(), 0);
    let user_cno = ctx.create_token_account(refs.no_mint, user.pubkey(), 0);
    let res = ctx.user_split(&user, refs, user_kass, user_cyes, user_cno, amount);
    assert!(res.is_ok(), "user_split: {res:?}");
    assert_eq!(ctx.token_balance(user_cyes), amount, "split minted cYES");
    assert_eq!(ctx.token_balance(user_cno), amount, "split minted cNO");

    let (drain_from, drain_mint) = if drain_yes {
        (user_cyes, refs.yes_mint)
    } else {
        (user_cno, refs.no_mint)
    };
    let sink = ctx.create_token_account(drain_mint, Pubkey::new_unique(), 0);
    let ix = spl_token::instruction::transfer(
        &spl_token::ID,
        &drain_from,
        &sink,
        &user.pubkey(),
        &[],
        amount,
    )
    .unwrap();
    let res = ctx.send(ix, &[&user]);
    assert!(res.is_ok(), "drain leg: {res:?}");
    assert_eq!(ctx.token_balance(drain_from), 0, "leg drained");
    (user, user_kass, user_cyes, user_cno)
}

/// The whole binary-market lifecycle in one flow, with conservation checks at
/// each stage. Each stage is commented; it is deliberately a long test.
#[test]
fn full_active_market_lifecycle_with_conservation() {
    // ── Stage 1: init_config ────────────────────────────────────────────────
    // A `min_liquidity` of 1 KASS is reachable by the creator's seed plus a
    // single second contributor.
    let mut ctx = TestCtx::new();
    ctx.load_metadao();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    let (_cfg, res) = ctx.init_config(authority.pubkey(), kass, MIN_LIQ);
    assert!(res.is_ok(), "init_config: {res:?}");

    // ── Stage 2: create_market + contribute (fund to min) ───────────────────
    // Creator seeds SEED_A; a second contributor tops up SEED_B so that
    // total_contributed == MIN_LIQ exactly.
    let oracle = ctx.seed_kass_oracle(2, PROPOSAL);

    let creator = Keypair::new();
    ctx.svm_airdrop(&creator.pubkey());
    let creator_ata = ctx.create_token_account(kass, creator.pubkey(), 5_000_000_000);
    let (market, res) = ctx.create_market(&creator, oracle, kass, creator_ata, SEED_A);
    assert!(res.is_ok(), "create_market: {res:?}");

    let c2 = Keypair::new();
    ctx.svm_airdrop(&c2.pubkey());
    let c2_ata = ctx.create_token_account(kass, c2.pubkey(), 5_000_000_000);
    let res = ctx.contribute(&c2, market, c2_ata, SEED_B);
    assert!(res.is_ok(), "contribute: {res:?}");

    let m: Market = ctx.read_pod(market);
    assert_eq!(
        m.total_contributed, MIN_LIQ,
        "funded to exactly min_liquidity"
    );
    assert_eq!(
        m.status,
        MarketStatus::Funding.as_u8(),
        "still Funding pre-activate"
    );
    let escrow = Pubkey::new_from_array(m.escrow_vault.to_bytes());
    assert_eq!(
        ctx.token_balance(escrow),
        MIN_LIQ,
        "escrow holds contributions"
    );

    // Total KASS the contributors put IN (this is the crowdfunded liquidity).
    let total_contributed = m.total_contributed;

    // ── Stage 3: compose MetaDAO market + activate ──────────────────────────
    // The client composes the Question/vault/AMM; `activate` splits the escrow
    // into balanced cYES/cNO, seeds the 50/50 pool, and mints LP into lp_vault.
    let refs = ctx.compose_metadao_market(market, oracle, kass);
    let res = ctx.activate(oracle, kass);
    assert!(res.is_ok(), "activate: {res:?}");

    let m: Market = ctx.read_pod(market);
    assert_eq!(m.status, MarketStatus::Active.as_u8(), "status → Active");
    assert_eq!(ctx.token_balance(escrow), 0, "escrow drained by activate");
    assert_eq!(
        ctx.token_balance(refs.vault_underlying_ata),
        MIN_LIQ,
        "contributions now back the conditional vault"
    );
    assert!(
        ctx.token_balance(refs.amm_vault_base) > 0,
        "pool cYES reserve seeded"
    );
    assert!(
        ctx.token_balance(refs.amm_vault_quote) > 0,
        "pool cNO reserve seeded"
    );
    assert!(m.lp_total > 0, "LP minted");
    let (lp_vault, _) = kassandra_markets_sdk::pda::lp_vault(&market);
    assert_eq!(
        ctx.token_balance(lp_vault),
        m.lp_total,
        "lp_vault == lp_total"
    );
    let lp_total = m.lp_total;

    // NOTE: claim_lp now gates on `market.fee_collected == 1`, so the pro-rata LP
    // claims are deferred to Stage 6b (after resolve + collect_fee) below — the
    // fee crank must finalize `lp_total` before any LP is distributed.

    // ── Stage 5: trade — hand users net single-leg positions via split ──────
    // (Substituting a client `split_tokens` for a MetaDAO `swap`: it is enough
    // to leave a user holding a net winning / losing leg.)
    //   • winner  → holds only cYES (drains cNO)  — the winning leg once YES resolves
    //   • loser   → holds only cNO  (drains cYES) — the losing leg
    //   • roundtrip → keeps BOTH legs; redeems both for an EXACT 1:1 round trip
    let (winner, win_kass, win_cyes, win_cno) =
        single_leg_holder(&mut ctx, kass, &refs, SPLIT_AMT, /*drain_yes=*/ false);
    let (loser, lose_kass, lose_cyes, lose_cno) =
        single_leg_holder(&mut ctx, kass, &refs, SPLIT_AMT, /*drain_yes=*/ true);

    let roundtrip = Keypair::new();
    ctx.svm_airdrop(&roundtrip.pubkey());
    let rt_kass = ctx.create_token_account(kass, roundtrip.pubkey(), SPLIT_AMT);
    let rt_cyes = ctx.create_token_account(refs.yes_mint, roundtrip.pubkey(), 0);
    let rt_cno = ctx.create_token_account(refs.no_mint, roundtrip.pubkey(), 0);
    let res = ctx.user_split(&roundtrip, &refs, rt_kass, rt_cyes, rt_cno, SPLIT_AMT);
    assert!(res.is_ok(), "roundtrip split: {res:?}");
    assert_eq!(
        ctx.token_balance(rt_kass),
        0,
        "roundtrip spent all KASS into the vault"
    );

    // Every KASS deposited into the system so far: crowdfunded liquidity (now in
    // the vault) + each traded user's split.
    let total_kass_in = total_contributed + SPLIT_AMT + SPLIT_AMT + SPLIT_AMT;

    // ── Stage 6: oracle resolves YES + resolve_market ───────────────────────
    ctx.set_oracle_resolved(oracle, 0); // option 0 == YES wins → numerators [1,0]
    let res = ctx.resolve_market(market, oracle, refs.question);
    assert!(res.is_ok(), "resolve_market: {res:?}");

    let m: Market = ctx.read_pod(market);
    assert_eq!(m.settled, 1, "settled flag set");
    assert_eq!(
        m.status,
        MarketStatus::Resolved.as_u8(),
        "status → Resolved"
    );
    assert_ne!(
        question_u32(&ctx, refs.question, Q_DENOMINATOR_OFFSET),
        0,
        "question resolved"
    );
    assert_eq!(
        question_u32(&ctx, refs.question, Q_NUM0_OFFSET),
        1,
        "YES numerator == 1"
    );
    assert_eq!(
        question_u32(&ctx, refs.question, Q_NUM1_OFFSET),
        0,
        "NO numerator == 0"
    );
    // Default fee_bps == 100 (> 0): the collect_fee crank must still run.
    assert_eq!(m.fee_collected, 0, "fee crank pending after resolve");

    // ── Stage 6a: collect_fee (permissionless crank) ────────────────────────
    // The "trade" above was a client `split_tokens`, which does NOT touch the pool
    // reserves, so the LP position accrued no earnings → the fee floors to ~0. The
    // crank still runs, redeems nothing meaningful, and stamps `fee_collected`.
    let fee_dest = ctx.config_fee_destination();
    let res = ctx.collect_fee(oracle, kass, fee_dest);
    assert!(res.is_ok(), "collect_fee: {res:?}");
    let m: Market = ctx.read_pod(market);
    assert_eq!(m.fee_collected, 1, "fee_collected stamped by crank");
    // No pool activity → no accrual → lp_total unchanged from activation.
    assert_eq!(m.lp_total, lp_total, "no accrual: lp_total intact");

    // ── Stage 6b: claim_lp for BOTH contributors (post-fee) ─────────────────
    // A claims first (floor pro-rata); B claims LAST (counter == 1) and sweeps the
    // ENTIRE remaining lp_vault to 0 — absorbing A's floor dust. Each claim CLOSES
    // its Contribution, returning that rent to the contributor.
    let a_share = expected_share(lp_total, SEED_A, total_contributed);
    assert!(a_share > 0, "A share positive");
    assert!(a_share <= lp_total, "LP claim never over-distributes");

    let a_lp_ata = ctx.create_token_account(refs.lp_mint, creator.pubkey(), 0);
    let res = ctx.claim_lp(market, creator.pubkey(), a_lp_ata);
    assert!(res.is_ok(), "claim_lp A: {res:?}");
    assert_eq!(ctx.token_balance(a_lp_ata), a_share, "A got pro-rata LP");
    assert_eq!(
        ctx.read_pod::<Market>(market).open_contributions,
        1,
        "counter 2 → 1 after A"
    );

    let vault_before_b = ctx.token_balance(lp_vault);
    assert_eq!(
        vault_before_b,
        lp_total - a_share,
        "vault holds the remainder"
    );
    let b_lp_ata = ctx.create_token_account(refs.lp_mint, c2.pubkey(), 0);
    let res = ctx.claim_lp(market, c2.pubkey(), b_lp_ata);
    assert!(res.is_ok(), "claim_lp B: {res:?}");
    assert_eq!(
        ctx.token_balance(b_lp_ata),
        vault_before_b,
        "B sweeps the remainder"
    );

    // Last-claimer sweep: lp_vault ends at EXACTLY 0 (no un-closeable dust).
    assert_eq!(
        ctx.token_balance(lp_vault),
        0,
        "lp_vault swept to 0 by last claim"
    );
    for who in [creator.pubkey(), c2.pubkey()] {
        let (cpda, _) = kassandra_markets_sdk::pda::contribution(&market, &who);
        assert_eq!(ctx.lamports(cpda), 0, "Contribution closed");
    }
    assert_eq!(
        ctx.read_pod::<Market>(market).open_contributions,
        0,
        "counter drained to 0"
    );

    // ── Stage 7: redemptions ────────────────────────────────────────────────
    // Winner redeems the winning cYES 1:1; the drained (worthless) cNO is gone,
    // so the winner's round trip is EXACT: put SPLIT_AMT in, gets SPLIT_AMT back.
    let res = ctx.redeem(&winner, &refs, win_kass, win_cyes, win_cno);
    assert!(res.is_ok(), "winner redeem: {res:?}");
    let winner_out = ctx.token_balance(win_kass);
    assert_eq!(winner_out, SPLIT_AMT, "winner paid full stake 1:1");
    assert!(winner_out > 0, "winner paid a positive amount");

    // Loser redeems the losing cNO → nothing. Their stake is forfeited (it sits
    // in the vault as the winning cYES they threw to the sink — never redeemed).
    let res = ctx.redeem(&loser, &refs, lose_kass, lose_cyes, lose_cno);
    assert!(res.is_ok(), "loser redeem: {res:?}");
    let loser_out = ctx.token_balance(lose_kass);
    assert_eq!(loser_out, 0, "losing leg pays 0");

    // Roundtrip holder redeems BOTH legs: cYES pays 1:1, cNO pays 0 → exactly
    // their split back. This is the tight, dust-free traded-portion conservation.
    let res = ctx.redeem(&roundtrip, &refs, rt_kass, rt_cyes, rt_cno);
    assert!(res.is_ok(), "roundtrip redeem: {res:?}");
    let roundtrip_out = ctx.token_balance(rt_kass);
    assert_eq!(
        roundtrip_out, SPLIT_AMT,
        "roundtrip conserves exactly (no dust)"
    );

    // ── Stage 8: conservation ───────────────────────────────────────────────
    // No instruction panicked (every `res.is_ok()` above held). Escrow is empty.
    // Total KASS redeemed out never exceeds total KASS deposited in; the only
    // KASS that does NOT come back out is the crowdfunded liquidity still locked
    // in the pool reserves (LP-removal omitted) plus the loser's forfeited stake
    // — never program-created dust. The two exact round trips (winner, roundtrip)
    // prove the traded portion conserves to the base unit.
    assert_eq!(ctx.token_balance(escrow), 0, "escrow fully drained");
    let total_kass_out = winner_out + loser_out + roundtrip_out;
    assert!(
        total_kass_out <= total_kass_in,
        "no KASS minted from nothing: out {total_kass_out} ≤ in {total_kass_in}"
    );
    // Traded-portion round trips are exact (0 KASS dust across split→redeem).
    assert_eq!(winner_out, SPLIT_AMT, "winner round trip exact");
    assert_eq!(roundtrip_out, SPLIT_AMT, "roundtrip exact");

    // ── Stage 9: close_market — reclaim all rent to the creator ─────────────
    // Every contributor has claimed (open_contributions == 0), the market is
    // Resolved with fee_collected == 1, and escrow/cyes/cno/lp_vault are all at 0.
    // close_market SPL-closes the four token accounts and the Market PDA, routing
    // every reclaimed lamport to the creator.
    let (cyes, _) = kassandra_markets_sdk::pda::market_cyes(&market);
    let (cno, _) = kassandra_markets_sdk::pda::market_cno(&market);
    for ta in [escrow, cyes, cno, lp_vault] {
        assert_eq!(ctx.token_balance(ta), 0, "token account empty pre-close");
    }
    let reclaimable = ctx.lamports(market)
        + ctx.lamports(escrow)
        + ctx.lamports(cyes)
        + ctx.lamports(cno)
        + ctx.lamports(lp_vault);
    assert!(reclaimable > 0, "there is rent to reclaim");
    let creator_before = ctx.lamports(creator.pubkey());
    let res = ctx.close_market(oracle, creator.pubkey());
    assert!(res.is_ok(), "close_market: {res:?}");
    // Market + all four token accounts are reaped; every lamport went to the creator.
    for gone in [market, escrow, cyes, cno, lp_vault] {
        assert_eq!(ctx.lamports(gone), 0, "account reaped by close_market");
    }
    assert_eq!(
        ctx.lamports(creator.pubkey()),
        creator_before + reclaimable,
        "all reclaimed rent → creator"
    );
}

//! Higher-level `settle_challenge` fixtures: the AI-claim seed, the
//! `open_challenge`/`settle` instruction builders, host-side resolution readers,
//! and the full `fixture` composer that stands up a disputed oracle with real
//! pass/fail AMMs. (Pure relocation from the crate root; `pub(crate)` is
//! visibility glue only.)

use super::support::*;
use super::*;

use kassandra_oracles_program::{
    cpi::metadao,
    instruction::Ix,
    state::{AccountType, AiClaim, Phase},
};
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use spl_token::ID as TOKEN_PROGRAM_ID;

fn seed_ai_claim(ctx: &mut TestCtx, oracle: Pubkey, proposer: Pubkey, option: u8) -> Pubkey {
    let (claim, bump) = Pubkey::find_program_address(
        &[b"claim", oracle.as_ref(), proposer.as_ref()],
        &ctx.program_id,
    );
    let mut c: AiClaim = bytemuck::Zeroable::zeroed();
    c.account_type = AccountType::AiClaim.as_u8();
    c.oracle = oracle.to_bytes().into();
    c.proposer = proposer.to_bytes().into();
    c.option = option;
    c.challenged = 0;
    c.bump = bump;
    ctx.seed_program_account_at(claim, bytemuck::bytes_of(&c).to_vec());
    claim
}

#[allow(clippy::too_many_arguments)]
fn open_challenge_ix(
    ctx: &TestCtx,
    oracle: Pubkey,
    ai_claim: Pubkey,
    proposer: Pubkey,
    market: Pubkey,
    challenger: Pubkey,
    m: &MarketAccounts,
    pass_amm: Pubkey,
    fail_amm: Pubkey,
    stake_vault: Pubkey,
    oracle_pass_kass: Pubkey,
    oracle_fail_kass: Pubkey,
    kass_dao: Pubkey,
    challenger_usdc_src: Pubkey,
    nonce: u64,
) -> Instruction {
    let (cv_event_auth, _) =
        Pubkey::find_program_address(&metadao::event_authority_seeds(), &vault_id());
    let (protocol, _) = TestCtx::protocol_pda(&ctx.program_id);
    let (escrow_vault, _) = TestCtx::challenge_usdc_vault_pda(&ctx.program_id, &market);
    let mut data = vec![Ix::OpenChallenge as u8];
    data.extend_from_slice(&nonce.to_le_bytes());
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(ai_claim, false),
            AccountMeta::new(proposer, false),
            AccountMeta::new(market, false),
            AccountMeta::new(challenger, true),
            AccountMeta::new_readonly(m.question, false),
            AccountMeta::new(m.kass_vault, false),
            AccountMeta::new_readonly(m.usdc_vault, false),
            AccountMeta::new_readonly(pass_amm, false),
            AccountMeta::new_readonly(fail_amm, false),
            AccountMeta::new(stake_vault, false),
            AccountMeta::new(m.kass_vault_underlying, false),
            AccountMeta::new(m.pass_mint, false),
            AccountMeta::new(m.fail_mint, false),
            AccountMeta::new(oracle_pass_kass, false),
            AccountMeta::new(oracle_fail_kass, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(solana_sdk_ids::system_program::ID, false),
            AccountMeta::new_readonly(cv_event_auth, false),
            AccountMeta::new_readonly(protocol, false),
            AccountMeta::new_readonly(kass_dao, false),
            AccountMeta::new_readonly(ctx.usdc_mint, false),
            AccountMeta::new(challenger_usdc_src, false),
            AccountMeta::new(escrow_vault, false),
        ],
        data,
    }
}

/// Settlement accounts beyond the core 0..=8 (C2 physical redeem + fees).
pub(crate) struct SettleExtras {
    stake_vault: Pubkey,
    kass_vault: Pubkey,
    kass_vault_underlying: Pubkey,
    pass_mint: Pubkey,
    fail_mint: Pubkey,
    oracle_pass_kass: Pubkey,
    oracle_fail_kass: Pubkey,
    escrow_vault: Pubkey,
    proposer_usdc: Pubkey,
    challenger_usdc_dest: Pubkey,
    challenger_kass: Pubkey,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn settle_ix(
    ctx: &TestCtx,
    oracle: Pubkey,
    market: Pubkey,
    ai_claim: Pubkey,
    proposer: Pubkey,
    question: Pubkey,
    pass_amm: Pubkey,
    fail_amm: Pubkey,
    x: &SettleExtras,
    nonce: u64,
) -> Instruction {
    let (cv_event_auth, _) =
        Pubkey::find_program_address(&metadao::event_authority_seeds(), &vault_id());
    let mut data = vec![Ix::SettleChallenge as u8];
    data.extend_from_slice(&nonce.to_le_bytes());
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(market, false),
            AccountMeta::new_readonly(ai_claim, false),
            AccountMeta::new(proposer, false),
            AccountMeta::new(question, false),
            AccountMeta::new_readonly(pass_amm, false),
            AccountMeta::new_readonly(fail_amm, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new_readonly(cv_event_auth, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new(x.stake_vault, false),
            AccountMeta::new(x.kass_vault, false),
            AccountMeta::new(x.kass_vault_underlying, false),
            AccountMeta::new(x.pass_mint, false),
            AccountMeta::new(x.fail_mint, false),
            AccountMeta::new(x.oracle_pass_kass, false),
            AccountMeta::new(x.oracle_fail_kass, false),
            AccountMeta::new(x.escrow_vault, false),
            AccountMeta::new(x.proposer_usdc, false),
            AccountMeta::new(x.challenger_usdc_dest, false),
            AccountMeta::new(x.challenger_kass, false),
        ],
        data,
    }
}

/// Read a little-endian `u32` from a (host-side) account-data slice.
fn read_u32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(data[off..off + 4].try_into().unwrap())
}

/// Question payout numerators (`[n0, n1]`) + denominator, decoded host-side.
/// Layout: disc[8] question_id[32]@8 oracle@40 payout_numerators(Vec<u32>) len@72
/// vals@76,@80 payout_denominator@84.
pub(crate) fn question_resolution(ctx: &TestCtx, question: Pubkey) -> (u32, u32, u32) {
    let acc = ctx.svm.get_account(&question).expect("question missing");
    (
        read_u32(&acc.data, 76),
        read_u32(&acc.data, 80),
        read_u32(&acc.data, 84),
    )
}

/// Physical KASS-conservation invariant (design §9 #3) for the split path:
/// the bond that left `stake_vault` is exactly what is now escrowed in the
/// MetaDAO KASS conditional vault's underlying token account, and
/// `total_oracle_stake` stays the conserved accumulator (it is intentionally NOT
/// decremented by the split — the KASS is still in-system, just escrowed). The
/// challenge split is the ONLY path where KASS physically leaves `stake_vault`,
/// so this asserts nothing was created or destroyed there.
pub(crate) fn assert_kass_conserved(ctx: &TestCtx, oracle: Pubkey, kass_vault_underlying: Pubkey) {
    let stake_vault = ctx.seeded(oracle).stake_vault;
    let total = ctx.oracle(oracle).total_oracle_stake;
    assert_eq!(
        ctx.token_balance(stake_vault) + ctx.token_balance(kass_vault_underlying),
        total,
        "physical KASS conservation: stake_vault + conditional-vault underlying == total_oracle_stake",
    );
}

pub(crate) const BOND: u64 = 1_000_000_000;
/// Base reserve common to all pools: 100 KASS (9 dp).
const BASE_RESERVE: u64 = 100_000_000_000;
/// Quote reserve giving price 1e9 (100 USDC, 6 dp). add_liquidity needs ≥ 1e8.
pub(crate) const QUOTE_LOW: u64 = 100_000_000;
/// Quote reserve giving price 3e9 (300 USDC) — fail far above pass+threshold.
pub(crate) const QUOTE_HIGH: u64 = 300_000_000;

pub(crate) struct Fixture {
    pub(crate) oracle: Pubkey,
    pub(crate) nonce: u64,
    pub(crate) proposer: Pubkey,
    pub(crate) proposer_other: Pubkey,
    pub(crate) ai_claim: Pubkey,
    pub(crate) market: Pubkey,
    pub(crate) m: MarketAccounts,
    pub(crate) pass_amm: Pubkey,
    pub(crate) fail_amm: Pubkey,
    // C2 physical-settlement accounts.
    pub(crate) stake_vault: Pubkey,
    pub(crate) oracle_pass_kass: Pubkey,
    pub(crate) oracle_fail_kass: Pubkey,
    pub(crate) escrow_vault: Pubkey,
    pub(crate) proposer_usdc: Pubkey,
    pub(crate) challenger_usdc_dest: Pubkey,
    pub(crate) challenger_kass: Pubkey,
}

impl Fixture {
    pub(crate) fn extras(&self) -> SettleExtras {
        SettleExtras {
            stake_vault: self.stake_vault,
            kass_vault: self.m.kass_vault,
            kass_vault_underlying: self.m.kass_vault_underlying,
            pass_mint: self.m.pass_mint,
            fail_mint: self.m.fail_mint,
            oracle_pass_kass: self.oracle_pass_kass,
            oracle_fail_kass: self.oracle_fail_kass,
            escrow_vault: self.escrow_vault,
            proposer_usdc: self.proposer_usdc,
            challenger_usdc_dest: self.challenger_usdc_dest,
            challenger_kass: self.challenger_kass,
        }
    }
}

/// What (if anything) to corrupt about the recorded AMMs, for the attack tests.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum AmmAttack {
    /// Record the canonical real pass/fail pools (honest path).
    None,
    /// Real pools, but the PASS pool is never cranked (pass_twap == 0). Used to
    /// prove an un-cranked pass side makes the claim survive regardless of fail.
    PassUncranked,
}

/// Full fixture: disputed oracle in Challenge, one challenged proposer with a
/// composed market and REAL pass/fail AMMs at the requested quote reserves, then
/// `open_challenge` run so the Market records everything. Leaves `now` unchanged
/// (only slots advanced), so the caller controls crossing `twap_end`.
pub(crate) fn fixture(pass_quote: u64, fail_quote: u64) -> (TestCtx, Fixture) {
    fixture_with_attack(pass_quote, fail_quote, AmmAttack::None)
}

pub(crate) fn fixture_with_attack(
    pass_quote: u64,
    fail_quote: u64,
    attack: AmmAttack,
) -> (TestCtx, Fixture) {
    let mut ctx = TestCtx::new();
    ctx.svm.add_program(vault_id(), VAULT_SO).unwrap();
    ctx.svm.add_program(amm_id(), AMM_SO).unwrap();

    // Protocol + governance with a deterministic kass_price so open_challenge
    // can size + escrow the challenger USDC.
    let kass_dao = ctx.bless_kass_price();

    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: BOND,
        },
        ProposerSpec {
            option: 1,
            bond: BOND,
        },
    ]);
    let seeded = ctx.seeded(oracle);
    let nonce = seeded.nonce;
    let stake_vault = seeded.stake_vault;
    let proposer = seeded.proposers[0].pda;
    let proposer_authority = seeded.proposers[0].authority.pubkey();
    let proposer_other = seeded.proposers[1].pda;

    ctx.set_phase(oracle, Phase::Challenge);
    let ai_claim = seed_ai_claim(&mut ctx, oracle, proposer, 0);

    let (m, oracle_pass_kass, oracle_fail_kass) = setup_market(&mut ctx, oracle);

    // Real pass/fail AMMs over the conditional (KASS, USDC) mint pairs. The PASS
    // pool is left un-cranked only for the PassUncranked case.
    let crank_pass = attack != AmmAttack::PassUncranked;
    let real_pass = build_amm(
        &mut ctx,
        m.pass_mint,
        m.pass_usdc,
        BASE_RESERVE,
        pass_quote,
        crank_pass,
    );
    let real_fail = build_amm(
        &mut ctx,
        m.fail_mint,
        m.fail_usdc,
        BASE_RESERVE,
        fail_quote,
        true,
    );
    // Both remaining attack modes record the real pools (PassUncranked just
    // leaves the pass side un-cranked); AMM-binding attacks are rejected at open.
    let (pass_amm, fail_amm) = (real_pass, real_fail);

    let (market, _) =
        Pubkey::find_program_address(&[b"market", ai_claim.as_ref()], &ctx.program_id);

    let challenger = Keypair::new();
    ctx.svm
        .airdrop(&challenger.pubkey(), 1_000_000_000)
        .unwrap();
    let challenger_usdc_src = ctx.fund_usdc(&challenger, 5_000_000);

    let ix = open_challenge_ix(
        &ctx,
        oracle,
        ai_claim,
        proposer,
        market,
        challenger.pubkey(),
        &m,
        pass_amm,
        fail_amm,
        stake_vault,
        oracle_pass_kass,
        oracle_fail_kass,
        kass_dao,
        challenger_usdc_src,
        nonce,
    );
    ctx.send_many(&cu(ix), &[&challenger])
        .expect("open_challenge should succeed");

    // Payout destinations for settle: proposer's USDC (fee on survive),
    // challenger's USDC (escrow return) + KASS (fee on disqualify). Fabricated
    // empty; settle pays into them.
    let usdc_mint = ctx.usdc_mint;
    let kass_mint = ctx.kass_mint;
    let challenger_pk = challenger.pubkey();
    let proposer_usdc = Pubkey::new_unique();
    let challenger_usdc_dest = Pubkey::new_unique();
    let challenger_kass = Pubkey::new_unique();
    fabricate_token_account(&mut ctx, proposer_usdc, usdc_mint, proposer_authority, 0);
    fabricate_token_account(&mut ctx, challenger_usdc_dest, usdc_mint, challenger_pk, 0);
    fabricate_token_account(&mut ctx, challenger_kass, kass_mint, challenger_pk, 0);
    let (escrow_vault, _) = TestCtx::challenge_usdc_vault_pda(&ctx.program_id, &market);

    (
        ctx,
        Fixture {
            oracle,
            nonce,
            proposer,
            proposer_other,
            ai_claim,
            market,
            m,
            pass_amm,
            fail_amm,
            stake_vault,
            oracle_pass_kass,
            oracle_fail_kass,
            escrow_vault,
            proposer_usdc,
            challenger_usdc_dest,
            challenger_kass,
        },
    )
}

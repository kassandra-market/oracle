// ---------------------------------------------------------------------------
// Conservation FUZZ (deliverable 2)
//
// Sweeps both outcomes × fee rates × bond sizes × pass/fail TWAP relation and
// asserts the KASS + USDC conservation equations across REAL open_challenge +
// settle_challenge against the INDEPENDENT ConservationModel. To keep each case
// cheap (the real-AMM TWAP-production path is heavy — and is covered by the two
// e2e tests above + settle_challenge.rs), the pass/fail AMMs are FABRICATED
// AMM-program-owned accounts carrying a chosen aggregator, so `verify_and_read_twap`
// reads a known pass_twap/fail_twap. open_challenge (split + escrow) and
// settle_challenge (redeem + directional fees) are the REAL instructions under test.
// ---------------------------------------------------------------------------

use super::ops::*;
use super::support::*;
use super::*;

use kassandra_oracles_program::{
    cpi::metadao,
    state::{AccountType, AiClaim, Phase},
};
use proptest::prelude::*;
use solana_account::Account;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

/// Fabricate an `Amm`-program-owned account that `verify_and_read_twap` accepts
/// (correct discriminator + base/quote conditional mints) and whose stored
/// aggregator/slots yield exactly `twap` (`twap == 0` ⇒ no observation).
fn fabricate_amm_with_twap(ctx: &mut TestCtx, base: Pubkey, quote: Pubkey, twap: u128) -> Pubkey {
    let addr = Pubkey::new_unique();
    let mut data = vec![0u8; metadao::AMM_MIN_LEN.max(256)];
    data[..8].copy_from_slice(&metadao::AMM_ACCOUNT_DISCRIMINATOR);
    let slots: u64 = 1_000;
    let (created_at, last_updated, aggregator): (u64, u64, u128) = if twap == 0 {
        (0, 0, 0)
    } else {
        (0, slots, twap * slots as u128)
    };
    data[metadao::AMM_CREATED_AT_SLOT_OFFSET..metadao::AMM_CREATED_AT_SLOT_OFFSET + 8]
        .copy_from_slice(&created_at.to_le_bytes());
    data[metadao::AMM_BASE_MINT_OFFSET..metadao::AMM_BASE_MINT_OFFSET + 32]
        .copy_from_slice(&base.to_bytes());
    data[metadao::AMM_QUOTE_MINT_OFFSET..metadao::AMM_QUOTE_MINT_OFFSET + 32]
        .copy_from_slice(&quote.to_bytes());
    data[metadao::AMM_LAST_UPDATED_SLOT_OFFSET..metadao::AMM_LAST_UPDATED_SLOT_OFFSET + 8]
        .copy_from_slice(&last_updated.to_le_bytes());
    data[metadao::AMM_AGGREGATOR_OFFSET..metadao::AMM_AGGREGATOR_OFFSET + 16]
        .copy_from_slice(&aggregator.to_le_bytes());
    data[metadao::AMM_START_DELAY_SLOTS_OFFSET..metadao::AMM_START_DELAY_SLOTS_OFFSET + 8]
        .copy_from_slice(&0u64.to_le_bytes());
    let lamports = ctx.svm.minimum_balance_for_rent_exemption(data.len());
    ctx.svm
        .set_account(
            addr,
            Account {
                lamports,
                data,
                owner: amm_id(),
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();
    addr
}

#[derive(Clone, Copy, Debug)]
struct FuzzCase {
    bond: u64,
    pass_twap: u128,
    fail_twap: u128,
    succ_num: u64,
    succ_den: u64,
    fail_num: u64,
    fail_den: u64,
}

fn fuzz_case_strategy() -> impl Strategy<Value = FuzzCase> {
    (
        1_000_000u64..5_000_000_000u64, // bond (escrow = bond * 5e8 / 1e12 > 0)
        0u128..4_000_000_000u128,       // pass_twap (incl. 0 → always survive)
        100_000_000u128..12_000_000_000u128, // fail_twap
        // Fee rates within bounds (num ≤ den, den > 0). Keep succ_num/den ≤ ~50%
        // so the kass_fee never collides with the (here-zero) prior slash.
        (1u64..=50u64, 100u64..=100u64),
        (1u64..=50u64, 100u64..=100u64),
    )
        .prop_map(
            |(bond, pass_twap, fail_twap, (succ_num, succ_den), (fail_num, fail_den))| FuzzCase {
                bond,
                pass_twap,
                fail_twap,
                succ_num,
                succ_den,
                fail_num,
                fail_den,
            },
        )
}

fn run_fuzz_case(fc: &FuzzCase) -> Result<(), TestCaseError> {
    let mut ctx = TestCtx::new();
    ctx.svm.add_program(vault_id(), VAULT_SO).unwrap();
    ctx.svm.add_program(amm_id(), AMM_SO).unwrap();
    let kass_dao = ctx.bless_kass_price();

    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: fc.bond,
        },
        ProposerSpec {
            option: 1,
            bond: fc.bond,
        },
    ]);
    // Retune the per-oracle fee snapshot to the fuzzed rates.
    ctx.set_challenge_fees(oracle, fc.fail_num, fc.fail_den, fc.succ_num, fc.succ_den);
    let seeded = ctx.seeded(oracle);
    let nonce = seeded.nonce;
    let stake_vault = seeded.stake_vault;
    let proposer = seeded.proposers[0].pda;
    let proposer_authority = seeded.proposers[0].authority.pubkey();
    ctx.set_phase(oracle, Phase::Challenge);

    let (claim, bump) = Pubkey::find_program_address(
        &[b"claim", oracle.as_ref(), proposer.as_ref()],
        &ctx.program_id,
    );
    let mut a: AiClaim = bytemuck::Zeroable::zeroed();
    a.account_type = AccountType::AiClaim.as_u8();
    a.oracle = oracle.to_bytes().into();
    a.proposer = proposer.to_bytes().into();
    a.option = 0;
    a.bump = bump;
    ctx.seed_program_account_at(claim, bytemuck::bytes_of(&a).to_vec());

    let (m, oracle_pass_kass, oracle_fail_kass) = setup_market(&mut ctx, oracle);
    // Stubbed-TWAP AMMs (see module note): known pass/fail TWAP, real binding.
    let pass_amm = fabricate_amm_with_twap(&mut ctx, m.pass_mint, m.pass_usdc, fc.pass_twap);
    let fail_amm = fabricate_amm_with_twap(&mut ctx, m.fail_mint, m.fail_usdc, fc.fail_twap);

    let (market, _) = Pubkey::find_program_address(&[b"market", claim.as_ref()], &ctx.program_id);
    let challenger = Keypair::new();
    ctx.svm
        .airdrop(&challenger.pubkey(), 1_000_000_000)
        .unwrap();
    let challenger_usdc_src = ctx.fund_usdc(&challenger, 1_000_000_000);

    let ix = open_challenge_ix(
        &ctx,
        oracle,
        claim,
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
        .map_err(|e| TestCaseError::fail(format!("open_challenge: {e:?}")))?;

    let payouts = fabricate_payouts(&mut ctx, market, proposer_authority, challenger.pubkey());
    let escrow = ctx.token_balance(payouts.escrow_vault);
    prop_assert!(escrow > 0, "escrow must be funded");

    let total_before = ctx.oracle(oracle).total_oracle_stake;
    let bond_pool_before = ctx.oracle(oracle).bond_pool;
    let stake_before = ctx.token_balance(stake_vault);

    ctx.warp(TWAP_WINDOW + 1);
    let extras = SettleExtras {
        stake_vault,
        kass_vault: m.kass_vault,
        kass_vault_underlying: m.kass_vault_underlying,
        pass_mint: m.pass_mint,
        fail_mint: m.fail_mint,
        oracle_pass_kass,
        oracle_fail_kass,
        escrow_vault: payouts.escrow_vault,
        proposer_usdc: payouts.proposer_usdc,
        challenger_usdc_dest: payouts.challenger_usdc_dest,
        challenger_kass: payouts.challenger_kass,
    };
    let ix = settle_ix(
        &ctx, oracle, market, claim, proposer, m.question, pass_amm, fail_amm, &extras, nonce,
    );
    ctx.send_many(&cu(ix), &[])
        .map_err(|e| TestCaseError::fail(format!("settle_challenge: {e:?}")))?;

    // Independent reference: outcome + conservation.
    let disqualify = ref_disqualify(fc.pass_twap, fc.fail_twap);
    let model = ConservationModel::compute(
        disqualify,
        fc.bond,
        escrow,
        0,
        fc.succ_num,
        fc.succ_den,
        fc.fail_num,
        fc.fail_den,
    );

    let (n0, n1, _) = question_resolution(&ctx, m.question);
    if disqualify {
        prop_assert_eq!((n0, n1), (0, 1));
    } else {
        prop_assert_eq!((n0, n1), (1, 0));
    }
    prop_assert_eq!(ctx.proposer(proposer).disqualified != 0, disqualify);

    // KASS.
    prop_assert_eq!(
        ctx.token_balance(payouts.challenger_kass),
        model.challenger_kass()
    );
    prop_assert_eq!(
        ctx.token_balance(stake_vault),
        stake_before + model.stake_vault_delta()
    );
    prop_assert_eq!(ctx.token_balance(m.kass_vault_underlying), 0);
    prop_assert_eq!(
        ctx.token_balance(stake_vault)
            + ctx.token_balance(m.kass_vault_underlying)
            + ctx.token_balance(payouts.challenger_kass),
        total_before,
        "KASS conservation incl. the kass_fee carve-out"
    );
    // USDC.
    prop_assert_eq!(
        ctx.token_balance(payouts.proposer_usdc),
        model.proposer_usdc()
    );
    prop_assert_eq!(
        ctx.token_balance(payouts.challenger_usdc_dest),
        model.challenger_usdc()
    );
    prop_assert_eq!(
        ctx.token_balance(payouts.proposer_usdc) + ctx.token_balance(payouts.challenger_usdc_dest),
        escrow,
        "USDC escrow fully accounted"
    );
    // bond_pool identity.
    if disqualify {
        prop_assert_eq!(
            ctx.oracle(oracle).bond_pool,
            bond_pool_before + model.stake_vault_delta()
        );
    } else {
        prop_assert_eq!(ctx.oracle(oracle).bond_pool, bond_pool_before);
    }
    Ok(())
}

proptest! {
    // Each case rebuilds LiteSVM, loads the vault + amm binaries, composes the
    // real conditional vaults, and drives real open + settle (~7 txs/case), so
    // the count is kept modest to stay fast and non-flaky.
    #![proptest_config(ProptestConfig {
        cases: 24,
        max_shrink_iters: 32,
        .. ProptestConfig::default()
    })]

    #[test]
    fn challenge_conservation_fuzz(fc in fuzz_case_strategy()) {
        run_fuzz_case(&fc)?;
    }
}

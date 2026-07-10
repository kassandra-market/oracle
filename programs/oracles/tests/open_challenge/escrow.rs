use super::*;
use super::fixture::*;
use super::support::*;

use kassandra_oracles_program::{error::KassandraError, state::Market};
use solana_instruction_error::InstructionError;
use solana_transaction_error::TransactionError;

#[test]
fn open_challenge_happy_path() {
    let (mut ctx, f) = fixture();

    let stake_before = ctx.token_balance(f.stake_vault);
    let now_before = ctx.now();
    let src_before = ctx.token_balance(f.challenger_usdc_src);
    let expected_usdc = required_escrow_usdc(f.bond);
    let (escrow_vault, _) = TestCtx::challenge_usdc_vault_pda(&ctx.program_id, &f.market);

    let ix = open_challenge_ix(
        &ctx,
        f.oracle,
        f.ai_claim,
        f.proposer,
        f.market,
        f.challenger.pubkey(),
        &f.m,
        f.stake_vault,
        f.oracle_pass_kass,
        f.oracle_fail_kass,
        f.kass_dao,
        f.challenger_usdc_src,
        f.nonce,
    );
    ctx.send_many(&cu(ix), &[&f.challenger])
        .expect("open_challenge should succeed");

    // Market PDA created + populated.
    let market: Market = ctx.read_pod(f.market);
    assert_eq!(market.account_type, AccountType::Market.as_u8());
    assert_eq!(market.oracle, f.oracle.to_bytes().into());
    assert_eq!(market.ai_claim, f.ai_claim.to_bytes().into());
    assert_eq!(market.proposer, f.proposer.to_bytes().into());
    assert_eq!(market.challenger, f.challenger.pubkey().to_bytes().into());
    assert_eq!(market.question, f.m.question.to_bytes().into());
    assert_eq!(market.kass_vault, f.m.kass_vault.to_bytes().into());
    assert_eq!(market.usdc_vault, f.m.usdc_vault.to_bytes().into());
    assert_eq!(market.pass_amm, f.m.pass_amm.to_bytes().into());
    assert_eq!(market.fail_amm, f.m.fail_amm.to_bytes().into());
    assert_eq!(
        market.oracle_pass_kass,
        f.oracle_pass_kass.to_bytes().into()
    );
    assert_eq!(
        market.oracle_fail_kass,
        f.oracle_fail_kass.to_bytes().into()
    );
    assert_eq!(market.challenger_usdc_vault, escrow_vault.to_bytes().into());
    assert_eq!(market.twap_end, now_before + TWAP_WINDOW);
    assert_eq!(market.settled, 0);

    // Escrow: exactly bond × kass_price USDC moved challenger → market vault.
    assert!(
        expected_usdc > 0,
        "sanity: nonzero escrow at the test price"
    );
    assert_eq!(
        market.challenger_usdc, expected_usdc,
        "Market records the on-chain-computed escrow size"
    );
    assert_eq!(
        ctx.token_balance(escrow_vault),
        expected_usdc,
        "escrow vault holds exactly bond × kass_price USDC"
    );
    assert_eq!(
        ctx.token_balance(f.challenger_usdc_src),
        src_before - expected_usdc,
        "challenger's USDC source debited by the escrow amount"
    );
    // Escrow vault is on the USDC mint, token authority == oracle PDA.
    let (mint, owner, _amt) = ctx.token_account(escrow_vault);
    assert_eq!(mint, ctx.usdc_mint.to_bytes());
    assert_eq!(owner, f.oracle.to_bytes());

    // Claim flipped to challenged.
    assert_eq!(ctx.ai_claim(f.ai_claim).challenged, 1);

    // Program-signed split moved exactly the bond out of the stake vault into
    // the KASS conditional vault, minting pass/fail conditional KASS to the
    // oracle-PDA-owned destinations.
    assert_eq!(ctx.token_balance(f.stake_vault), stake_before - f.bond);
    assert_eq!(ctx.token_balance(f.m.kass_vault_underlying), f.bond);
    assert_eq!(ctx.token_balance(f.oracle_pass_kass), f.bond);
    assert_eq!(ctx.token_balance(f.oracle_fail_kass), f.bond);
}

#[test]
fn open_challenge_insufficient_usdc_fails() {
    let (mut ctx, f) = fixture();

    // A USDC source holding far less than the required escrow (bond × price).
    // The escrow Transfer must fail, reverting the whole instruction — no Market
    // and no challenged flip persist.
    let expected_usdc = required_escrow_usdc(f.bond);
    assert!(expected_usdc > 1, "test price requires a real escrow");
    let poor_src = ctx.fund_usdc(&f.challenger, 1);

    let ix = open_challenge_ix(
        &ctx,
        f.oracle,
        f.ai_claim,
        f.proposer,
        f.market,
        f.challenger.pubkey(),
        &f.m,
        f.stake_vault,
        f.oracle_pass_kass,
        f.oracle_fail_kass,
        f.kass_dao,
        poor_src,
        f.nonce,
    );
    let res = ctx.send_many(&cu(ix), &[&f.challenger]);
    assert!(
        res.is_err(),
        "an under-funded challenger must fail the escrow Transfer: {res:?}"
    );
    // Whole tx reverted: no Market account, claim not challenged.
    assert!(
        ctx.svm.get_account(&f.market).is_none(),
        "failed escrow must not leave a Market account"
    );
    assert_eq!(ctx.ai_claim(f.ai_claim).challenged, 0);
}

#[test]
fn open_challenge_zero_escrow_fails() {
    // A sub-micro bond (1 base unit) prices to `1 × 5e8 / 1e12 == 0` USDC escrow.
    // A zero-escrow challenge has no skin-in-the-game and no source for the
    // directional USDC fee at settle, so open_challenge must reject it (ZeroStake)
    // BEFORE moving any funds — no Market, no challenged flip.
    let (mut ctx, f) = fixture_with_bond(1);
    assert_eq!(
        required_escrow_usdc(f.bond),
        0,
        "sanity: escrow truncates to 0"
    );

    let ix = open_challenge_ix(
        &ctx,
        f.oracle,
        f.ai_claim,
        f.proposer,
        f.market,
        f.challenger.pubkey(),
        &f.m,
        f.stake_vault,
        f.oracle_pass_kass,
        f.oracle_fail_kass,
        f.kass_dao,
        f.challenger_usdc_src,
        f.nonce,
    );
    let err = ctx.send_many(&cu(ix), &[&f.challenger]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(KassandraError::ZeroStake as u32),
        ),
    );
    assert!(
        ctx.svm.get_account(&f.market).is_none(),
        "zero-escrow reject must not leave a Market account"
    );
    assert_eq!(ctx.ai_claim(f.ai_claim).challenged, 0);
}

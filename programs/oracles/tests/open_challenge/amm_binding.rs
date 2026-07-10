use super::fixture::*;
use super::support::*;

use kassandra_oracles_program::error::KassandraError;
use solana_instruction_error::InstructionError;
use solana_transaction_error::TransactionError;

/// Regression: an AMM that can't bind to this market's conditional (KASS, USDC)
/// mint pair must be REJECTED at open — never recorded on the Market. Recording
/// an unbindable AMM would make `settle_challenge` (which pins to the recorded
/// address) revert forever: `open_challenge_count` would stay > 0, blocking
/// `finalize_oracle` and permanently locking every stake in the oracle.
#[test]
fn open_challenge_unbindable_amm_rejected() {
    let (mut ctx, mut f) = fixture();
    // An AMM owned by the AMM program with the right discriminator but the WRONG
    // (base, quote) mints: passes the owner check the old code relied on, fails
    // the mint-pair binding the fix now enforces at open.
    let bogus = fabricate_amm_account(&mut ctx, Pubkey::new_unique(), Pubkey::new_unique());
    f.m.pass_amm = bogus;

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
            InstructionError::Custom(KassandraError::InvalidAccount as u32),
        ),
    );
    // The brick precondition must be impossible: no Market, claim not flipped.
    assert!(
        ctx.svm.get_account(&f.market).is_none(),
        "an unbindable AMM must not create a Market"
    );
    assert_eq!(
        ctx.ai_claim(f.ai_claim).challenged,
        0,
        "claim must not be flipped to challenged"
    );
}

/// Regression: `pass_amm == fail_amm` must be rejected at open — a challenger
/// cannot collapse the two outcome pools into one it steers. A single pool
/// cannot bind to both outcomes' (KASS, USDC) mint pairs, and the explicit
/// `pass_amm != fail_amm` guard backs it up. (Previously only `settle` caught
/// this — too late, after the Market was already recorded.)
#[test]
fn open_challenge_aliased_amms_rejected() {
    let (mut ctx, mut f) = fixture();
    f.m.fail_amm = f.m.pass_amm;

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
            InstructionError::Custom(KassandraError::InvalidAccount as u32),
        ),
    );
    assert!(
        ctx.svm.get_account(&f.market).is_none(),
        "aliased AMMs must not create a Market"
    );
}

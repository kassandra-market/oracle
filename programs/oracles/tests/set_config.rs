//! Tests for `set_config` (Task F3): the DAO-gated, bounds-checked retune of
//! the `Protocol`-resident governable params.

mod common;
use common::*;

use kassandra_oracles_program::config::{
    CHALLENGE_FAIL_USDC_FEE_DEN, CHALLENGE_FAIL_USDC_FEE_NUM, CHALLENGE_SUCCESS_KASS_FEE_DEN,
    CHALLENGE_SUCCESS_KASS_FEE_NUM, PHASE_WINDOW, THRESHOLD_DEN, THRESHOLD_NUM,
};
use kassandra_oracles_program::error::KassandraError;
use solana_keypair::Keypair;
use solana_signer::Signer;

/// Decode a LiteSVM transaction error into its `Custom(u32)` code, if any.
fn custom_code(res: &litesvm::types::TransactionResult) -> Option<u32> {
    use solana_instruction_error::InstructionError;
    use solana_transaction_error::TransactionError;
    match res {
        Err(meta) => match &meta.err {
            TransactionError::InstructionError(_, InstructionError::Custom(code)) => Some(*code),
            _ => None,
        },
        Ok(_) => None,
    }
}

/// Init the protocol and hand governance off to a fresh, funded `dao` keypair
/// that can then sign `set_config`. Returns `(protocol_pda, dao_keypair)`.
fn governed_ctx() -> (TestCtx, solana_pubkey::Pubkey, Keypair) {
    let mut ctx = TestCtx::new();
    // Use ensure_protocol so the harness records the singleton as initialized;
    // a later create_real_oracle then won't try to re-init it.
    let protocol_pda = ctx.ensure_protocol();

    let dao = Keypair::new();
    ctx.svm.airdrop(&dao.pubkey(), 1_000_000_000).unwrap();
    let (_da, kass_dao) = TestCtx::stand_in_governance(0x33);
    // Record a SIGNABLE keypair as `dao_authority` directly: the Task G1-hardened
    // `set_governance` only accepts the derived (unsignable) Squads vault PDA, so
    // the accept path is driven via the direct-write harness helper.
    ctx.force_governance(dao.pubkey(), kass_dao);
    (ctx, protocol_pda, dao)
}

#[test]
fn dao_sets_config_and_new_oracle_snapshots_new_values() {
    let (mut ctx, protocol_pda, dao) = governed_ctx();

    // Retune two behavioral knobs to non-default values.
    let mut params = ConfigParams::defaults();
    params.phase_window = 7200; // default 3600
    params.threshold_num = 3; // default 2/3 -> 3/4
    params.threshold_den = 4;
    let (_pda, res) = ctx.set_config(&dao, params);
    assert!(
        res.is_ok(),
        "dao_authority set_config should succeed: {res:?}"
    );

    // Protocol reflects the new values.
    let p = ctx.protocol(protocol_pda);
    assert_eq!(p.phase_window, 7200);
    assert_eq!(p.threshold_num, 3);
    assert_eq!(p.threshold_den, 4);
    // Identity/linkage untouched.
    assert_eq!(p.dao_authority, dao.pubkey().to_bytes().into());
    assert_eq!(p.governance_set, 1);

    // A subsequently-created oracle snapshots the NEW values.
    let oracle = ctx.create_real_oracle(2, TWAP_WINDOW);
    let o = ctx.oracle(oracle);
    assert_eq!(
        o.phase_window, 7200,
        "new oracle must snapshot new phase_window"
    );
    assert_eq!(
        o.threshold_num, 3,
        "new oracle must snapshot new threshold_num"
    );
    assert_eq!(
        o.threshold_den, 4,
        "new oracle must snapshot new threshold_den"
    );
}

#[test]
fn new_oracle_snapshots_default_challenge_fees() {
    let (mut ctx, _protocol_pda, _dao) = governed_ctx();

    // A created oracle snapshots the default (1/100) challenge-fee rates.
    let oracle = ctx.create_real_oracle(2, TWAP_WINDOW);
    let o = ctx.oracle(oracle);
    assert_eq!(o.challenge_fail_usdc_fee_num, CHALLENGE_FAIL_USDC_FEE_NUM);
    assert_eq!(o.challenge_fail_usdc_fee_den, CHALLENGE_FAIL_USDC_FEE_DEN);
    assert_eq!(
        o.challenge_success_kass_fee_num,
        CHALLENGE_SUCCESS_KASS_FEE_NUM
    );
    assert_eq!(
        o.challenge_success_kass_fee_den,
        CHALLENGE_SUCCESS_KASS_FEE_DEN
    );
}

#[test]
fn dao_updates_challenge_fees_and_new_oracle_snapshots() {
    let (mut ctx, protocol_pda, dao) = governed_ctx();

    let mut params = ConfigParams::defaults();
    params.challenge_fail_usdc_fee_num = 5; // 5%
    params.challenge_fail_usdc_fee_den = 100;
    params.challenge_success_kass_fee_num = 25; // 2.5%
    params.challenge_success_kass_fee_den = 1000;
    let (_pda, res) = ctx.set_config(&dao, params);
    assert!(
        res.is_ok(),
        "set_config should accept valid fee rates: {res:?}"
    );

    let p = ctx.protocol(protocol_pda);
    assert_eq!(p.challenge_fail_usdc_fee_num, 5);
    assert_eq!(p.challenge_fail_usdc_fee_den, 100);
    assert_eq!(p.challenge_success_kass_fee_num, 25);
    assert_eq!(p.challenge_success_kass_fee_den, 1000);

    // A subsequently-created oracle snapshots the NEW fee rates.
    let oracle = ctx.create_real_oracle(2, TWAP_WINDOW);
    let o = ctx.oracle(oracle);
    assert_eq!(o.challenge_fail_usdc_fee_num, 5);
    assert_eq!(o.challenge_fail_usdc_fee_den, 100);
    assert_eq!(o.challenge_success_kass_fee_num, 25);
    assert_eq!(o.challenge_success_kass_fee_den, 1000);
}

#[test]
fn challenge_fee_zero_denominator_rejected() {
    let (mut ctx, _protocol_pda, dao) = governed_ctx();

    let mut params = ConfigParams::defaults();
    params.challenge_fail_usdc_fee_den = 0;
    let (_pda, res) = ctx.set_config(&dao, params);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidConfig as u32),
        "challenge_fail_usdc_fee_den==0 must be rejected: {res:?}"
    );

    let mut params = ConfigParams::defaults();
    params.challenge_success_kass_fee_den = 0;
    let (_pda, res) = ctx.set_config(&dao, params);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidConfig as u32),
        "challenge_success_kass_fee_den==0 must be rejected: {res:?}"
    );
}

#[test]
fn challenge_fee_over_one_rejected() {
    let (mut ctx, _protocol_pda, dao) = governed_ctx();

    let mut params = ConfigParams::defaults();
    params.challenge_success_kass_fee_num = 101;
    params.challenge_success_kass_fee_den = 100;
    let (_pda, res) = ctx.set_config(&dao, params);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidConfig as u32),
        "a challenge fee >100% must be rejected: {res:?}"
    );
}

#[test]
fn flip_slash_plus_success_fee_over_one_rejected() {
    // JOINT bound (settle_challenge liveness): flip_slash_frac +
    // success_kass_fee_frac must be ≤ 1, else a flip-slashed-then-disqualified
    // proposer underflows settle's carve-out. Each fraction alone is valid here
    // (60% and 50%, both ≤ 1), but their sum (110%) must be rejected.
    let (mut ctx, _protocol_pda, dao) = governed_ctx();

    let mut params = ConfigParams::defaults();
    params.flip_slash_num = 6;
    params.flip_slash_den = 10; // 60%
    params.challenge_success_kass_fee_num = 50;
    params.challenge_success_kass_fee_den = 100; // 50% — sum 110% > 1
    let (_pda, res) = ctx.set_config(&dao, params);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidConfig as u32),
        "flip_slash + success_kass_fee summing to >1 must be rejected: {res:?}"
    );

    // The boundary sum == 1 (exactly) is ACCEPTED (no underflow possible).
    let mut params = ConfigParams::defaults();
    params.flip_slash_num = 1;
    params.flip_slash_den = 2; // 50%
    params.challenge_success_kass_fee_num = 1;
    params.challenge_success_kass_fee_den = 2; // 50% — sum exactly 1
    let (_pda, res) = ctx.set_config(&dao, params);
    assert!(
        res.is_ok(),
        "flip_slash + success_kass_fee == 1 must be accepted: {res:?}"
    );
}

#[test]
fn non_dao_authority_rejected() {
    let (mut ctx, _protocol_pda, _dao) = governed_ctx();

    let stranger = Keypair::new();
    ctx.svm.airdrop(&stranger.pubkey(), 1_000_000_000).unwrap();
    let (_pda, res) = ctx.set_config(&stranger, ConfigParams::defaults());
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::Unauthorized as u32),
        "non-dao_authority set_config must fail Unauthorized: {res:?}"
    );
}

#[test]
fn zero_denominator_rejected() {
    let (mut ctx, _protocol_pda, dao) = governed_ctx();

    // threshold_den == 0
    let mut params = ConfigParams::defaults();
    params.threshold_den = 0;
    let (_pda, res) = ctx.set_config(&dao, params);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidConfig as u32),
        "threshold_den==0 must be rejected: {res:?}"
    );

    // flip_slash_den == 0
    let mut params = ConfigParams::defaults();
    params.flip_slash_den = 0;
    let (_pda, res) = ctx.set_config(&dao, params);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidConfig as u32),
        "flip_slash_den==0 must be rejected: {res:?}"
    );
}

#[test]
fn fraction_over_one_rejected() {
    let (mut ctx, _protocol_pda, dao) = governed_ctx();

    // flip_slash_num > flip_slash_den
    let mut params = ConfigParams::defaults();
    params.flip_slash_num = 3;
    params.flip_slash_den = 2;
    let (_pda, res) = ctx.set_config(&dao, params);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidConfig as u32),
        "flip_slash_num>flip_slash_den must be rejected: {res:?}"
    );
}

#[test]
fn zero_window_rejected() {
    let (mut ctx, _protocol_pda, dao) = governed_ctx();

    let mut params = ConfigParams::defaults();
    params.phase_window = 0;
    let (_pda, res) = ctx.set_config(&dao, params);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidConfig as u32),
        "phase_window==0 must be rejected: {res:?}"
    );
}

#[test]
fn both_reward_weights_zero_rejected() {
    let (mut ctx, _protocol_pda, dao) = governed_ctx();

    let mut params = ConfigParams::defaults();
    params.reward_proposer_weight = 0;
    params.reward_fact_weight = 0;
    let (_pda, res) = ctx.set_config(&dao, params);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidConfig as u32),
        "both reward weights zero must be rejected: {res:?}"
    );
}

#[test]
fn in_flight_oracle_snapshot_unchanged() {
    let (mut ctx, _protocol_pda, dao) = governed_ctx();

    // Create an oracle FIRST, under the default config.
    let oracle = ctx.create_real_oracle(2, TWAP_WINDOW);
    let before = ctx.oracle(oracle);
    assert_eq!(before.phase_window, PHASE_WINDOW);
    assert_eq!(before.threshold_num, THRESHOLD_NUM);

    // Now retune the config.
    let mut params = ConfigParams::defaults();
    params.phase_window = 7200;
    params.threshold_num = 3;
    params.threshold_den = 4;
    let (_pda, res) = ctx.set_config(&dao, params);
    assert!(res.is_ok(), "set_config should succeed: {res:?}");

    // The EXISTING oracle's snapshot is frozen — the goalposts do not move.
    let after = ctx.oracle(oracle);
    assert_eq!(
        after.phase_window, PHASE_WINDOW,
        "in-flight oracle must keep its phase_window"
    );
    assert_eq!(
        after.threshold_num, THRESHOLD_NUM,
        "in-flight oracle must keep its threshold_num"
    );
    assert_eq!(
        after.threshold_den, THRESHOLD_DEN,
        "in-flight oracle must keep its threshold_den"
    );
}

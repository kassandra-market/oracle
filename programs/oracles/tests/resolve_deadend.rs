//! Tests for `resolve_deadend` (Task F4): the DAO-gated resolution of a
//! dead-ended oracle (`InvalidDeadend` → `Resolved` + `resolved_option`).

mod common;
use common::*;

use kassandra_oracles_program::error::KassandraError;
use kassandra_oracles_program::state::Phase;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
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

/// Init the protocol, hand governance off to a fresh funded `dao` keypair (so it
/// can sign `resolve_deadend`), then seed a 2-option oracle and force it into
/// [`Phase::InvalidDeadend`]. Returns `(ctx, dao, oracle)`.
fn deadended_ctx() -> (TestCtx, Keypair, solana_pubkey::Pubkey) {
    let mut ctx = TestCtx::new();
    ctx.ensure_protocol();

    let dao = Keypair::new();
    ctx.svm.airdrop(&dao.pubkey(), 1_000_000_000).unwrap();
    let (_da, kass_dao) = TestCtx::stand_in_governance(0x44);
    // Record a SIGNABLE keypair as `dao_authority` directly: the Task G1-hardened
    // `set_governance` only accepts the derived (unsignable) Squads vault PDA.
    ctx.force_governance(dao.pubkey(), kass_dao);

    // Two distinct options -> options_count == 2; force the dead-end phase
    // directly (the dead-end mechanics are tested elsewhere).
    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: 1_000,
        },
        ProposerSpec {
            option: 1,
            bond: 1_000,
        },
    ]);
    ctx.set_phase(oracle, Phase::InvalidDeadend);

    (ctx, dao, oracle)
}

#[test]
fn dao_resolves_deadend() {
    let (mut ctx, dao, oracle) = deadended_ctx();

    let (_pda, res) = ctx.resolve_deadend(oracle, &dao, 1);
    assert!(res.is_ok(), "dao resolve_deadend should succeed: {res:?}");

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::Resolved.as_u8(), "phase must be Resolved");
    assert_eq!(
        o.resolved_option, 1,
        "resolved_option must equal the option"
    );
}

#[test]
fn non_dao_authority_rejected() {
    let (mut ctx, _dao, oracle) = deadended_ctx();

    let stranger = Keypair::new();
    ctx.svm.airdrop(&stranger.pubkey(), 1_000_000_000).unwrap();
    let (_pda, res) = ctx.resolve_deadend(oracle, &stranger, 1);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::Unauthorized as u32),
        "non-dao_authority resolve_deadend must fail Unauthorized: {res:?}"
    );

    // Oracle untouched.
    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::InvalidDeadend.as_u8());
}

#[test]
fn wrong_phase_rejected() {
    let (mut ctx, dao, oracle) = deadended_ctx();
    // Move the oracle OUT of InvalidDeadend (e.g. already Resolved).
    ctx.set_phase(oracle, Phase::Resolved);

    let (_pda, res) = ctx.resolve_deadend(oracle, &dao, 1);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::WrongPhase as u32),
        "resolve_deadend on a non-InvalidDeadend oracle must fail WrongPhase: {res:?}"
    );
}

#[test]
fn option_out_of_range_rejected() {
    let (mut ctx, dao, oracle) = deadended_ctx();
    // options_count == 2, so option 2 is out of range.
    let (_pda, res) = ctx.resolve_deadend(oracle, &dao, 2);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidOptionsCount as u32),
        "option >= options_count must fail InvalidOptionsCount: {res:?}"
    );

    // Oracle untouched.
    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::InvalidDeadend.as_u8());
}

#[test]
fn idempotent_second_call_rejected() {
    let (mut ctx, dao, oracle) = deadended_ctx();

    let (_pda, res) = ctx.resolve_deadend(oracle, &dao, 0);
    assert!(res.is_ok(), "first resolve_deadend should succeed: {res:?}");
    assert_eq!(ctx.oracle(oracle).phase, Phase::Resolved.as_u8());

    // Second call: phase is now Resolved, so require_phase(InvalidDeadend) fails.
    let (_pda, res2) = ctx.resolve_deadend(oracle, &dao, 1);
    assert_eq!(
        custom_code(&res2),
        Some(KassandraError::WrongPhase as u32),
        "second resolve_deadend must fail WrongPhase: {res2:?}"
    );
    // The first outcome stands.
    assert_eq!(ctx.oracle(oracle).resolved_option, 0);
}

#[test]
fn substituted_protocol_rejected() {
    let (mut ctx, dao, oracle) = deadended_ctx();

    // Pass a bogus (non-canonical) protocol account: load_protocol pins the
    // `[b"protocol"]` PDA address, so a substitute is rejected before the gate.
    let fake_protocol = Pubkey::new_unique();
    let ix = ctx.resolve_deadend_ix(fake_protocol, oracle, dao.pubkey(), 1);
    let res = ctx.send(ix, &[&dao]);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidAccount as u32),
        "a substituted protocol account must be rejected: {res:?}"
    );
}

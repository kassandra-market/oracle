use super::*;
use super::fixture::*;
use super::support::*;

use kassandra_oracles_program::error::KassandraError;
use solana_instruction_error::InstructionError;
use solana_transaction_error::TransactionError;

#[test]
fn open_challenge_twice_is_already_challenged() {
    let (mut ctx, f) = fixture();

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
    ctx.send_many(&cu(ix.clone()), &[&f.challenger])
        .expect("first open_challenge should succeed");

    // Second attempt: the claim is now challenged.
    let err = ctx.send_many(&cu(ix), &[&f.challenger]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(KassandraError::AlreadyChallenged as u32),
        ),
    );
}

#[test]
fn open_challenge_against_disqualified_proposer_fails() {
    let (mut ctx, f) = fixture();
    ctx.set_proposer_disqualified(f.proposer);

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
            InstructionError::Custom(KassandraError::Unauthorized as u32),
        ),
    );
}

#[test]
fn open_challenge_wrong_phase_fails() {
    let (mut ctx, f) = fixture();
    // Knock the oracle out of Challenge.
    ctx.set_phase(f.oracle, Phase::AiClaim);

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
            InstructionError::Custom(KassandraError::WrongPhase as u32),
        ),
    );
}

#[test]
fn open_challenge_after_window_fails() {
    let (mut ctx, f) = fixture();
    ctx.warp(WINDOW + 1);

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
            InstructionError::Custom(KassandraError::WindowClosed as u32),
        ),
    );
}

#[test]
fn open_challenge_question_not_bound_to_oracle_fails() {
    let mut ctx = TestCtx::new();
    ctx.svm.add_program(vault_id(), VAULT_SO).unwrap();
    ctx.svm.add_program(amm_id(), AMM_SO).unwrap();

    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: 1_000_000_000,
        },
        ProposerSpec {
            option: 1,
            bond: 1_000_000_000,
        },
    ]);
    let seeded = ctx.seeded(oracle);
    let nonce = seeded.nonce;
    let stake_vault = seeded.stake_vault;
    let proposer = seeded.proposers[0].pda;
    ctx.set_phase(oracle, Phase::Challenge);
    let ai_claim = seed_ai_claim(&mut ctx, oracle, proposer, 0);

    // Build the market with a DIFFERENT resolver — its question.oracle will not
    // equal the Kassandra oracle PDA, so settle could never resolve it.
    let bogus_resolver = Pubkey::new_unique();
    let (m, oracle_pass_kass, oracle_fail_kass) = setup_market(&mut ctx, bogus_resolver);

    let (market, _) =
        Pubkey::find_program_address(&[b"market", ai_claim.as_ref()], &ctx.program_id);
    let challenger = Keypair::new();
    ctx.svm
        .airdrop(&challenger.pubkey(), 1_000_000_000)
        .unwrap();

    let ix = open_challenge_ix(
        &ctx,
        oracle,
        ai_claim,
        proposer,
        market,
        challenger.pubkey(),
        &m,
        stake_vault,
        oracle_pass_kass,
        oracle_fail_kass,
        // The question.oracle binding fails before escrow pricing, so these
        // escrow accounts are never read (placeholders).
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        nonce,
    );
    let err = ctx.send_many(&cu(ix), &[&challenger]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(KassandraError::InvalidAccount as u32),
        ),
    );
}

#[test]
fn unchallenged_claim_has_no_market_account() {
    // Dormant by default: seeding a disputed oracle + an AiClaim creates NO
    // Market account — zero cost on the uncontested happy path.
    let mut ctx = TestCtx::new();
    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: 1_000_000_000,
        },
        ProposerSpec {
            option: 1,
            bond: 1_000_000_000,
        },
    ]);
    let proposer = ctx.seeded(oracle).proposers[0].pda;
    ctx.set_phase(oracle, Phase::Challenge);
    let ai_claim = seed_ai_claim(&mut ctx, oracle, proposer, 0);

    let (market, _) =
        Pubkey::find_program_address(&[b"market", ai_claim.as_ref()], &ctx.program_id);
    assert!(
        ctx.svm.get_account(&market).is_none(),
        "no challenge means no Market account"
    );
}

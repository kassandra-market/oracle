use super::*;

use kassandra_oracles_program::error::KassandraError;
use solana_instruction_error::InstructionError;
use solana_signer::Signer;
use solana_transaction_error::TransactionError;

#[test]
fn double_claim_fails_account_gone() {
    let mut ctx = TestCtx::new();
    let seed = resolved_full(&mut ctx);
    let p = &seed.proposers[0];
    let recip = p.authority.pubkey();

    let ix = ctx.claim_proposer_ix(
        seed.oracle,
        seed.nonce,
        p.account,
        p.dest_kass,
        seed.stake_vault,
        recip,
    );
    assert!(ctx.send(ix, &[]).is_ok());
    assert!(ctx.is_closed(p.account));

    // Second claim: the account is gone (zeroed/reaped) → owner/type guard fails.
    let ix = ctx.claim_proposer_ix(
        seed.oracle,
        seed.nonce,
        p.account,
        p.dest_kass,
        seed.stake_vault,
        recip,
    );
    let err = ctx.send(ix, &[]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::InvalidAccount as u32),
        ),
    );
}

#[test]
fn dest_owner_mismatch_rejected() {
    let mut ctx = TestCtx::new();
    let seed = resolved_full(&mut ctx);
    let p = &seed.proposers[0];

    // A KASS account owned by a DIFFERENT party cannot receive the payout.
    let attacker = solana_keypair::Keypair::new();
    let bad_dest = ctx.fund_kass(&attacker, 0);

    let ix = ctx.claim_proposer_ix(
        seed.oracle,
        seed.nonce,
        p.account,
        bad_dest,
        seed.stake_vault,
        p.authority.pubkey(),
    );
    let err = ctx.send(ix, &[]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::InvalidAccount as u32),
        ),
    );
}

#[test]
fn submitter_before_voters_rejected() {
    let mut ctx = TestCtx::new();
    let seed = resolved_full(&mut ctx);
    // Fact 0 (agreed) still has two unclaimed approve voters; the submitter's
    // claim must run last (it closes the Fact every voter still reads).
    let s = &seed.facts[0].submitter;
    let ix = ctx.claim_fact_ix(
        seed.oracle,
        seed.nonce,
        s.account,
        s.dest_kass,
        seed.stake_vault,
        s.authority.pubkey(),
    );
    let err = ctx.send(ix, &[]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::VotersOutstanding as u32),
        ),
    );
}

#[test]
fn non_terminal_oracle_rejected() {
    let mut ctx = TestCtx::new();
    // A disputed (FactProposal) oracle is NOT terminal.
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
    let nonce = ctx.seeded(oracle).nonce;
    let stake_vault = ctx.seeded(oracle).stake_vault;
    let pda = ctx.proposers(oracle)[0].pda;
    let authority = ctx.proposers(oracle)[0].authority.insecure_clone();
    let dest = ctx.fund_kass(&authority, 0);

    let ix = ctx.claim_proposer_ix(oracle, nonce, pda, dest, stake_vault, authority.pubkey());
    let err = ctx.send(ix, &[]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::WrongPhase as u32),
        ),
    );
}

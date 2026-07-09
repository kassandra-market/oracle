//! `submit_fact` integration tests.
//!
//! These drive the real deployed program in LiteSVM against a seeded disputed
//! oracle (in `Phase::FactProposal`). They lock in:
//!
//! * Fact PDA seeds `[b"fact", oracle, content_hash]`.
//! * Instruction payload `disc=0 ++ content_hash[32] ++ stake u64 LE ++
//!   uri_len u16 LE ++ uri[uri_len]`.
//! * Account order: oracle, fact, submitter, submitter-KASS, stake-vault,
//!   token-program, system-program.

mod common;
use common::*;

use kassandra_program::{error::KassandraError, instruction::Ix, state::Fact};
use solana_instruction_error::InstructionError;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction_error::TransactionError;

/// Encode a `submit_fact` instruction payload.
fn payload(content_hash: &[u8; 32], stake: u64, uri: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(1 + 32 + 8 + 2 + uri.len());
    data.push(Ix::SubmitFact as u8);
    data.extend_from_slice(content_hash);
    data.extend_from_slice(&stake.to_le_bytes());
    data.extend_from_slice(&(uri.len() as u16).to_le_bytes());
    data.extend_from_slice(uri);
    data
}

/// One seeded-oracle + funded-submitter fixture for a `submit_fact` test.
struct Fixture {
    ctx: TestCtx,
    oracle: Pubkey,
    vault: Pubkey,
    submitter: Keypair,
    submitter_kass: Pubkey,
    fact: Pubkey,
    content_hash: [u8; 32],
}

/// Seed an oracle in `FactProposal` and fund a fresh submitter with KASS.
fn fixture(stake: u64) -> Fixture {
    let mut ctx = TestCtx::new();
    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: 1_000,
        },
        ProposerSpec {
            option: 1,
            bond: 2_000,
        },
    ]);
    let vault = ctx.seeded(oracle).stake_vault;

    let submitter = Keypair::new();
    ctx.svm.airdrop(&submitter.pubkey(), 1_000_000_000).unwrap();
    let submitter_kass = ctx.fund_kass(&submitter, stake * 8);

    let content_hash = [0x42u8; 32];
    let (fact, _) = TestCtx::fact_pda(&ctx.program_id, &oracle, &content_hash);
    Fixture {
        ctx,
        oracle,
        vault,
        submitter,
        submitter_kass,
        fact,
        content_hash,
    }
}

#[test]
fn submit_fact_happy_path() {
    let stake = 500u64;
    let Fixture {
        mut ctx,
        oracle,
        vault,
        submitter,
        submitter_kass,
        fact,
        content_hash,
    } = fixture(stake);

    let vault_before = ctx.token_balance(vault);
    let stake_before = ctx.oracle(oracle).total_oracle_stake;

    let uri = b"ipfs://abc";
    let ix = submit_fact_ix(
        &ctx,
        oracle,
        fact,
        submitter.pubkey(),
        submitter_kass,
        vault,
        payload(&content_hash, stake, uri),
    );
    ctx.send(ix, &[&submitter])
        .expect("submit_fact should succeed");

    // Fact account materialized with the right contents.
    let f: Fact = ctx.fact(fact);
    assert_eq!(f.content_hash, content_hash);
    assert_eq!(f.stake, stake);
    assert_eq!(f.proposer, submitter.pubkey().to_bytes().into());
    assert_eq!(f.oracle, oracle.to_bytes().into());
    assert_eq!(f.uri_len as usize, uri.len());
    assert_eq!(&f.uri[..uri.len()], uri);
    assert_eq!(f.approve_stake, 0);
    assert_eq!(f.duplicate_stake, 0);
    assert_eq!(f.agreed, 0);
    assert_eq!(f.settled, 0);

    // Stake moved into the vault, oracle bookkeeping bumped.
    assert_eq!(ctx.token_balance(vault), vault_before + stake);
    let o = ctx.oracle(oracle);
    assert_eq!(o.fact_count, 1);
    assert_eq!(o.total_oracle_stake, stake_before + stake);
}

#[test]
fn submit_fact_duplicate_content_hash_fails() {
    let stake = 500u64;
    let Fixture {
        mut ctx,
        oracle,
        vault,
        submitter,
        submitter_kass,
        fact,
        content_hash,
    } = fixture(stake);

    let ix1 = submit_fact_ix(
        &ctx,
        oracle,
        fact,
        submitter.pubkey(),
        submitter_kass,
        vault,
        payload(&content_hash, stake, b"first"),
    );
    ctx.send(ix1, &[&submitter])
        .expect("first submit should succeed");

    let ix2 = submit_fact_ix(
        &ctx,
        oracle,
        fact,
        submitter.pubkey(),
        submitter_kass,
        vault,
        payload(&content_hash, stake, b"second"),
    );
    let err = ctx.send(ix2, &[&submitter]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::DuplicateFact as u32),
        ),
    );
}

#[test]
fn submit_fact_wrong_phase_fails() {
    use kassandra_program::state::Phase;
    let stake = 500u64;
    let Fixture {
        mut ctx,
        oracle,
        vault,
        submitter,
        submitter_kass,
        fact,
        content_hash,
    } = fixture(stake);

    // Move the oracle out of FactProposal.
    ctx.set_phase(oracle, Phase::FactVoting);

    let ix = submit_fact_ix(
        &ctx,
        oracle,
        fact,
        submitter.pubkey(),
        submitter_kass,
        vault,
        payload(&content_hash, stake, b"x"),
    );
    let err = ctx.send(ix, &[&submitter]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::WrongPhase as u32),
        ),
    );
}

#[test]
fn submit_fact_after_window_fails() {
    let stake = 500u64;
    let Fixture {
        mut ctx,
        oracle,
        vault,
        submitter,
        submitter_kass,
        fact,
        content_hash,
    } = fixture(stake);

    // Cross phase_ends_at.
    ctx.warp(WINDOW + 1);

    let ix = submit_fact_ix(
        &ctx,
        oracle,
        fact,
        submitter.pubkey(),
        submitter_kass,
        vault,
        payload(&content_hash, stake, b"x"),
    );
    let err = ctx.send(ix, &[&submitter]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::WindowClosed as u32),
        ),
    );
}

#[test]
fn submit_fact_zero_stake_ok_when_floor_zero() {
    // Bootstrapping: with a 0 floor (genesis / low activity) a 0-stake fact is
    // accepted — participation needs no premined KASS.
    let stake = 500u64;
    let Fixture {
        mut ctx,
        oracle,
        vault,
        submitter,
        submitter_kass,
        fact,
        content_hash,
    } = fixture(stake);
    assert_eq!(
        ctx.oracle(oracle).min_stake,
        0,
        "genesis oracle floor must be 0"
    );

    let ix = submit_fact_ix(
        &ctx,
        oracle,
        fact,
        submitter.pubkey(),
        submitter_kass,
        vault,
        payload(&content_hash, 0, b"x"),
    );
    assert!(
        ctx.send(ix, &[&submitter]).is_ok(),
        "0-stake fact must succeed when floor is 0"
    );
}

#[test]
fn submit_fact_below_floor_fails() {
    // Once activity raises the floor, a stake below it is rejected.
    let stake = 500u64;
    let Fixture {
        mut ctx,
        oracle,
        vault,
        submitter,
        submitter_kass,
        fact,
        content_hash,
    } = fixture(stake);
    ctx.set_oracle_min_stake(oracle, 1_000);

    let ix = submit_fact_ix(
        &ctx,
        oracle,
        fact,
        submitter.pubkey(),
        submitter_kass,
        vault,
        payload(&content_hash, 999, b"x"),
    );
    let err = ctx.send(ix, &[&submitter]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::BelowMinStake as u32),
        ),
    );
}

#[test]
fn submit_fact_oracle_type_confusion_fails() {
    use bytemuck::Zeroable;
    use kassandra_program::state::{AccountType, Oracle};

    let stake = 500u64;
    let Fixture {
        mut ctx,
        oracle,
        vault,
        submitter,
        submitter_kass,
        fact,
        content_hash,
    } = fixture(stake);

    // Fabricate a program-owned, Oracle-sized account whose tag is NOT Oracle
    // (here: Fact) and feed it into the oracle slot. The load_oracle guard must
    // reject it on the account_type check.
    let mut fake = Oracle::zeroed();
    fake.account_type = AccountType::Fact.as_u8();
    let not_an_oracle = ctx.seed_program_account(bytemuck::bytes_of(&fake).to_vec());
    // Keep using the real oracle's vault so the failure is unambiguously the tag.
    let _ = oracle;

    let ix = submit_fact_ix(
        &ctx,
        not_an_oracle,
        fact,
        submitter.pubkey(),
        submitter_kass,
        vault,
        payload(&content_hash, stake, b"x"),
    );
    let err = ctx.send(ix, &[&submitter]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::InvalidAccount as u32),
        ),
    );
}

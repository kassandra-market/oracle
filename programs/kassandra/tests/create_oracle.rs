//! Tests for `create_oracle` (Task H1): an oracle in [`Phase::Proposal`] with a
//! future deadline plus its program-created stake vault. No fee yet (Task H2).

mod common;
use common::*;

use kassandra_program::error::KassandraError;
use kassandra_program::state::{AccountType, Phase};
use solana_pubkey::Pubkey;
use solana_signer::Signer;

/// Proposal window added to `deadline` to compute `phase_ends_at` (mirrors
/// `config::PROPOSAL_WINDOW`).
const PROPOSAL_WINDOW: i64 = 3600;

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

#[test]
fn create_oracle_happy_path() {
    let mut ctx = TestCtx::new();
    let (_p, res) = ctx.init_protocol();
    assert!(res.is_ok(), "init_protocol should succeed: {res:?}");

    let deadline = ctx.now() + 1_000;
    let twap_window = 600;
    // Emission is ON by default: create mints `reward_emission` into the vault.
    let emission = ctx.expected_creation_emission();
    assert!(emission > 0, "default config emits at genesis supply");
    let (oracle_pda, res) = ctx.create_oracle(7, 3, deadline, twap_window);
    assert!(res.is_ok(), "create_oracle should succeed: {res:?}");

    let o = ctx.oracle(oracle_pda);
    assert_eq!(o.account_type, AccountType::Oracle.as_u8());
    assert_eq!(o.phase, Phase::Proposal.as_u8());
    assert_eq!(o.creator, ctx.payer.pubkey().to_bytes().into());
    assert_eq!(o.kass_mint, ctx.kass_mint.to_bytes().into());
    assert_eq!(o.usdc_mint, ctx.usdc_mint.to_bytes().into());
    assert_eq!(o.deadline, deadline);
    assert_eq!(o.phase_ends_at, deadline + PROPOSAL_WINDOW);
    assert_eq!(o.twap_window, twap_window);
    assert_eq!(o.options_count, 3);
    assert_eq!(o.proposer_count, 0);
    assert_eq!(o.surviving_count, 0);
    assert_eq!(o.fact_count, 0);
    assert_eq!(o.total_oracle_stake, 0);
    assert_eq!(o.bond_pool, 0);
    assert_eq!(o.dispute_bond_total, 0);
    assert_eq!(o.settled_count, 0);
    assert_eq!(o.ai_finalized_count, 0);
    assert_eq!(o.open_challenge_count, 0);
    // The pre-minted emission is recorded on the oracle and sits in the vault.
    assert_eq!(o.reward_emission, emission);

    // The stake vault is a KASS token account, authority == oracle PDA, holding
    // exactly the minted emission (no proposer bonds yet).
    let (vault_pda, _) = TestCtx::stake_vault_pda(&ctx.program_id, &oracle_pda);
    assert_eq!(o.stake_vault, vault_pda.to_bytes().into());
    let (mint, owner, amount) = ctx.token_account(vault_pda);
    assert_eq!(mint, ctx.kass_mint.to_bytes());
    assert_eq!(owner, oracle_pda.to_bytes());
    assert_eq!(amount, emission);
}

#[test]
fn options_count_below_two_fails() {
    let mut ctx = TestCtx::new();
    let _ = ctx.init_protocol();
    let deadline = ctx.now() + 1_000;
    let (_o, res) = ctx.create_oracle(1, 1, deadline, 600);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidOptionsCount as u32),
        "options_count < 2 must fail InvalidOptionsCount: {res:?}"
    );
}

#[test]
fn deadline_in_past_fails() {
    let mut ctx = TestCtx::new();
    let _ = ctx.init_protocol();
    let deadline = ctx.now() - 1;
    let (_o, res) = ctx.create_oracle(1, 2, deadline, 600);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidDeadline as u32),
        "deadline < now must fail InvalidDeadline: {res:?}"
    );
}

#[test]
fn nonpositive_twap_window_fails() {
    let mut ctx = TestCtx::new();
    let _ = ctx.init_protocol();
    let deadline = ctx.now() + 1_000;
    let (_o, res) = ctx.create_oracle(1, 2, deadline, 0);
    // twap_window <= 0 is a payload sanity failure → InvalidInstructionData.
    use solana_instruction_error::InstructionError;
    use solana_transaction_error::TransactionError;
    assert!(
        matches!(
            &res,
            Err(meta) if matches!(
                meta.err,
                TransactionError::InstructionError(_, InstructionError::InvalidInstructionData)
            )
        ),
        "twap_window <= 0 must fail InvalidInstructionData: {res:?}"
    );
}

#[test]
fn mint_mismatch_vs_protocol_fails() {
    let mut ctx = TestCtx::new();
    let _ = ctx.init_protocol();
    let deadline = ctx.now() + 1_000;

    // A bogus KASS mint not equal to the protocol's canonical mint.
    let fake_kass = Pubkey::new_unique();
    let (oracle_pda, _) = TestCtx::oracle_pda(&ctx.program_id, 1);
    let ix = ctx.create_oracle_ix(1, 2, deadline, 600, oracle_pda, fake_kass, ctx.usdc_mint);
    let res = ctx.send(ix, &[]);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidAccount as u32),
        "spoofed KASS mint must fail InvalidAccount: {res:?}"
    );

    // Likewise a bogus USDC mint.
    let fake_usdc = Pubkey::new_unique();
    let ix = ctx.create_oracle_ix(
        2,
        2,
        deadline,
        600,
        TestCtx::oracle_pda(&ctx.program_id, 2).0,
        ctx.kass_mint,
        fake_usdc,
    );
    let res = ctx.send(ix, &[]);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidAccount as u32),
        "spoofed USDC mint must fail InvalidAccount: {res:?}"
    );
}

#[test]
fn duplicate_oracle_same_nonce_fails() {
    let mut ctx = TestCtx::new();
    let _ = ctx.init_protocol();
    let deadline = ctx.now() + 1_000;
    let (_o, res) = ctx.create_oracle(5, 2, deadline, 600);
    assert!(res.is_ok(), "first create should succeed: {res:?}");

    let (_o2, res2) = ctx.create_oracle(5, 2, deadline, 600);
    assert_eq!(
        custom_code(&res2),
        Some(KassandraError::InvalidAccount as u32),
        "duplicate nonce must fail InvalidAccount: {res2:?}"
    );
}

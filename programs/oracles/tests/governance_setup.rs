//! Tests for `set_governance` (Task F1): the one-time DAO-linkage handoff.

mod common;
use common::*;

use kassandra_oracles_program::error::KassandraError;
use kassandra_oracles_program::state::AccountType;
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

#[test]
fn admin_sets_governance_records_linkage_and_defaults() {
    let mut ctx = TestCtx::new();
    let (protocol_pda, res) = ctx.init_protocol();
    assert!(res.is_ok(), "init_protocol should succeed: {res:?}");

    // Pre-handoff: linkage unset, monetary params == config defaults.
    let p0 = ctx.protocol(protocol_pda);
    assert_eq!(p0.governance_set, 0);
    assert_eq!(p0.dao_authority, [0u8; 32].into());
    assert_eq!(p0.kass_dao, [0u8; 32].into());
    assert_eq!(p0.emission_num, kassandra_oracles_program::config::EMISSION_NUM);
    assert_eq!(p0.emission_den, kassandra_oracles_program::config::EMISSION_DEN);
    assert_eq!(
        p0.total_supply_cap,
        kassandra_oracles_program::config::TOTAL_SUPPLY_CAP
    );
    assert_eq!(
        p0.fee_ema_halflife,
        kassandra_oracles_program::config::FEE_EMA_HALFLIFE_SECS
    );
    assert_eq!(
        p0.fee_per_ema_unit,
        kassandra_oracles_program::config::FEE_PER_EMA_UNIT
    );
    assert_eq!(
        p0.fee_ema_increment,
        kassandra_oracles_program::config::FEE_EMA_INCREMENT
    );

    // Task G1: the linkage is validated against a REAL futarchy `Dao` account +
    // its derived Squads vault. `fabricate_dao_and_vault` stands up a
    // futarchy-owned `Dao` (valid discriminator) and returns the vault PDA the
    // handoff requires as `dao_authority`.
    let (kass_dao, dao_authority) = ctx.fabricate_dao_and_vault();
    let payer = ctx.payer.insecure_clone();
    let (_pda, res) = ctx.set_governance(&payer, dao_authority, kass_dao);
    assert!(res.is_ok(), "admin set_governance should succeed: {res:?}");

    let p = ctx.protocol(protocol_pda);
    assert_eq!(p.account_type, AccountType::Protocol.as_u8());
    assert_eq!(p.governance_set, 1);
    assert_eq!(p.dao_authority, dao_authority.to_bytes().into());
    assert_eq!(p.kass_dao, kass_dao.to_bytes().into());
    // Monetary params untouched by the handoff.
    assert_eq!(p.emission_den, kassandra_oracles_program::config::EMISSION_DEN);
    assert_eq!(
        p.fee_per_ema_unit,
        kassandra_oracles_program::config::FEE_PER_EMA_UNIT
    );
}

#[test]
fn non_admin_cannot_set_governance() {
    let mut ctx = TestCtx::new();
    let (_pda, res) = ctx.init_protocol();
    assert!(res.is_ok());

    let stranger = Keypair::new();
    ctx.svm.airdrop(&stranger.pubkey(), 1_000_000_000).unwrap();
    let (dao_authority, kass_dao) = TestCtx::stand_in_governance(0xBB);
    let (_pda, res) = ctx.set_governance(&stranger, dao_authority, kass_dao);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::Unauthorized as u32),
        "non-admin set_governance must fail Unauthorized: {res:?}"
    );
}

#[test]
fn handoff_is_one_shot_admin_rejected_after_handoff() {
    let mut ctx = TestCtx::new();
    let (protocol_pda, res) = ctx.init_protocol();
    assert!(res.is_ok());

    // Admin performs the one-time handoff with the REAL validated linkage (a
    // futarchy `Dao` + its derived Squads vault as `dao_authority`).
    let (kass_dao, vault) = ctx.fabricate_dao_and_vault();
    let payer = ctx.payer.insecure_clone();
    let (_pda, res) = ctx.set_governance(&payer, vault, kass_dao);
    assert!(res.is_ok(), "admin handoff should succeed: {res:?}");
    assert_eq!(ctx.protocol(protocol_pda).governance_set, 1);

    // The OLD admin can no longer change the linkage (the one-shot gate fires
    // BEFORE any linkage validation, so a fresh real Dao is unnecessary here).
    let (kass_dao2, vault2) = ctx.fabricate_dao_and_vault();
    let (_pda, res) = ctx.set_governance(&payer, vault2, kass_dao2);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::GovernanceAlreadySet as u32),
        "post-handoff admin must be rejected GovernanceAlreadySet: {res:?}"
    );

    // Rotation semantics (post-handoff, ONLY the current `dao_authority` may
    // rotate) remain in force, but the recorded `dao_authority` is now the Squads
    // vault PDA — which NO test keypair can sign for (only Squads'
    // vault_transaction_execute can, in production). So the rotation accept-path
    // is unsignable from a test, exactly like the `set_config`/`resolve_deadend`
    // gates (see governance_seam.rs). The linkage is therefore unchanged.
    let p = ctx.protocol(protocol_pda);
    assert_eq!(p.dao_authority, vault.to_bytes().into());
    assert_eq!(p.kass_dao, kass_dao.to_bytes().into());
}

// ─────────────────────────────────────────────────────────────────────────────
// Task G1: the hardened handoff VALIDATES the real Squads-vault / futarchy-DAO
// linkage instead of trusting the payload. Reject arms:
//   - a `kass_dao` not owned by the futarchy program  -> InvalidFutarchyDao
//   - a futarchy-owned `kass_dao` with a bad discriminator -> InvalidFutarchyDao
//   - a `dao_authority` that is NOT the derived Squads vault -> DaoAuthorityMismatch
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn rejects_non_futarchy_owned_kass_dao() {
    let mut ctx = TestCtx::new();
    let (_pda, res) = ctx.init_protocol();
    assert!(res.is_ok());

    // A `Dao`-shaped blob (valid discriminator) but owned by SOME OTHER program.
    let kass_dao = Pubkey::new_unique();
    let foreign_owner = Pubkey::new_unique();
    ctx.fabricate_owned_account(
        kass_dao,
        foreign_owner,
        common::build_dao_blob(1, 1_000_000, 0, 0),
    );
    let dao_authority = TestCtx::squads_vault_for_dao(&kass_dao);

    let payer = ctx.payer.insecure_clone();
    let (_pda, res) = ctx.set_governance(&payer, dao_authority, kass_dao);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidFutarchyDao as u32),
        "a kass_dao not owned by the futarchy program must be rejected: {res:?}"
    );
}

#[test]
fn rejects_kass_dao_with_bad_discriminator() {
    let mut ctx = TestCtx::new();
    let (_pda, res) = ctx.init_protocol();
    assert!(res.is_ok());

    // Owned by the futarchy program, but the first 8 bytes are NOT the `Dao`
    // Anchor discriminator.
    let kass_dao = Pubkey::new_unique();
    let owner = Pubkey::new_from_array(kassandra_oracles_program::cpi::metadao_v06::FUTARCHY_ID.to_bytes());
    let mut blob = common::build_dao_blob(1, 1_000_000, 0, 0);
    blob[..8].copy_from_slice(&[0xDE; 8]); // clobber the discriminator
    ctx.fabricate_owned_account(kass_dao, owner, blob);
    let dao_authority = TestCtx::squads_vault_for_dao(&kass_dao);

    let payer = ctx.payer.insecure_clone();
    let (_pda, res) = ctx.set_governance(&payer, dao_authority, kass_dao);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidFutarchyDao as u32),
        "a kass_dao with the wrong Anchor discriminator must be rejected: {res:?}"
    );
}

#[test]
fn rejects_dao_authority_that_is_not_derived_vault() {
    let mut ctx = TestCtx::new();
    let (_pda, res) = ctx.init_protocol();
    assert!(res.is_ok());

    // A REAL futarchy `Dao` (owner + discriminator pass), but a `dao_authority`
    // that is NOT the Squads vault derived for it.
    let (kass_dao, _correct_vault) = ctx.fabricate_dao_and_vault();
    let wrong_authority = Pubkey::new_unique();

    let payer = ctx.payer.insecure_clone();
    let (_pda, res) = ctx.set_governance(&payer, wrong_authority, kass_dao);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::DaoAuthorityMismatch as u32),
        "a dao_authority that is not the derived Squads vault must be rejected: {res:?}"
    );
}

#[test]
fn zero_linkage_keys_rejected() {
    let mut ctx = TestCtx::new();
    let (_pda, res) = ctx.init_protocol();
    assert!(res.is_ok());
    let payer = ctx.payer.insecure_clone();

    // Zero dao_authority.
    let (_pda, res) = ctx.set_governance(&payer, Pubkey::default(), Pubkey::new_unique());
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidAccount as u32),
        "zero dao_authority must be rejected: {res:?}"
    );

    // Zero kass_dao.
    let (_pda, res) = ctx.set_governance(&payer, Pubkey::new_unique(), Pubkey::default());
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidAccount as u32),
        "zero kass_dao must be rejected: {res:?}"
    );
}

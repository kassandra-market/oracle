//! Task F6 — v0.6 futarchy / Squads-v4 governance EXECUTION SEAM (Kassandra side).
//!
//! F6 was NARROWED to the documented seam fallback (see the plan's "Governance
//! seam" section + the F6 delta): driving the full v0.6 proposal → conditional
//! pass/fail market (v0.6 vault + Meteora DAMM v2) → trade → finalize → Squads
//! vault execute lifecycle inside LiteSVM is intractable. Instead we prove the
//! seam rigorously and document the rest as deferred.
//!
//! ## What this file proves (Kassandra side of the seam)
//! The DAO execution authority is a **Squads v4 multisig vault PDA** (F0 finding
//! #1). Kassandra records that vault PDA as `Protocol.dao_authority` and gates
//! `set_config` / `resolve_deadend` on it via `assert_dao_authority`. Here we:
//!
//! 1. DERIVE the Squads v4 vault PDA from the DOCUMENTED seeds
//!    (`[b"multisig", b"multisig", dao]` → multisig, then
//!    `[b"multisig", multisig, b"vault", [0]]` → vault, under `SQDS4…`),
//! 2. record THAT derived vault PDA as `Protocol.dao_authority` via
//!    `set_governance`, and
//! 3. show every privileged instruction (`set_config`, `resolve_deadend`)
//!    REJECTS a different signer with `Unauthorized`.
//!
//! ## Honest note on signing as the vault PDA
//! A LiteSVM test (like any client) CANNOT fabricate a signature for a PDA — only
//! the owning program can `invoke_signed` it. So once `dao_authority` is a Squads
//! vault PDA, NO test keypair can satisfy the gate; the accept-path is exercised
//! in F3/F4 with an ordinary recorded keypair (and re-anchored minimally below).
//! In PRODUCTION the vault-PDA signature is produced by Squads'
//! `vault_transaction_execute` CPI (whose discriminator + dispatch is validated
//! against the real `squads_v4.so` in
//! `metadao_v06_cpi::squads_vault_transaction_execute_discriminator_recognized`).
//! The composition — futarchy proposal passes → Squads executes the staged
//! `set_config`/`resolve_deadend` CPI signed by the vault PDA → Kassandra's gate
//! accepts because that PDA == `Protocol.dao_authority` — is the deferred
//! full-integration (a real-validator / surfpool follow-up; see the F6 delta).

mod common;
use common::*;

use kassandra_oracles_program::error::KassandraError;
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

// ─────────────────────────────────────────────────────────────────────────────
// 1. The realistic authority: the DERIVED Squads v4 vault PDA recorded as
//    `dao_authority` through the Task G1-hardened handoff (a REAL futarchy `Dao`
//    + its derived vault — the accept-real path). `set_config` then rejects
//    EVERY signer a test can produce (the admin/payer and an unrelated keypair),
//    with `Unauthorized` — proving the gate accepts ONLY the recorded vault PDA,
//    which in production only Squads' vault_transaction_execute can sign for.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn set_config_gate_accepts_only_recorded_squads_vault_pda() {
    let mut ctx = TestCtx::new();
    ctx.ensure_protocol();

    // Stand up a REAL futarchy `Dao` (valid owner + discriminator) and its
    // derived Squads vault — the exact linkage the hardened handoff requires.
    let (kass_dao, vault_pda) = ctx.fabricate_dao_and_vault();

    // Admin (payer) hands governance off through the real (validated) handoff,
    // recording the vault PDA as the DAO execution authority.
    let payer = ctx.payer.insecure_clone();
    let (protocol_pda, res) = ctx.set_governance(&payer, vault_pda, kass_dao);
    assert!(res.is_ok(), "handoff should succeed: {res:?}");
    assert_eq!(
        ctx.protocol(protocol_pda).dao_authority,
        vault_pda.to_bytes().into(),
        "dao_authority must be the derived Squads vault PDA"
    );

    let params = ConfigParams::defaults();

    // (a) The admin/payer is NOT the vault PDA → Unauthorized.
    let (_pda, res) = ctx.set_config(&payer, params);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::Unauthorized as u32),
        "admin must not be able to set_config once governance is the vault PDA: {res:?}"
    );

    // (b) An unrelated funded keypair is NOT the vault PDA → Unauthorized.
    let stranger = Keypair::new();
    ctx.svm.airdrop(&stranger.pubkey(), 1_000_000_000).unwrap();
    let (_pda, res) = ctx.set_config(&stranger, params);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::Unauthorized as u32),
        "a stranger must not be able to set_config: {res:?}"
    );

    // The config is unchanged (no signer could pass the gate).
    let p = ctx.protocol(protocol_pda);
    assert_eq!(p.dao_authority, vault_pda.to_bytes().into());
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Same seam for `resolve_deadend`: with the derived vault PDA recorded as
//    `dao_authority`, a dead-ended oracle cannot be resolved by any test signer.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn resolve_deadend_gate_rejects_non_vault_signer() {
    let mut ctx = TestCtx::new();
    ctx.ensure_protocol();

    let (kass_dao, vault_pda) = ctx.fabricate_dao_and_vault();

    let payer = ctx.payer.insecure_clone();
    let (_p, res) = ctx.set_governance(&payer, vault_pda, kass_dao);
    assert!(res.is_ok(), "handoff should succeed: {res:?}");

    // Stand up a dead-ended oracle.
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
    ctx.set_phase(oracle, kassandra_oracles_program::state::Phase::InvalidDeadend);

    // A stranger (and the admin) cannot resolve it — only the recorded vault PDA.
    let stranger = Keypair::new();
    ctx.svm.airdrop(&stranger.pubkey(), 1_000_000_000).unwrap();
    let (_p, res) = ctx.resolve_deadend(oracle, &stranger, 0);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::Unauthorized as u32),
        "a stranger must not resolve a dead-end once governance is the vault PDA: {res:?}"
    );
    let (_p, res) = ctx.resolve_deadend(oracle, &payer, 0);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::Unauthorized as u32),
        "admin must not resolve a dead-end once governance is the vault PDA: {res:?}"
    );

    // The oracle is untouched: still dead-ended, not resolved.
    assert_eq!(
        ctx.oracle(oracle).phase,
        kassandra_oracles_program::state::Phase::InvalidDeadend.as_u8(),
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. Accept-path anchor (minimal — fully covered in F3/F4): when the RECORDED
//    `dao_authority` is an ordinary, signable keypair, the very same gate ACCEPTS
//    it. This proves the gate is an identity check (accept iff signer == recorded
//    authority), not a blanket reject — so production's vault-PDA signature
//    (produced by Squads' vault_transaction_execute) will likewise be accepted.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn gate_accepts_recorded_authority_when_signable() {
    let mut ctx = TestCtx::new();
    ctx.ensure_protocol();

    let dao_kp = Keypair::new();
    ctx.svm.airdrop(&dao_kp.pubkey(), 1_000_000_000).unwrap();
    let kass_dao = Pubkey::new_unique();

    // A SIGNABLE keypair as `dao_authority` is recorded directly: the Task
    // G1-hardened handoff only accepts the derived (unsignable) Squads vault PDA,
    // so the accept path uses the direct-write harness helper.
    let protocol_pda = ctx.force_governance(dao_kp.pubkey(), kass_dao);

    // The recorded authority signs → accepted.
    let mut params = ConfigParams::defaults();
    params.phase_window = 7200;
    let (_pda, res) = ctx.set_config(&dao_kp, params);
    assert!(
        res.is_ok(),
        "recorded authority set_config should succeed: {res:?}"
    );
    assert_eq!(ctx.protocol(protocol_pda).phase_window, 7200);
}

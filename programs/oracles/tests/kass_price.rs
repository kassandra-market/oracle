//! Tests for `kass_price` (Task F5): the governance-anchored KASS/USDC spot TWAP
//! read from the canonical futarchy `Dao` account (`Protocol.kass_dao`).
//!
//! A live futarchy `Dao` requires the full F6 setup (Squads v4 + mints + a driven
//! proposal), so — mirroring F0's `futarchy_spot_twap` test — these exercise the
//! read against a HAND-BUILT `Dao`/spot-oracle blob placed in an account owned by
//! the futarchy program, with its key recorded as `Protocol.kass_dao` via
//! `set_governance`. The read source is the `Dao` account ITSELF (the embedded
//! spot oracle at fixed offsets), so there is no separate pool account to bind.

mod common;
use common::*;

use kassandra_oracles_program::cpi::metadao_v06 as md6;
use kassandra_oracles_program::error::KassandraError;
use solana_pubkey::Pubkey;

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

/// The futarchy program id as a `solana_sdk::Pubkey` (the owner a real `Dao`
/// account carries).
fn futarchy_owner() -> Pubkey {
    Pubkey::new_from_array(md6::FUTARCHY_ID.to_bytes())
}
/// `get_twap` reference math (independent of the program): the expected value a
/// correct read must return.
fn expected_twap(aggregator: u128, last_updated: i64, created_at: i64, start_delay: u32) -> u128 {
    aggregator / (last_updated - (created_at + start_delay as i64)) as u128
}

/// Extract the 16-byte little-endian `u128` return data from a successful tx.
fn return_u128(res: &litesvm::types::TransactionResult) -> u128 {
    let meta = res.as_ref().expect("tx should succeed");
    let bytes: [u8; 16] = meta
        .return_data
        .data
        .as_slice()
        .try_into()
        .expect("return data should be 16 bytes (u128 LE)");
    u128::from_le_bytes(bytes)
}

/// Init the protocol and hand governance off (payer is admin), recording
/// `kass_dao` as the given DAO account key. Returns the Protocol PDA.
fn governed_ctx(ctx: &mut TestCtx, kass_dao: Pubkey) -> Pubkey {
    ctx.ensure_protocol();
    // The kass_price read only consults `kass_dao`; record the linkage directly
    // (force_governance) rather than via the Task G1-hardened handoff, which
    // would require a matching derived Squads vault for `dao_authority`.
    let (dao_authority, _) = TestCtx::stand_in_governance(0x55);
    ctx.force_governance(dao_authority, kass_dao)
}

#[test]
fn reads_spot_twap_from_kass_dao() {
    let mut ctx = TestCtx::new();

    let aggregator: u128 = 4_200_000_000_000_000;
    let last_updated: i64 = 1_000_000;
    let created_at: i64 = 100_000;
    let start_delay: u32 = 0;

    // Fabricate the canonical futarchy Dao account and bless it as `kass_dao`.
    let kass_dao = Pubkey::new_unique();
    ctx.fabricate_owned_account(
        kass_dao,
        futarchy_owner(),
        build_dao_blob(aggregator, last_updated, created_at, start_delay),
    );
    let protocol = governed_ctx(&mut ctx, kass_dao);

    let ix = ctx.kass_price_ix(protocol, kass_dao);
    let res = ctx.send(ix, &[]);
    assert!(res.is_ok(), "kass_price should succeed: {res:?}");

    assert_eq!(
        return_u128(&res),
        expected_twap(aggregator, last_updated, created_at, start_delay),
        "returned TWAP must match the independent get_twap computation",
    );
}

#[test]
fn wrong_account_rejected() {
    let mut ctx = TestCtx::new();

    // Bless one DAO account...
    let kass_dao = Pubkey::new_unique();
    ctx.fabricate_owned_account(
        kass_dao,
        futarchy_owner(),
        build_dao_blob(4_200_000_000_000_000, 1_000_000, 100_000, 0),
    );
    let protocol = governed_ctx(&mut ctx, kass_dao);

    // ...but pass a DIFFERENT (also futarchy-owned, valid) account. The
    // governance anchor (`key == Protocol.kass_dao`) must reject it.
    let impostor = Pubkey::new_unique();
    ctx.fabricate_owned_account(
        impostor,
        futarchy_owner(),
        build_dao_blob(9_999_999_999_999_999, 1_000_000, 100_000, 0),
    );

    let ix = ctx.kass_price_ix(protocol, impostor);
    let res = ctx.send(ix, &[]);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidAccount as u32),
        "an account != Protocol.kass_dao must be rejected: {res:?}",
    );
}

#[test]
fn non_futarchy_owner_rejected() {
    let mut ctx = TestCtx::new();

    // The blessed `kass_dao` carries a valid oracle blob, but is owned by the
    // WRONG program (the Kassandra program here). The owner anchor must reject.
    let kass_dao = Pubkey::new_unique();
    let wrong_owner = Pubkey::new_from_array(kassandra_oracles_program::ID.to_bytes());
    ctx.fabricate_owned_account(
        kass_dao,
        wrong_owner,
        build_dao_blob(4_200_000_000_000_000, 1_000_000, 100_000, 0),
    );
    let protocol = governed_ctx(&mut ctx, kass_dao);

    let ix = ctx.kass_price_ix(protocol, kass_dao);
    let res = ctx.send(ix, &[]);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidAccount as u32),
        "a kass_dao not owned by the futarchy program must be rejected: {res:?}",
    );
}

#[test]
fn zero_aggregator_rejected() {
    let mut ctx = TestCtx::new();

    // aggregator == 0 -> no observation yet -> not observable (per F0 contract).
    let kass_dao = Pubkey::new_unique();
    ctx.fabricate_owned_account(
        kass_dao,
        futarchy_owner(),
        build_dao_blob(0, 1_000_000, 100_000, 0),
    );
    let protocol = governed_ctx(&mut ctx, kass_dao);

    let ix = ctx.kass_price_ix(protocol, kass_dao);
    let res = ctx.send(ix, &[]);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidAccount as u32),
        "a zero-aggregator (no-observation) oracle must be rejected: {res:?}",
    );
}

#[test]
fn substituted_protocol_rejected() {
    let mut ctx = TestCtx::new();

    let kass_dao = Pubkey::new_unique();
    ctx.fabricate_owned_account(
        kass_dao,
        futarchy_owner(),
        build_dao_blob(4_200_000_000_000_000, 1_000_000, 100_000, 0),
    );
    let _protocol = governed_ctx(&mut ctx, kass_dao);

    // A non-canonical protocol account: `load_protocol` pins the `[b"protocol"]`
    // PDA, so a substitute is rejected before the price read.
    let fake_protocol = Pubkey::new_unique();
    let ix = ctx.kass_price_ix(fake_protocol, kass_dao);
    let res = ctx.send(ix, &[]);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidAccount as u32),
        "a substituted protocol account must be rejected: {res:?}",
    );
}

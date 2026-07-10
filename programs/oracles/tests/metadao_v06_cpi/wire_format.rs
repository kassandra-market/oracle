use super::*;

use solana_instruction::AccountMeta;

// ─────────────────────────────────────────────────────────────────────────────
// 1. All fixtures load.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn all_v06_fixtures_load() {
    let mut svm = LiteSVM::new();
    load_all(&mut svm);

    for (name, id) in [
        ("futarchy", futarchy_id()),
        ("conditional_vault_v06", vault_id()),
        ("meteora_damm_v2", meteora_id()),
        ("squads_v4", squads_id()),
    ] {
        let acc = svm
            .get_account(&id)
            .unwrap_or_else(|| panic!("{name} not loaded"));
        assert!(acc.executable, "{name} must be executable");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 1b. Squads v4 `vault_transaction_execute` discriminator anchor (F6 seam).
//     The Squads v4 multisig vault PDA is Kassandra's `Protocol.dao_authority`;
//     a passed futarchy proposal signs `set_config`/`resolve_deadend` as that
//     vault via Squads' `vault_transaction_execute`. We anchor the COMPUTED
//     `sha256("global:vault_transaction_execute")[..8]` discriminator against the
//     real dumped binary: the program logs "Instruction: VaultTransactionExecute"
//     iff its Anchor dispatch table recognizes our discriminator. A garbage
//     discriminator is not recognized. Both txs fail (we pass only the payer as
//     an account), but the dispatch log distinguishes a real disc from a fake —
//     proving the wire format of the execution seam without the (intractable in
//     LiteSVM) full multisig + proposal + vault-transaction setup.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn squads_vault_transaction_execute_discriminator_recognized() {
    let mut svm = LiteSVM::new();
    load_all(&mut svm);

    let payer = Keypair::new();
    svm.airdrop(&payer.pubkey(), 1_000_000_000).unwrap();

    // Real discriminator + deliberately insufficient accounts.
    let real = Instruction {
        program_id: squads_id(),
        accounts: vec![AccountMeta::new(payer.pubkey(), true)],
        data: md6::SQUADS_VAULT_TRANSACTION_EXECUTE.to_vec(),
    };
    let real_logs = send_capture_logs(&mut svm, &payer, real);
    assert!(
        real_logs
            .iter()
            .any(|l| l.contains("Instruction: VaultTransactionExecute")),
        "real Squads v4 binary did not dispatch our vault_transaction_execute discriminator;\nlogs: {real_logs:#?}"
    );

    // Bogus discriminator → no such instruction → never logs the handler name.
    let bogus = Instruction {
        program_id: squads_id(),
        accounts: vec![AccountMeta::new(payer.pubkey(), true)],
        data: vec![0xde, 0xad, 0xbe, 0xef, 0x01, 0x02, 0x03, 0x04],
    };
    let bogus_logs = send_capture_logs(&mut svm, &payer, bogus);
    assert!(
        !bogus_logs
            .iter()
            .any(|l| l.contains("Instruction: VaultTransactionExecute")),
        "a bogus discriminator was (impossibly) dispatched as vault_transaction_execute;\nlogs: {bogus_logs:#?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 1c. Squads v4 vault-PDA seed derivation is self-consistent (F6 seam).
//     The DAO execution authority is `[b"multisig", multisig, b"vault", [0]]`
//     under SQDS4…, where `multisig` is `[b"multisig", b"multisig", dao]`. These
//     are the seeds Kassandra records as `Protocol.dao_authority`; the gate test
//     in `tests/governance_seam.rs` proves the Kassandra side end-to-end.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn squads_vault_pda_derivation() {
    let dao = Pubkey::new_unique().to_bytes();
    let (multisig, _) =
        Pubkey::find_program_address(&md6::squads_multisig_seeds(&dao.into()), &squads_id());
    let multisig_arr = multisig.to_bytes();
    let (vault, _vbump) = Pubkey::find_program_address(
        &md6::squads_vault_seeds(&multisig_arr.into(), &[0u8]),
        &squads_id(),
    );
    // Re-derivation is stable.
    let (vault2, _) = Pubkey::find_program_address(
        &md6::squads_vault_seeds(&multisig_arr.into(), &[0u8]),
        &squads_id(),
    );
    assert_eq!(vault, vault2);
    assert_ne!(vault, multisig);
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Host-runnable seed builders match the documented v0.6 layout.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn seed_helpers_match_documented_layout() {
    let creator = [3u8; 32];
    let nonce_le = 7u64.to_le_bytes();
    assert_eq!(
        md6::dao_seeds(&creator.into(), &nonce_le),
        [b"dao".as_ref(), creator.as_ref(), nonce_le.as_ref()],
    );

    let squads_proposal = [9u8; 32];
    assert_eq!(
        md6::proposal_seeds(&squads_proposal.into()),
        [b"proposal".as_ref(), squads_proposal.as_ref()],
    );

    let dao = [1u8; 32];
    assert_eq!(
        md6::squads_multisig_seeds(&dao.into()),
        [b"multisig".as_ref(), b"multisig".as_ref(), dao.as_ref()],
    );

    let multisig = [2u8; 32];
    let vault_index = [0u8; 1];
    assert_eq!(
        md6::squads_vault_seeds(&multisig.into(), &vault_index),
        [
            b"multisig".as_ref(),
            multisig.as_ref(),
            b"vault".as_ref(),
            vault_index.as_ref()
        ],
    );

    assert_eq!(
        md6::event_authority_seeds(),
        [b"__event_authority".as_ref()]
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. Anchor-discriminator anchor against the real futarchy binary.
//    The deployed program logs "Instruction: InitializeDao" iff its dispatch
//    table recognizes the discriminator we computed. A garbage discriminator is
//    NOT recognized, so it never logs that. Both txs fail (we pass only the payer
//    as an account), but the dispatch log distinguishes a real disc from a fake.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn futarchy_initialize_dao_discriminator_recognized() {
    let mut svm = LiteSVM::new();
    load_all(&mut svm);

    let payer = Keypair::new();
    svm.airdrop(&payer.pubkey(), 1_000_000_000).unwrap();

    // Real discriminator + deliberately insufficient accounts.
    let real = Instruction {
        program_id: futarchy_id(),
        accounts: vec![AccountMeta::new(payer.pubkey(), true)],
        data: md6::FUT_INITIALIZE_DAO.to_vec(),
    };
    let real_logs = send_capture_logs(&mut svm, &payer, real);
    assert!(
        real_logs.iter().any(|l| l.contains("Instruction: InitializeDao")),
        "real futarchy binary did not dispatch our initialize_dao discriminator;\nlogs: {real_logs:#?}"
    );

    // Bogus discriminator → no such instruction → never logs InitializeDao.
    let bogus = Instruction {
        program_id: futarchy_id(),
        accounts: vec![AccountMeta::new(payer.pubkey(), true)],
        data: vec![0xde, 0xad, 0xbe, 0xef, 0x01, 0x02, 0x03, 0x04],
    };
    let bogus_logs = send_capture_logs(&mut svm, &payer, bogus);
    assert!(
        !bogus_logs.iter().any(|l| l.contains("Instruction: InitializeDao")),
        "a bogus discriminator was (impossibly) dispatched as initialize_dao;\nlogs: {bogus_logs:#?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. Futarchy spot-TWAP offset map validated against the source get_twap math.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn futarchy_spot_twap_offsets_match_get_twap() {
    // Hand-build a Dao blob: 8-byte disc + PoolState::Spot tag (0) + a spot Pool
    // whose TwapOracle has known fields at the documented offsets.
    let mut data = vec![0u8; md6::DAO_SPOT_TWAP_MIN_LEN];
    data[0..8].copy_from_slice(&md6::DAO_ACCOUNT_DISCRIMINATOR);
    data[md6::DAO_POOLSTATE_TAG_OFFSET] = 0; // Spot

    let aggregator: u128 = 4_200_000_000_000_000; // running sum
    let last_updated: i64 = 1_000_000;
    let created_at: i64 = 100_000;
    let start_delay: u32 = 0;

    data[md6::DAO_SPOT_AGGREGATOR_OFFSET..md6::DAO_SPOT_AGGREGATOR_OFFSET + 16]
        .copy_from_slice(&aggregator.to_le_bytes());
    data[md6::DAO_SPOT_LAST_UPDATED_TS_OFFSET..md6::DAO_SPOT_LAST_UPDATED_TS_OFFSET + 8]
        .copy_from_slice(&last_updated.to_le_bytes());
    data[md6::DAO_SPOT_CREATED_AT_TS_OFFSET..md6::DAO_SPOT_CREATED_AT_TS_OFFSET + 8]
        .copy_from_slice(&created_at.to_le_bytes());
    data[md6::DAO_SPOT_START_DELAY_SECONDS_OFFSET..md6::DAO_SPOT_START_DELAY_SECONDS_OFFSET + 4]
        .copy_from_slice(&start_delay.to_le_bytes());

    // get_twap = aggregator / (last_updated - (created_at + start_delay))
    let expected = aggregator / (last_updated - (created_at + start_delay as i64)) as u128;
    let got = md6::futarchy_spot_twap(&data).expect("twap decode");
    assert_eq!(got, expected, "spot TWAP offset/decode mismatch");

    // Zero aggregator (no observation yet) -> not observable.
    let mut empty = data.clone();
    empty[md6::DAO_SPOT_AGGREGATOR_OFFSET..md6::DAO_SPOT_AGGREGATOR_OFFSET + 16]
        .copy_from_slice(&0u128.to_le_bytes());
    assert!(md6::futarchy_spot_twap(&empty).is_err());

    // Truncated buffer -> InvalidAccount.
    assert!(md6::futarchy_spot_twap(&data[..md6::DAO_SPOT_POOL_OFFSET]).is_err());
}

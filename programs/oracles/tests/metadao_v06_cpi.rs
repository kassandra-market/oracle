//! MetaDAO **futarchy v0.6** + **Meteora DAMM v2** wire-format / layout
//! validation (Task F0).
//!
//! These tests load the **real** mainnet binaries dumped by
//! `scripts/fetch-metadao-v06.sh` into `tests/fixtures/` and exercise the v0.6
//! stack against them, using the discriminators, PDA seeds, arg encoders, and
//! layout offsets from [`kassandra_oracles_program::cpi::metadao_v06`].
//!
//! Coverage vs. deferral (be honest):
//! * [`all_v06_fixtures_load`] — all four v0.6/Meteora/Squads program fixtures
//!   load into LiteSVM and are executable. (Required deliverable.)
//! * [`squads_vault_transaction_execute_discriminator_recognized`] (F6) — anchors
//!   the COMPUTED Squads v4 `vault_transaction_execute` discriminator against the
//!   real dumped `squads_v4.so`: the binary dispatches into its
//!   `VaultTransactionExecute` handler for our discriminator (logs the handler
//!   name) and rejects a bogus one. This is the DAO-execution seam's wire format.
//! * [`v06_conditional_vault_split`] — a FULL real-binary CPI: initialize_question
//!   → initialize_conditional_vault → split_tokens against the v0.6-dumped
//!   conditional_vault, proving its discriminators + PDA seeds + arg layout match
//!   the deployed v0.6 binary (the vault is unchanged from v0.4).
//! * [`futarchy_initialize_dao_discriminator_recognized`] — anchors the COMPUTED
//!   `initialize_dao` discriminator against the real futarchy binary: the program
//!   dispatches into the `InitializeDao` handler (logs `Instruction: InitializeDao`)
//!   for our discriminator, but treats a bogus discriminator as a fallback. This
//!   proves the futarchy instruction wire format without the heavy full setup.
//! * [`futarchy_spot_twap_offsets_match_get_twap`] — validates the documented
//!   `Dao` spot-`TwapOracle` byte offsets + the `get_twap` math against a
//!   hand-built `Dao` blob.
//! * DEFERRED to F5/F6: a full `initialize_dao` success (needs the Squads v4
//!   multisig program + mints), driving a proposal to pass/execute, and reading a
//!   LIVE Meteora cp-amm pool's `sqrt_price` (cp-amm has no TWAP; see the module
//!   docs).

use kassandra_oracles_program::cpi::metadao_v06 as md6;
use litesvm::LiteSVM;
use solana_account::Account;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::Transaction;
use spl_token::{
    solana_program::{program_option::COption, program_pack::Pack},
    state::{Account as TokenAccount, AccountState, Mint},
    ID as TOKEN_PROGRAM_ID,
};

const FUTARCHY_SO: &[u8] = include_bytes!("fixtures/metadao_futarchy_v06.so");
const VAULT_V06_SO: &[u8] = include_bytes!("fixtures/metadao_conditional_vault_v06.so");
const METEORA_SO: &[u8] = include_bytes!("fixtures/meteora_damm_v2.so");
const SQUADS_SO: &[u8] = include_bytes!("fixtures/squads_v4.so");

const ATA_PROGRAM_ID: Pubkey =
    solana_pubkey::pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

fn futarchy_id() -> Pubkey {
    Pubkey::new_from_array(md6::FUTARCHY_ID.to_bytes())
}
fn vault_id() -> Pubkey {
    Pubkey::new_from_array(md6::CONDITIONAL_VAULT_V06_ID.to_bytes())
}
fn meteora_id() -> Pubkey {
    Pubkey::new_from_array(md6::METEORA_DAMM_V2_ID.to_bytes())
}
fn squads_id() -> Pubkey {
    Pubkey::new_from_array(md6::SQUADS_V4_ID.to_bytes())
}

fn load_all(svm: &mut LiteSVM) {
    svm.add_program(futarchy_id(), FUTARCHY_SO).unwrap();
    svm.add_program(vault_id(), VAULT_V06_SO).unwrap();
    svm.add_program(meteora_id(), METEORA_SO).unwrap();
    svm.add_program(squads_id(), SQUADS_SO).unwrap();
}

fn ata(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[owner.as_ref(), TOKEN_PROGRAM_ID.as_ref(), mint.as_ref()],
        &ATA_PROGRAM_ID,
    )
    .0
}

fn fabricate_mint(svm: &mut LiteSVM, decimals: u8, authority: Pubkey) -> Pubkey {
    let mint = Pubkey::new_unique();
    let state = Mint {
        mint_authority: COption::Some(authority),
        supply: 0,
        decimals,
        is_initialized: true,
        freeze_authority: COption::None,
    };
    let mut data = vec![0u8; Mint::LEN];
    state.pack_into_slice(&mut data);
    let lamports = svm.minimum_balance_for_rent_exemption(Mint::LEN);
    svm.set_account(
        mint,
        Account {
            lamports,
            data,
            owner: TOKEN_PROGRAM_ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
    mint
}

fn fabricate_token_account(
    svm: &mut LiteSVM,
    addr: Pubkey,
    mint: Pubkey,
    owner: Pubkey,
    amount: u64,
) {
    let state = TokenAccount {
        mint,
        owner,
        amount,
        delegate: COption::None,
        state: AccountState::Initialized,
        is_native: COption::None,
        delegated_amount: 0,
        close_authority: COption::None,
    };
    let mut data = vec![0u8; TokenAccount::LEN];
    state.pack_into_slice(&mut data);
    let lamports = svm.minimum_balance_for_rent_exemption(TokenAccount::LEN);
    svm.set_account(
        addr,
        Account {
            lamports,
            data,
            owner: TOKEN_PROGRAM_ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
}

fn token_balance(svm: &LiteSVM, addr: Pubkey) -> u64 {
    let acc = svm.get_account(&addr).expect("token account missing");
    TokenAccount::unpack(&acc.data)
        .expect("not a token account")
        .amount
}

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
// 3. Full real-binary CPI: conditional_vault split against the v0.6 vault binary.
//    (The v0.6 vault is the same deployed program as v0.4; this re-proves the
//    wire format against the freshly-dumped v0.6 fixture.)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn v06_conditional_vault_split() {
    use kassandra_oracles_program::cpi::metadao as md4; // shared vault discriminators/seeds

    let mut svm = LiteSVM::new();
    load_all(&mut svm);

    let payer = Keypair::new();
    svm.airdrop(&payer.pubkey(), 10_000_000_000).unwrap();

    let kass = fabricate_mint(&mut svm, 9, payer.pubkey());

    let underlying_amount: u64 = 5_000_000_000;
    let user_underlying = ata(&payer.pubkey(), &kass);
    fabricate_token_account(
        &mut svm,
        user_underlying,
        kass,
        payer.pubkey(),
        underlying_amount,
    );

    let num_outcomes: u8 = 2;
    let question_id = [7u8; 32];
    let resolver = Pubkey::new_unique();
    let resolver_pk = resolver.to_bytes();

    let kass_arr = kass.to_bytes();
    let (question, _) = Pubkey::find_program_address(
        &md4::question_seeds(&question_id, &resolver_pk.into(), &[num_outcomes]),
        &vault_id(),
    );
    let question_arr = question.to_bytes();
    let (vault, _) = Pubkey::find_program_address(
        &md4::vault_seeds(&question_arr.into(), &kass_arr.into()),
        &vault_id(),
    );
    let vault_arr = vault.to_bytes();
    let (cond0, _) = Pubkey::find_program_address(
        &md4::conditional_token_mint_seeds(&vault_arr.into(), &[0u8]),
        &vault_id(),
    );
    let (cond1, _) = Pubkey::find_program_address(
        &md4::conditional_token_mint_seeds(&vault_arr.into(), &[1u8]),
        &vault_id(),
    );
    let (event_authority, _) =
        Pubkey::find_program_address(&md4::event_authority_seeds(), &vault_id());

    let vault_underlying = ata(&vault, &kass);

    let ix_q = Instruction {
        program_id: vault_id(),
        accounts: vec![
            AccountMeta::new(question, false),
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new_readonly(solana_sdk_ids::system_program::ID, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(vault_id(), false),
        ],
        data: md4::initialize_question_data(&question_id, &resolver_pk.into(), num_outcomes)
            .to_vec(),
    };
    send(&mut svm, &payer, &[ix_q]).expect("initialize_question failed");

    let ix_v = Instruction {
        program_id: vault_id(),
        accounts: vec![
            AccountMeta::new(vault, false),
            AccountMeta::new_readonly(question, false),
            AccountMeta::new_readonly(kass, false),
            AccountMeta::new(vault_underlying, false),
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(ATA_PROGRAM_ID, false),
            AccountMeta::new_readonly(solana_sdk_ids::system_program::ID, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new(cond0, false),
            AccountMeta::new(cond1, false),
        ],
        data: md4::initialize_conditional_vault_data().to_vec(),
    };
    send(&mut svm, &payer, &[ix_v]).expect("initialize_conditional_vault failed");

    let user_cond0 = ata(&payer.pubkey(), &cond0);
    let user_cond1 = ata(&payer.pubkey(), &cond1);
    fabricate_token_account(&mut svm, user_cond0, cond0, payer.pubkey(), 0);
    fabricate_token_account(&mut svm, user_cond1, cond1, payer.pubkey(), 0);

    let split_amount: u64 = 2_000_000_000;
    let ix_s = Instruction {
        program_id: vault_id(),
        accounts: vec![
            AccountMeta::new_readonly(question, false),
            AccountMeta::new(vault, false),
            AccountMeta::new(vault_underlying, false),
            AccountMeta::new_readonly(payer.pubkey(), true),
            AccountMeta::new(user_underlying, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new(cond0, false),
            AccountMeta::new(cond1, false),
            AccountMeta::new(user_cond0, false),
            AccountMeta::new(user_cond1, false),
        ],
        data: md4::split_tokens_data(split_amount).to_vec(),
    };
    send(&mut svm, &payer, &[ix_s]).expect("split_tokens failed");

    assert_eq!(token_balance(&svm, user_cond0), split_amount);
    assert_eq!(token_balance(&svm, user_cond1), split_amount);
    assert_eq!(token_balance(&svm, vault_underlying), split_amount);
    assert_eq!(
        token_balance(&svm, user_underlying),
        underlying_amount - split_amount
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

// ─────────────────────────────────────────────────────────────────────────────
// helpers
// ─────────────────────────────────────────────────────────────────────────────

#[allow(clippy::result_large_err)]
fn send(
    svm: &mut LiteSVM,
    payer: &Keypair,
    ixs: &[Instruction],
) -> Result<(), litesvm::types::FailedTransactionMetadata> {
    let mut all = vec![ComputeBudgetInstruction::set_compute_unit_limit(600_000)];
    all.extend_from_slice(ixs);
    svm.expire_blockhash();
    let bh = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(&all, Some(&payer.pubkey()), &[payer], bh);
    svm.send_transaction(tx).map(|_| ())
}

/// Send a single instruction and return its logs whether it succeeds or fails.
fn send_capture_logs(svm: &mut LiteSVM, payer: &Keypair, ix: Instruction) -> Vec<String> {
    let all = vec![
        ComputeBudgetInstruction::set_compute_unit_limit(400_000),
        ix,
    ];
    svm.expire_blockhash();
    let bh = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(&all, Some(&payer.pubkey()), &[payer], bh);
    match svm.send_transaction(tx) {
        Ok(meta) => meta.logs,
        Err(failed) => failed.meta.logs,
    }
}

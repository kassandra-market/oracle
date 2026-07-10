//! MetaDAO `conditional_vault` CPI wire-format validation (Task 9).
//!
//! These tests load the **real** mainnet program binaries (dumped by
//! `scripts/fetch-metadao.sh` into `tests/fixtures/`) into LiteSVM and exercise
//! the conditional-vault split path by constructing the MetaDAO instructions
//! *directly* — using the discriminators, PDA seeds, and arg encoders from
//! [`kassandra_oracles_program::cpi::metadao`]. If those values are wrong, the real
//! binary rejects the transaction, so a green test proves our hand-built wire
//! format matches the deployed program before we wrap it behind a CPI (Task 10).

use kassandra_oracles_program::cpi::metadao;
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

const VAULT_SO: &[u8] = include_bytes!("fixtures/metadao_conditional_vault.so");
const AMM_SO: &[u8] = include_bytes!("fixtures/metadao_amm.so");

/// SPL associated-token-account program id (loaded by `LiteSVM::new()`).
const ATA_PROGRAM_ID: Pubkey =
    solana_pubkey::pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

fn vault_id() -> Pubkey {
    Pubkey::new_from_array(metadao::CONDITIONAL_VAULT_ID.to_bytes())
}
fn amm_id() -> Pubkey {
    Pubkey::new_from_array(metadao::AMM_ID.to_bytes())
}

/// Canonical associated token account address for `(owner, mint)`.
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

/// Fabricate a token account at `addr` holding `amount` of `mint`, owned by `owner`.
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

fn load_metadao(svm: &mut LiteSVM) {
    svm.add_program(vault_id(), VAULT_SO).unwrap();
    svm.add_program(amm_id(), AMM_SO).unwrap();
}

/// Drift guard: the host-runnable seed-assembly helpers must produce exactly
/// the documented seed byte-slices in the documented order. The `*_pda`
/// wrappers (SBF-only) reuse these same helpers, so this pins their seeds
/// without needing the `find_program_address` syscall. The end-to-end
/// `split_tokens_mints_conditional_tokens` test then proves the order matches
/// the deployed binary.
#[test]
fn seed_helpers_match_documented_layout() {
    let question_id = [9u8; 32];
    let oracle = [3u8; 32];
    let num_outcomes = [2u8; 1];
    assert_eq!(
        metadao::question_seeds(&question_id, &oracle.into(), &num_outcomes),
        [
            b"question".as_ref(),
            question_id.as_ref(),
            oracle.as_ref(),
            num_outcomes.as_ref(),
        ],
    );

    let question = [4u8; 32];
    let mint = [5u8; 32];
    assert_eq!(
        metadao::vault_seeds(&question.into(), &mint.into()),
        [
            b"conditional_vault".as_ref(),
            question.as_ref(),
            mint.as_ref()
        ],
    );

    let vault = [6u8; 32];
    let index = [1u8; 1];
    assert_eq!(
        metadao::conditional_token_mint_seeds(&vault.into(), &index),
        [
            b"conditional_token".as_ref(),
            vault.as_ref(),
            index.as_ref()
        ],
    );

    assert_eq!(
        metadao::event_authority_seeds(),
        [b"__event_authority".as_ref()]
    );
}

#[test]
fn metadao_programs_load_without_panic() {
    let mut svm = LiteSVM::new();
    load_metadao(&mut svm);

    let vault = svm.get_account(&vault_id()).expect("vault not loaded");
    let amm = svm.get_account(&amm_id()).expect("amm not loaded");
    assert!(vault.executable, "conditional_vault must be executable");
    assert!(amm.executable, "amm must be executable");
}

/// End-to-end split validation against the real conditional_vault binary:
/// initialize_question (2 outcomes) → initialize_conditional_vault →
/// split_tokens, then assert the user received `amount` of each conditional
/// token and the vault escrowed `amount` of the underlying KASS-like mint.
#[test]
fn split_tokens_mints_conditional_tokens() {
    let mut svm = LiteSVM::new();
    load_metadao(&mut svm);

    let payer = Keypair::new();
    svm.airdrop(&payer.pubkey(), 10_000_000_000).unwrap();

    // KASS-like underlying mint, authority = payer.
    let kass = fabricate_mint(&mut svm, 9, payer.pubkey());

    // User (acts as both payer of these txs and split authority).
    let underlying_amount: u64 = 5_000_000_000;
    let user_underlying = ata(&payer.pubkey(), &kass);
    fabricate_token_account(
        &mut svm,
        user_underlying,
        kass,
        payer.pubkey(),
        underlying_amount,
    );

    // ----- derive all PDAs using the cpi::metadao SEED constants ------------
    // (PDA *derivation* uses solana_sdk here because pinocchio's
    // find_program_address is a syscall only available in the SBF runtime; the
    // SEED bytes + program id are the module's, so a successful run still
    // proves those seeds match the real binary's expectations.)
    let num_outcomes: u8 = 2;
    let question_id = [7u8; 32];
    let resolver = Pubkey::new_unique(); // the question's oracle/resolver authority
    let resolver_pk = resolver.to_bytes();

    // Derive every PDA via the module's host-runnable SEED-ASSEMBLY helpers
    // (NOT inline seed arrays). `find_program_address` itself is an SBF-only
    // syscall, so the host test still does the search with `solana_sdk`, but the
    // SEED ORDER comes from `metadao::*_seeds`. A green end-to-end run below
    // proves those helpers match the deployed binary, guarding against drift in
    // the dead-code `*_pda` wrappers that reuse the same builders.
    let question_resolver = resolver.to_bytes();
    let kass_arr = kass.to_bytes();
    let (question, _) = Pubkey::find_program_address(
        &metadao::question_seeds(&question_id, &question_resolver.into(), &[num_outcomes]),
        &vault_id(),
    );
    let question_arr = question.to_bytes();
    let (vault, _) = Pubkey::find_program_address(
        &metadao::vault_seeds(&question_arr.into(), &kass_arr.into()),
        &vault_id(),
    );
    let vault_arr = vault.to_bytes();
    let (cond0, _) = Pubkey::find_program_address(
        &metadao::conditional_token_mint_seeds(&vault_arr.into(), &[0u8]),
        &vault_id(),
    );
    let (cond1, _) = Pubkey::find_program_address(
        &metadao::conditional_token_mint_seeds(&vault_arr.into(), &[1u8]),
        &vault_id(),
    );
    let (event_authority, _) =
        Pubkey::find_program_address(&metadao::event_authority_seeds(), &vault_id());

    let vault_underlying = ata(&vault, &kass);

    // ----- 1. initialize_question ------------------------------------------
    let ix_q = Instruction {
        program_id: vault_id(),
        accounts: vec![
            AccountMeta::new(question, false),
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new_readonly(solana_sdk_ids::system_program::ID, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(vault_id(), false),
        ],
        data: metadao::initialize_question_data(&question_id, &resolver_pk.into(), num_outcomes)
            .to_vec(),
    };
    send(&mut svm, &payer, &[ix_q]).expect("initialize_question failed");

    // ----- 2. initialize_conditional_vault ---------------------------------
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
            // remaining: conditional-token mints (created here)
            AccountMeta::new(cond0, false),
            AccountMeta::new(cond1, false),
        ],
        data: metadao::initialize_conditional_vault_data().to_vec(),
    };
    send(&mut svm, &payer, &[ix_v]).expect("initialize_conditional_vault failed");

    // The vault now owns the underlying ATA; create the user's conditional ATAs.
    let user_cond0 = ata(&payer.pubkey(), &cond0);
    let user_cond1 = ata(&payer.pubkey(), &cond1);
    fabricate_token_account(&mut svm, user_cond0, cond0, payer.pubkey(), 0);
    fabricate_token_account(&mut svm, user_cond1, cond1, payer.pubkey(), 0);

    // ----- 3. split_tokens --------------------------------------------------
    let split_amount: u64 = 2_000_000_000;
    let ix_s = Instruction {
        program_id: vault_id(),
        accounts: vec![
            AccountMeta::new_readonly(question, false),
            AccountMeta::new(vault, false),
            AccountMeta::new(vault_underlying, false),
            AccountMeta::new_readonly(payer.pubkey(), true), // authority (signer)
            AccountMeta::new(user_underlying, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(vault_id(), false),
            // remaining: mints then user conditional accounts
            AccountMeta::new(cond0, false),
            AccountMeta::new(cond1, false),
            AccountMeta::new(user_cond0, false),
            AccountMeta::new(user_cond1, false),
        ],
        data: metadao::split_tokens_data(split_amount).to_vec(),
    };
    send(&mut svm, &payer, &[ix_s]).expect("split_tokens failed");

    // ----- assertions: real binary minted the conditional tokens -----------
    assert_eq!(
        token_balance(&svm, user_cond0),
        split_amount,
        "pass/outcome-0 balance"
    );
    assert_eq!(
        token_balance(&svm, user_cond1),
        split_amount,
        "fail/outcome-1 balance"
    );
    assert_eq!(
        token_balance(&svm, vault_underlying),
        split_amount,
        "vault escrowed underlying"
    );
    assert_eq!(
        token_balance(&svm, user_underlying),
        underlying_amount - split_amount,
        "user underlying decreased by split amount"
    );
}

/// Build, sign, and send a transaction with a generous compute-unit limit
/// (the Anchor init paths + `#[event_cpi]` are CU-heavy).
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

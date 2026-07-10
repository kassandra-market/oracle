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
use solana_instruction::Instruction;
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

#[path = "metadao_v06_cpi/wire_format.rs"]
mod wire_format;
#[path = "metadao_v06_cpi/vault_split.rs"]
mod vault_split;

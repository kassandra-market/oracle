//! Shared LiteSVM test harness for the Kassandra-market program.
//!
//! Every integration test starts with `mod common; use common::*;` and builds a
//! [`TestCtx`], which deploys the compiled `.so` into a fresh [`LiteSVM`], funds
//! a payer, and exposes convenience builders for the program's instructions.
//!
//! The `.so` is `include_bytes!`'d at compile time, so `just build`
//! (`cargo build-sbf`) MUST run **before** `cargo test` — otherwise the embedded
//! bytes are stale (or missing).

#![allow(dead_code)]

use litesvm::{types::TransactionResult, LiteSVM};
use solana_sdk::{
    account::Account,
    clock::Clock,
    compute_budget::ComputeBudgetInstruction,
    instruction::{Instruction, InstructionError},
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::{Transaction, TransactionError},
};
use spl_token::{
    solana_program::{program_option::COption, program_pack::Pack},
    state::{Account as TokenAccount, AccountState, Mint},
    ID as TOKEN_PROGRAM_ID,
};

/// Build a raw `Oracle` account body (`ORACLE_LEN` bytes) matching the sibling
/// Kassandra layout the market gate reads: tag byte 0, plus `options_count`,
/// `phase`, and `resolved_option` stamped at their exact offsets. Mirrors
/// `kassandra_market_program::kass_oracle::KassOracle::read` so the harness and
/// the on-chain gate agree byte-for-byte.
fn kass_oracle_bytes(options_count: u8, phase: u8, resolved_option: u8) -> Vec<u8> {
    use kassandra_market_program::kass_oracle as k;
    let mut data = vec![0u8; k::ORACLE_LEN];
    data[0] = k::ORACLE_ACCOUNT_TYPE;
    data[k::OPTIONS_COUNT_OFFSET] = options_count;
    data[k::PHASE_OFFSET] = phase;
    data[k::RESOLVED_OPTION_OFFSET] = resolved_option;
    data
}

/// The Kassandra program id that must own a fabricated oracle account.
fn kass_oracle_owner() -> Pubkey {
    Pubkey::new_from_array(kassandra_market_program::kass_oracle::KASSANDRA_PROGRAM_ID.to_bytes())
}

/// LiteSVM-backed test context: a funded payer plus the deployed program.
pub struct TestCtx {
    pub svm: LiteSVM,
    pub payer: Keypair,
    pub program_id: Pubkey,
}

/// Fabricate the BPF-Upgradeable-Loader `ProgramData` account of `program_id`
/// with `authority` as the stored `upgrade_authority`, at the canonical PDA
/// `find_program_address([program_id], BPF_UPGRADEABLE_LOADER_ID)`.
///
/// Builds the 45-byte `UpgradeableLoaderState::ProgramData` metadata the program
/// reads: `u32 LE variant == 3 @0`, `u64 LE slot @4`, `Option::Some tag == 1 @12`,
/// then the 32-byte authority `@13..45`. The account is loader-owned + rent-exempt.
fn set_program_data(svm: &mut LiteSVM, program_id: &Pubkey, authority: &Pubkey) {
    let (program_data, _) = kassandra_market_sdk::pda::program_data(program_id);
    let mut data = vec![0u8; 45];
    data[0..4].copy_from_slice(&3u32.to_le_bytes()); // ProgramData variant
                                                     // bytes [4..12] = slot (0)
    data[12] = 1; // Option::Some
    data[13..45].copy_from_slice(&authority.to_bytes());
    svm.set_account(
        program_data,
        Account {
            lamports: 1_000_000_000,
            data,
            owner: kassandra_market_sdk::pda::BPF_UPGRADEABLE_LOADER_ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
}

impl TestCtx {
    /// Build a fresh context: a funded payer and the compiled
    /// `kassandra_market_program` deployed so tests can submit real
    /// transactions via [`TestCtx::send`].
    pub fn new() -> Self {
        let mut svm = LiteSVM::new();
        let payer = Keypair::new();
        svm.airdrop(&payer.pubkey(), 1_000_000_000_000).unwrap();

        let program_id = Pubkey::new_from_array(kassandra_market_program::ID.to_bytes());
        svm.add_program(
            program_id,
            include_bytes!("../../../../target/deploy/kassandra_market_program.so"),
        );

        // Fabricate the program's BPF-Upgradeable-Loader `ProgramData` account so
        // `init_config` (which requires the caller be the program's upgrade
        // authority) accepts the canonical `payer`. The upgrade authority stored
        // here MUST equal the key that signs `init_config` — the harness `payer`.
        set_program_data(&mut svm, &program_id, &payer.pubkey());

        Self {
            svm,
            payer,
            program_id,
        }
    }

    /// Overwrite the fabricated `ProgramData` account so `authority` becomes the
    /// program's on-chain upgrade authority (for the negative front-run test,
    /// where a non-authority signer must be rejected).
    pub fn set_upgrade_authority(&mut self, authority: &Pubkey) {
        set_program_data(&mut self.svm, &self.program_id, authority);
    }

    /// `init_config` signed by an arbitrary `signer` (the ix payer). Used by the
    /// front-run negative test: when `signer` is NOT the program's upgrade
    /// authority the processor must reject with `NotUpgradeAuthority`. The harness
    /// `payer` remains the fee payer; `signer` co-signs.
    #[allow(clippy::result_large_err)]
    pub fn init_config_signed_by(
        &mut self,
        signer: &Keypair,
        authority: Pubkey,
        kass_mint: Pubkey,
        min_liquidity: u64,
        fee_bps: u16,
        fee_destination: Pubkey,
    ) -> TransactionResult {
        let ix = kassandra_market_sdk::ix::init_config(
            &signer.pubkey(),
            &kass_mint,
            &authority,
            min_liquidity,
            fee_bps,
            &fee_destination,
        );
        self.send(ix, &[signer])
    }

    /// Deploy the two MetaDAO v0.4 programs (`conditional_vault` + `amm`) into
    /// the SVM from the vendored `.so` fixtures, so activation tests can compose
    /// and CPI into a real MetaDAO market. The bytes are `include_bytes!`'d at
    /// compile time from `tests/fixtures/`.
    pub fn load_metadao(&mut self) {
        const VAULT_SO: &[u8] = include_bytes!("../fixtures/metadao_conditional_vault.so");
        const AMM_SO: &[u8] = include_bytes!("../fixtures/metadao_amm.so");
        // Program IDs (base58): conditional_vault
        // VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg, amm
        // AMMyu265tkBpRW21iGQxKGLaves3gKm2JcMUqfXNSpqD.
        const VAULT_ID: Pubkey = solana_sdk::pubkey!("VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg");
        const AMM_ID: Pubkey = solana_sdk::pubkey!("AMMyu265tkBpRW21iGQxKGLaves3gKm2JcMUqfXNSpqD");
        self.svm.add_program(VAULT_ID, VAULT_SO);
        self.svm.add_program(AMM_ID, AMM_SO);
    }

    // ----- transaction submission --------------------------------------------

    /// Sign and submit a single-instruction transaction, returning the LiteSVM
    /// result so tests can assert `Ok`/`Err` and introspect the error.
    ///
    /// Signed by the payer (fee payer) plus every keypair in `extra_signers`.
    /// The blockhash is expired and re-fetched each call so two otherwise
    /// identical transactions get distinct signatures and never collide.
    #[allow(clippy::result_large_err)]
    pub fn send(&mut self, ix: Instruction, extra_signers: &[&Keypair]) -> TransactionResult {
        self.svm.expire_blockhash();
        let blockhash = self.svm.latest_blockhash();
        let mut all_signers: Vec<&Keypair> = Vec::with_capacity(extra_signers.len() + 1);
        all_signers.push(&self.payer);
        all_signers.extend_from_slice(extra_signers);
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&self.payer.pubkey()),
            &all_signers,
            blockhash,
        );
        self.svm.send_transaction(tx)
    }

    /// Sign and submit a multi-instruction transaction (a 1.4M-CU compute budget
    /// is prepended automatically — the MetaDAO composition + `activate` CPIs
    /// exceed the 200k default). Signed by the payer plus `extra_signers`.
    #[allow(clippy::result_large_err)]
    pub fn send_many(
        &mut self,
        ixs: &[Instruction],
        extra_signers: &[&Keypair],
    ) -> TransactionResult {
        self.svm.expire_blockhash();
        let blockhash = self.svm.latest_blockhash();
        let mut all_ixs = Vec::with_capacity(ixs.len() + 1);
        all_ixs.push(ComputeBudgetInstruction::set_compute_unit_limit(1_400_000));
        all_ixs.extend_from_slice(ixs);
        let mut all_signers: Vec<&Keypair> = Vec::with_capacity(extra_signers.len() + 1);
        all_signers.push(&self.payer);
        all_signers.extend_from_slice(extra_signers);
        let tx = Transaction::new_signed_with_payer(
            &all_ixs,
            Some(&self.payer.pubkey()),
            &all_signers,
            blockhash,
        );
        self.svm.send_transaction(tx)
    }

    // ----- accessors ---------------------------------------------------------

    /// Read a program account and reinterpret its data as a `Pod` struct `T`.
    ///
    /// Uses [`bytemuck::pod_read_unaligned`] so correctness does not depend on
    /// the alignment of the account-data buffer.
    pub fn read_pod<T: bytemuck::Pod>(&self, key: Pubkey) -> T {
        let acc = self
            .svm
            .get_account(&key)
            .unwrap_or_else(|| panic!("account {key} not found"));
        bytemuck::pod_read_unaligned::<T>(&acc.data)
    }

    /// Current on-chain unix timestamp from the `Clock` sysvar.
    pub fn now(&self) -> i64 {
        self.svm.get_sysvar::<Clock>().unix_timestamp
    }

    /// Lamports balance of any account (0 if it does not exist).
    pub fn lamports(&self, key: Pubkey) -> u64 {
        self.svm.get_account(&key).map(|a| a.lamports).unwrap_or(0)
    }

    // ----- low-level fabrication ---------------------------------------------

    /// Airdrop 1e12 lamports to an arbitrary key (so it exists as a funded
    /// system account, e.g. a rent recipient or extra signer). Later tasks need
    /// to fund keys that are not the payer.
    pub fn svm_airdrop(&mut self, key: &Pubkey) {
        self.svm.airdrop(key, 1_000_000_000_000).unwrap();
    }

    /// Fabricate an initialized SPL mint with the given decimals, authority =
    /// the payer, supply 0. Returns its address.
    pub fn create_mint(&mut self, decimals: u8) -> Pubkey {
        let mint = Pubkey::new_unique();
        let state = Mint {
            mint_authority: COption::Some(self.payer.pubkey()),
            supply: 0,
            decimals,
            is_initialized: true,
            freeze_authority: COption::None,
        };
        let mut data = vec![0u8; Mint::LEN];
        state.pack_into_slice(&mut data);
        let lamports = self.svm.minimum_balance_for_rent_exemption(Mint::LEN);
        self.svm
            .set_account(
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

    /// Fabricate an initialized SPL token account on `mint` owned by `owner`
    /// holding `amount` base units. Returns its address.
    pub fn create_token_account(&mut self, mint: Pubkey, owner: Pubkey, amount: u64) -> Pubkey {
        let addr = Pubkey::new_unique();
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
        let lamports = self
            .svm
            .minimum_balance_for_rent_exemption(TokenAccount::LEN);
        self.svm
            .set_account(
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
        addr
    }

    /// Overwrite an existing SPL token account's `amount` in place (preserving
    /// mint/owner/state) — used to simulate a dust donation into a program-owned
    /// token account (e.g. an escrow PDA a griefer transfers into).
    pub fn set_token_amount(&mut self, addr: Pubkey, amount: u64) {
        let acc = self
            .svm
            .get_account(&addr)
            .unwrap_or_else(|| panic!("token account {addr} not found"));
        let mut state = TokenAccount::unpack(&acc.data).expect("not a token account");
        state.amount = amount;
        let mut data = vec![0u8; TokenAccount::LEN];
        state.pack_into_slice(&mut data);
        self.svm
            .set_account(
                addr,
                Account {
                    lamports: acc.lamports,
                    data,
                    owner: TOKEN_PROGRAM_ID,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
    }

    /// Read the token balance (base units) of an SPL token account.
    pub fn token_balance(&self, key: Pubkey) -> u64 {
        let acc = self
            .svm
            .get_account(&key)
            .unwrap_or_else(|| panic!("token account {key} not found"));
        TokenAccount::unpack(&acc.data)
            .expect("not a token account")
            .amount
    }

    /// Fabricate a minimal Kassandra `Oracle` account (owned by the Kassandra
    /// program) carrying the given `options_count` and `phase`, so market tests
    /// can point `create_market` at a real-looking oracle. Returns its address.
    pub fn seed_kass_oracle(&mut self, options_count: u8, phase: u8) -> Pubkey {
        let addr = Pubkey::new_unique();
        let data = kass_oracle_bytes(options_count, phase, 0);
        let lamports = self.svm.minimum_balance_for_rent_exemption(data.len());
        self.svm
            .set_account(
                addr,
                Account {
                    lamports,
                    data,
                    owner: kass_oracle_owner(),
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
        addr
    }

    // ----- instruction convenience -------------------------------------------

    /// Send an `InitConfig` instruction creating the `Config` singleton. Returns
    /// the config PDA plus the result so tests can assert success / rejection.
    ///
    /// Threads a default protocol fee (100 bps) and a freshly fabricated KASS
    /// `fee_destination` (a token account on `kass_mint` owned by `authority`), so
    /// existing tests need not care about the fee config. Use
    /// [`TestCtx::init_config_full`] to control the fee args.
    #[allow(clippy::result_large_err)]
    pub fn init_config(
        &mut self,
        authority: Pubkey,
        kass_mint: Pubkey,
        min_liquidity: u64,
    ) -> (Pubkey, TransactionResult) {
        let fee_destination = self.create_token_account(kass_mint, authority, 0);
        self.init_config_full(authority, kass_mint, min_liquidity, 100, fee_destination)
    }

    /// Full `InitConfig` with explicit `fee_bps` + `fee_destination` (for the
    /// fee-validation tests). Returns the config PDA plus the result.
    #[allow(clippy::result_large_err)]
    pub fn init_config_full(
        &mut self,
        authority: Pubkey,
        kass_mint: Pubkey,
        min_liquidity: u64,
        fee_bps: u16,
        fee_destination: Pubkey,
    ) -> (Pubkey, TransactionResult) {
        let (config, _) = kassandra_market_sdk::pda::config();
        let ix = kassandra_market_sdk::ix::init_config(
            &self.payer.pubkey(),
            &kass_mint,
            &authority,
            min_liquidity,
            fee_bps,
            &fee_destination,
        );
        let res = self.send(ix, &[]);
        (config, res)
    }

    /// Send a `CreateMarket` instruction for the binary (`outcome_index = 0`)
    /// sub-market. `creator` signs (and pays rent for the market/escrow/
    /// contribution PDAs). Returns the market PDA plus the result.
    #[allow(clippy::result_large_err)]
    pub fn create_market(
        &mut self,
        creator: &Keypair,
        oracle: Pubkey,
        kass_mint: Pubkey,
        creator_ata: Pubkey,
        seed: u64,
    ) -> (Pubkey, TransactionResult) {
        self.create_market_full(creator, oracle, kass_mint, creator_ata, seed, 0)
    }

    /// Full `CreateMarket` with an explicit `outcome_index` (the sub-market this
    /// binds to). Returns the sub-market PDA plus the result.
    #[allow(clippy::result_large_err, clippy::too_many_arguments)]
    pub fn create_market_full(
        &mut self,
        creator: &Keypair,
        oracle: Pubkey,
        kass_mint: Pubkey,
        creator_ata: Pubkey,
        seed: u64,
        outcome_index: u8,
    ) -> (Pubkey, TransactionResult) {
        let (market, _) = kassandra_market_sdk::pda::market(&oracle, outcome_index);
        let ix = kassandra_market_sdk::ix::create_market(
            &creator.pubkey(),
            &oracle,
            &kass_mint,
            &creator_ata,
            seed,
            outcome_index,
        );
        let res = self.send(ix, &[creator]);
        (market, res)
    }

    /// Send a `Contribute` instruction. `contributor` signs (and is the token
    /// authority for `contributor_ata`). Returns the LiteSVM result.
    #[allow(clippy::result_large_err)]
    pub fn contribute(
        &mut self,
        contributor: &solana_sdk::signature::Keypair,
        market: Pubkey,
        contributor_ata: Pubkey,
        amount: u64,
    ) -> litesvm::types::TransactionResult {
        let (escrow, _) = kassandra_market_sdk::pda::escrow(&market);
        let ix = kassandra_market_sdk::ix::contribute(
            &contributor.pubkey(),
            &market,
            &escrow,
            &contributor_ata,
            amount,
        );
        self.send(ix, &[contributor])
    }

    /// Fabricate a Market account at its canonical PDA with an arbitrary status,
    /// for testing status guards without going through the full lifecycle.
    pub fn seed_market_with_status(
        &mut self,
        oracle: Pubkey,
        kass_mint: Pubkey,
        escrow: Pubkey,
        status: u8,
    ) -> Pubkey {
        use bytemuck::Zeroable;
        use kassandra_market_program::state::{AccountType, Market};
        let (market, bump) = kassandra_market_sdk::pda::market(&oracle, 0);
        let mut m = Market::zeroed();
        m.account_type = AccountType::Market.as_u8();
        m.oracle = oracle.to_bytes().into();

        m.kass_mint = kass_mint.to_bytes().into();

        m.escrow_vault = escrow.to_bytes().into();

        m.status = status;
        m.bump = bump;
        let data = bytemuck::bytes_of(&m).to_vec();
        let lamports = self.svm.minimum_balance_for_rent_exemption(data.len());
        self.svm
            .set_account(
                market,
                Account {
                    lamports,
                    data,
                    owner: self.program_id,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
        market
    }

    /// Send a `Cancel` instruction (permissionless). Returns the LiteSVM result.
    #[allow(clippy::result_large_err)]
    pub fn cancel(&mut self, market: Pubkey, oracle: Pubkey) -> litesvm::types::TransactionResult {
        let ix = kassandra_market_sdk::ix::cancel(&market, &oracle);
        self.send(ix, &[])
    }

    /// Send a `Refund` instruction (permissionless). Derives the escrow and the
    /// contribution PDA from `market` + `contributor`, and refunds to
    /// `contributor_ata`. Returns the LiteSVM result.
    #[allow(clippy::result_large_err)]
    pub fn refund(
        &mut self,
        market: Pubkey,
        contributor: Pubkey,
        contributor_ata: Pubkey,
    ) -> litesvm::types::TransactionResult {
        let (escrow, _) = kassandra_market_sdk::pda::escrow(&market);
        let (contribution, _) = kassandra_market_sdk::pda::contribution(&market, &contributor);
        let ix = kassandra_market_sdk::ix::refund(
            &market,
            &escrow,
            &contribution,
            &contributor_ata,
            &contributor,
        );
        self.send(ix, &[])
    }

    /// Send a `CloseMarket` (Ix 10) for the binary (`outcome_index = 0`) sub-market.
    /// Permissionless; reclaims all rent to `creator`. Returns the LiteSVM result.
    #[allow(clippy::result_large_err)]
    pub fn close_market(
        &mut self,
        oracle: Pubkey,
        creator: Pubkey,
    ) -> litesvm::types::TransactionResult {
        let ix = kassandra_market_sdk::ix::close_market(&oracle, &creator, 0);
        self.send(ix, &[])
    }

    /// Attack variant of `refund`: derives the `Contribution` PDA from
    /// `contributor` (the recorded staker) but sends the tokens to an arbitrary
    /// `dest_ata`. Used to prove a cranker cannot redirect someone's refund.
    #[allow(clippy::result_large_err)]
    pub fn refund_to(
        &mut self,
        market: Pubkey,
        contributor: Pubkey,
        dest_ata: Pubkey,
    ) -> litesvm::types::TransactionResult {
        let (escrow, _) = kassandra_market_sdk::pda::escrow(&market);
        let (contribution, _) = kassandra_market_sdk::pda::contribution(&market, &contributor);
        // Recorded contributor is `contributor`; the wrong-dest guard fires before
        // the contributor binding is checked, so pass the real contributor here.
        let ix = kassandra_market_sdk::ix::refund(
            &market,
            &escrow,
            &contribution,
            &dest_ata,
            &contributor,
        );
        self.send(ix, &[])
    }

    /// Attack variant of `refund` with an explicit `Contribution` account:
    /// derives the escrow from `market` but pairs it with an arbitrary
    /// `contribution` PDA (e.g. one belonging to a DIFFERENT market), to prove
    /// the `contribution.market != market` cross-market guard fires. Sends to
    /// `dest_ata`. Returns the LiteSVM result.
    #[allow(clippy::result_large_err)]
    pub fn refund_with_contribution(
        &mut self,
        market: Pubkey,
        contribution: Pubkey,
        dest_ata: Pubkey,
    ) -> litesvm::types::TransactionResult {
        let (escrow, _) = kassandra_market_sdk::pda::escrow(&market);
        // The cross-market guard fires before the contributor binding is checked, so
        // the placeholder contributor (`dest_ata`) is never validated.
        let ix =
            kassandra_market_sdk::ix::refund(&market, &escrow, &contribution, &dest_ata, &dest_ata);
        self.send(ix, &[])
    }

    /// Rewrite an existing fabricated Kassandra oracle account to a new phase
    /// (keeps options_count = 2). Lets a test move an oracle to a terminal phase
    /// after a market has been created against it.
    pub fn set_oracle_phase(&mut self, oracle: Pubkey, phase: u8) {
        let data = kass_oracle_bytes(2, phase, 0);
        let lamports = self.svm.minimum_balance_for_rent_exemption(data.len());
        self.svm
            .set_account(
                oracle,
                solana_sdk::account::Account {
                    lamports,
                    data,
                    owner: kass_oracle_owner(),
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
    }

    /// Compose the MetaDAO market for `market`/`oracle` (the client precondition
    /// for `activate`), using the sdk-rs builders: `initialize_question`
    /// (oracle-authority = the MARKET PDA, question_id = the kassandra oracle
    /// address bytes, num_outcomes = 2), `initialize_conditional_vault`
    /// (underlying = `kass_mint`, creating cYES/cNO mints idx 0/1), and
    /// `create_amm` (base = cYES, quote = cNO, balanced 1e12 initial observation).
    /// Each is its own compute-budgeted transaction. Returns all derived addresses.
    pub fn compose_metadao_market(
        &mut self,
        market: Pubkey,
        oracle: Pubkey,
        kass_mint: Pubkey,
    ) -> MetaDaoRefs {
        use kassandra_market_sdk::metadao as md;
        let payer = self.payer.pubkey();
        let question_id = oracle.to_bytes();

        // (1) initialize_question — oracle-authority == the MARKET PDA.
        let (question, _) = md::question(&question_id, &market, 2);
        let ix_q = md::initialize_question(&payer, &market, &question_id, 2);
        self.send_many(&[ix_q], &[]).expect("initialize_question");

        // (2) initialize_conditional_vault — underlying == kass_mint.
        let (vault, _) = md::vault(&question, &kass_mint);
        let vault_underlying_ata = md::ata(&vault, &kass_mint);
        let (yes_mint, _) = md::conditional_token_mint(&vault, 0);
        let (no_mint, _) = md::conditional_token_mint(&vault, 1);
        let ix_v = md::initialize_conditional_vault(&payer, &question, &kass_mint, 2);
        self.send_many(&[ix_v], &[])
            .expect("initialize_conditional_vault");

        // (3) create_amm — base = cYES, quote = cNO, balanced (price 1.0).
        let (amm, _) = md::amm(&yes_mint, &no_mint);
        let (lp_mint, _) = md::amm_lp_mint(&amm);
        let amm_vault_base = md::ata(&amm, &yes_mint);
        let amm_vault_quote = md::ata(&amm, &no_mint);
        let max_change: u128 = (u64::MAX as u128) * 1_000_000_000_000;
        let ix_a = md::create_amm(
            &payer,
            &yes_mint,
            &no_mint,
            1_000_000_000_000,
            max_change,
            0,
        );
        self.send_many(&[ix_a], &[]).expect("create_amm");

        MetaDaoRefs {
            question,
            vault,
            vault_underlying_ata,
            yes_mint,
            no_mint,
            amm,
            lp_mint,
            amm_vault_base,
            amm_vault_quote,
        }
    }

    /// Send an `Activate` instruction (fee-payer signs and pays rent for the
    /// three market-owned token accounts). Returns the LiteSVM result.
    #[allow(clippy::result_large_err)]
    pub fn activate(&mut self, oracle: Pubkey, kass_mint: Pubkey) -> TransactionResult {
        self.activate_at(oracle, kass_mint, 0)
    }

    /// Send an `Activate` instruction for the `outcome_index` sub-market. Returns
    /// the LiteSVM result.
    #[allow(clippy::result_large_err)]
    pub fn activate_at(
        &mut self,
        oracle: Pubkey,
        kass_mint: Pubkey,
        outcome_index: u8,
    ) -> TransactionResult {
        let ix = kassandra_market_sdk::ix::activate(
            &self.payer.pubkey(),
            &oracle,
            &kass_mint,
            outcome_index,
        );
        self.send_many(&[ix], &[])
    }

    /// Send a `ClaimLp` instruction (permissionless). Derives the `lp_vault` and
    /// the `contribution` PDA from `market` + `contributor`, distributing the
    /// pro-rata LP to `contributor_lp_ata`. Returns the LiteSVM result.
    #[allow(clippy::result_large_err)]
    pub fn claim_lp(
        &mut self,
        market: Pubkey,
        contributor: Pubkey,
        contributor_lp_ata: Pubkey,
    ) -> litesvm::types::TransactionResult {
        let (lp_vault, _) = kassandra_market_sdk::pda::lp_vault(&market);
        let (contribution, _) = kassandra_market_sdk::pda::contribution(&market, &contributor);
        let ix = kassandra_market_sdk::ix::claim_lp(
            &market,
            &lp_vault,
            &contribution,
            &contributor_lp_ata,
            &contributor,
        );
        self.send(ix, &[])
    }

    /// Attack variant of `claim_lp` with an explicit `Contribution` account:
    /// derives the `lp_vault` from `market` but pairs it with an arbitrary
    /// `contribution` PDA (e.g. one belonging to a DIFFERENT market), to prove the
    /// `contribution.market != market` cross-market guard fires. Returns the result.
    #[allow(clippy::result_large_err)]
    pub fn claim_lp_with_contribution(
        &mut self,
        market: Pubkey,
        contribution: Pubkey,
        dest_ata: Pubkey,
    ) -> litesvm::types::TransactionResult {
        let (lp_vault, _) = kassandra_market_sdk::pda::lp_vault(&market);
        // The cross-market guard fires before the contributor binding is checked, so
        // the placeholder contributor (`dest`) is never validated.
        let ix = kassandra_market_sdk::ix::claim_lp(
            &market,
            &lp_vault,
            &contribution,
            &dest_ata,
            &dest_ata,
        );
        self.send(ix, &[])
    }

    /// Attack variant of `claim_lp`: derives the `Contribution` PDA from the
    /// recorded `contributor` but sends the LP to an arbitrary `dest_ata`. Used
    /// to prove a cranker cannot redirect a contributor's LP (wrong owner) or
    /// point at a non-LP-mint account.
    #[allow(clippy::result_large_err)]
    pub fn claim_lp_to(
        &mut self,
        market: Pubkey,
        contributor: Pubkey,
        dest_ata: Pubkey,
    ) -> litesvm::types::TransactionResult {
        let (lp_vault, _) = kassandra_market_sdk::pda::lp_vault(&market);
        let (contribution, _) = kassandra_market_sdk::pda::contribution(&market, &contributor);
        // Recorded contributor is `contributor`; the wrong-dest guard fires before
        // the contributor binding is checked, so pass the real contributor here.
        let ix = kassandra_market_sdk::ix::claim_lp(
            &market,
            &lp_vault,
            &contribution,
            &dest_ata,
            &contributor,
        );
        self.send(ix, &[])
    }

    /// Rewrite a fabricated Kassandra oracle to the terminal `Resolved` phase with
    /// the given `resolved_option` (options_count = 2). Lets a resolve test move an
    /// activated market's oracle to a winning binary outcome.
    pub fn set_oracle_resolved(&mut self, oracle: Pubkey, resolved_option: u8) {
        self.set_oracle_resolved_full(oracle, 2, resolved_option);
    }

    /// Rewrite a fabricated Kassandra oracle to the terminal `Resolved` phase with
    /// an explicit `options_count` + winning `resolved_option`. Lets a categorical
    /// resolve test move a 3-option oracle to a chosen outcome.
    pub fn set_oracle_resolved_full(
        &mut self,
        oracle: Pubkey,
        options_count: u8,
        resolved_option: u8,
    ) {
        let data = kass_oracle_bytes(
            options_count,
            kassandra_market_program::kass_oracle::PHASE_RESOLVED,
            resolved_option,
        );
        let lamports = self.svm.minimum_balance_for_rent_exemption(data.len());
        self.svm
            .set_account(
                oracle,
                solana_sdk::account::Account {
                    lamports,
                    data,
                    owner: kass_oracle_owner(),
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
    }

    /// Send a `ResolveMarket` instruction (permissionless). Derives the
    /// conditional_vault event-authority; a 1.4M-CU budget is prepended for the
    /// `resolve_question` CPI. Returns the LiteSVM result.
    #[allow(clippy::result_large_err)]
    pub fn resolve_market(
        &mut self,
        market: Pubkey,
        oracle: Pubkey,
        question: Pubkey,
    ) -> TransactionResult {
        use kassandra_market_sdk::metadao as md;
        let (cv_event_auth, _) = md::event_authority(&md::CONDITIONAL_VAULT_ID);
        let ix =
            kassandra_market_sdk::ix::resolve_market(&market, &oracle, &question, &cv_event_auth);
        self.send_many(&[ix], &[])
    }

    /// Send a `CollectFee` instruction (permissionless crank). Derives every
    /// account from `oracle` + `kass_mint` + the given `fee_destination`; a 1.4M-CU
    /// budget is prepended for the remove_liquidity → redeem → transfer CPIs.
    #[allow(clippy::result_large_err)]
    pub fn collect_fee(
        &mut self,
        oracle: Pubkey,
        kass_mint: Pubkey,
        fee_destination: Pubkey,
    ) -> TransactionResult {
        let ix = kassandra_market_sdk::ix::collect_fee(&oracle, &kass_mint, &fee_destination, 0);
        self.send_many(&[ix], &[])
    }

    /// Read the `Config` singleton's `fee_destination` (a KASS token account).
    pub fn config_fee_destination(&self) -> Pubkey {
        use kassandra_market_program::state::Config;
        let (config, _) = kassandra_market_sdk::pda::config();
        Pubkey::new_from_array(self.read_pod::<Config>(config).fee_destination.to_bytes())
    }

    /// Client `amm::swap`: `user` swaps `input` of one conditional leg for the
    /// other (fee accrues to the pool, growing the LP position's value). `user`
    /// owns `user_cyes`/`user_cno`. Returns the LiteSVM result.
    #[allow(clippy::result_large_err, clippy::too_many_arguments)]
    pub fn user_swap(
        &mut self,
        user: &Keypair,
        refs: &MetaDaoRefs,
        user_cyes: Pubkey,
        user_cno: Pubkey,
        swap_type: kassandra_market_sdk::metadao::SwapType,
        input_amount: u64,
        min_out: u64,
    ) -> TransactionResult {
        use kassandra_market_sdk::metadao as md;
        let ix = md::swap(
            &user.pubkey(),
            &refs.yes_mint,
            &refs.no_mint,
            &user_cyes,
            &user_cno,
            swap_type,
            input_amount,
            min_out,
        );
        self.send_many(&[ix], &[user])
    }

    /// Client `split_tokens`: `user` splits `amount` KASS out of `user_kass_ata`
    /// into the vault, receiving `amount` of BOTH cYES and cNO into
    /// `user_cyes`/`user_cno`. Returns the LiteSVM result.
    #[allow(clippy::result_large_err, clippy::too_many_arguments)]
    pub fn user_split(
        &mut self,
        user: &Keypair,
        refs: &MetaDaoRefs,
        user_kass_ata: Pubkey,
        user_cyes: Pubkey,
        user_cno: Pubkey,
        amount: u64,
    ) -> TransactionResult {
        use kassandra_market_sdk::metadao as md;
        let ix = md::split_tokens(
            &user.pubkey(),
            &refs.question,
            &refs.vault,
            &refs.vault_underlying_ata,
            &user_kass_ata,
            &refs.yes_mint,
            &refs.no_mint,
            &user_cyes,
            &user_cno,
            amount,
        );
        self.send_many(&[ix], &[user])
    }

    /// Client `redeem_tokens`: `user` burns their full cYES/cNO balances and
    /// receives the resolved payout underlying into `user_kass_ata`. Returns the
    /// LiteSVM result.
    #[allow(clippy::result_large_err)]
    pub fn redeem(
        &mut self,
        user: &Keypair,
        refs: &MetaDaoRefs,
        user_kass_ata: Pubkey,
        user_cyes: Pubkey,
        user_cno: Pubkey,
    ) -> TransactionResult {
        use kassandra_market_sdk::metadao as md;
        let ix = md::redeem_tokens(
            &user.pubkey(),
            &refs.question,
            &refs.vault,
            &refs.vault_underlying_ata,
            &user_kass_ata,
            &refs.yes_mint,
            &refs.no_mint,
            &user_cyes,
            &user_cno,
        );
        self.send_many(&[ix], &[user])
    }

    /// Send an `UpdateConfig` instruction. The `authority` signs as an extra
    /// signer (the payer remains fee-payer). Threads a default fee (100 bps) and a
    /// freshly fabricated KASS `fee_destination` on `kass_mint`; use
    /// [`TestCtx::update_config_full`] to control the fee args.
    #[allow(clippy::result_large_err)]
    pub fn update_config(
        &mut self,
        authority: &solana_sdk::signature::Keypair,
        kass_mint: Pubkey,
        min_liquidity: u64,
    ) -> litesvm::types::TransactionResult {
        let fee_destination = self.create_token_account(kass_mint, authority.pubkey(), 0);
        self.update_config_full(authority, min_liquidity, 100, fee_destination)
    }

    /// Full `UpdateConfig` with explicit `fee_bps` + `fee_destination`.
    #[allow(clippy::result_large_err)]
    pub fn update_config_full(
        &mut self,
        authority: &solana_sdk::signature::Keypair,
        min_liquidity: u64,
        fee_bps: u16,
        fee_destination: Pubkey,
    ) -> litesvm::types::TransactionResult {
        let ix = kassandra_market_sdk::ix::update_config(
            &authority.pubkey(),
            min_liquidity,
            fee_bps,
            &fee_destination,
        );
        self.send(ix, &[authority])
    }
}

impl Default for TestCtx {
    fn default() -> Self {
        Self::new()
    }
}

/// Derived addresses of a client-composed MetaDAO market (the precondition for
/// `activate`), returned by [`TestCtx::compose_metadao_market`].
pub struct MetaDaoRefs {
    pub question: Pubkey,
    pub vault: Pubkey,
    pub vault_underlying_ata: Pubkey,
    pub yes_mint: Pubkey,
    pub no_mint: Pubkey,
    pub amm: Pubkey,
    pub lp_mint: Pubkey,
    pub amm_vault_base: Pubkey,
    pub amm_vault_quote: Pubkey,
}

/// Decode a LiteSVM transaction error into its `Custom(u32)` code, if any.
pub fn custom_code(res: &TransactionResult) -> Option<u32> {
    match res {
        Err(meta) => match &meta.err {
            TransactionError::InstructionError(_, InstructionError::Custom(code)) => Some(*code),
            _ => None,
        },
        Ok(_) => None,
    }
}

//! Shared test harness for the Kassandra dispute-core program.
//!
//! Dispute-core instructions (create / propose / advance-phase / ...) do not
//! exist yet. To exercise the instructions that *will* exist, every test needs
//! an oracle that is already sitting in a disputed state with conflicting
//! proposers. This harness fabricates that state directly: it writes
//! program-owned account data into LiteSVM with [`LiteSVM::set_account`],
//! bypassing the (not-yet-built) create/propose flow.
//!
//! # PDA seed conventions (CONTRACT)
//!
//! These seeds are part of the program's public contract. Later instruction
//! tasks MUST derive their PDAs with exactly these seeds (see
//! [`TestCtx::oracle_pda`] / [`TestCtx::proposer_pda`]):
//!
//! * **Oracle PDA** — seeds `[b"oracle", &nonce.to_le_bytes()]`, where
//!   `nonce: u64`; program = [`kassandra_oracles_program::ID`].
//! * **Proposer PDA** — seeds
//!   `[b"proposer", oracle_pubkey.as_ref(), authority_pubkey.as_ref()]`;
//!   program = [`kassandra_oracles_program::ID`].
//! * **Stake vault** — an SPL token account on the KASS mint whose **owner
//!   (token authority) is the Oracle PDA**, so the program can sign transfers
//!   out of it later via the oracle PDA seeds. The vault's address is an
//!   arbitrary fresh pubkey; it is stored in `Oracle.stake_vault`. Because it
//!   is *not* a PDA, downstream program code must **READ `oracle.stake_vault`**
//!   to learn the vault address — it must never re-derive it.
//!
//! The seeded vault's token balance always equals `Oracle.total_oracle_stake`
//! (the sum of all proposer bonds), so downstream payout/slash logic sees a
//! self-consistent fixture.

#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};

use bytemuck::Zeroable;
use kassandra_oracles_program::config::{
    CHALLENGE_FAIL_USDC_FEE_DEN, CHALLENGE_FAIL_USDC_FEE_NUM, CHALLENGE_SUCCESS_KASS_FEE_DEN,
    CHALLENGE_SUCCESS_KASS_FEE_NUM, EMISSION_DEN, EMISSION_NUM, FLIP_SLASH_DEN, FLIP_SLASH_NUM,
    MARKET_THRESHOLD_DEN, MARKET_THRESHOLD_NUM, PHASE_WINDOW, PROPOSAL_WINDOW, THRESHOLD_DEN,
    THRESHOLD_NUM, TOTAL_SUPPLY_CAP,
};
use kassandra_oracles_program::cpi::metadao_v06 as md6;
use kassandra_oracles_program::instruction::Ix;
use kassandra_oracles_program::reward;
use kassandra_oracles_program::state::{
    AccountType, AiClaim, Fact, FactVote, Market, Oracle, Phase, Proposer, CLAIM_OPTION_NONE,
    VOTE_APPROVE, VOTE_DUPLICATE,
};
use litesvm::{types::TransactionResult, LiteSVM};
use solana_account::Account;
use solana_clock::Clock;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_sdk_ids::system_program;
use solana_signer::Signer;
use solana_transaction::Transaction;
use spl_token::{
    solana_program::{program_option::COption, program_pack::Pack},
    state::{Account as TokenAccount, AccountState, Mint},
    ID as TOKEN_PROGRAM_ID,
};

/// Duration (seconds) added to the current clock to compute `phase_ends_at`
/// for a freshly seeded oracle. Tests can cross it with [`TestCtx::warp`].
pub const WINDOW: i64 = 3600;
/// Default TWAP window (seconds) written into seeded oracles.
pub const TWAP_WINDOW: i64 = 600;
/// Seconds between a real oracle's creation `now` and its `deadline`, used by
/// the real-flow builders ([`TestCtx::create_real_oracle`]). The builder warps
/// this far to reach the deadline so the proposal window is open.
pub const DEADLINE_DELTA: i64 = 1_000;

/// SPL Associated Token Account program id — the DAO treasury (SW1 sweep target)
/// is the canonical KASS ATA of `dao_authority`, derived under this program.
pub const ATA_PROGRAM_ID: Pubkey =
    solana_pubkey::pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

/// KASS mint decimals.
pub const KASS_DECIMALS: u8 = 9;
/// USDC mint decimals.
pub const USDC_DECIMALS: u8 = 6;

/// Deterministic `kass_price` TWAP the harness blesses for challenge-escrow
/// tests: raw USDC per raw KASS × `1e12` (== KASS at $0.50). With a 1 KASS bond
/// (`1e9` base units) this escrows `1e9 × 5e8 / 1e12 = 500_000` USDC base units.
pub const KASS_PRICE_TWAP: u128 = 500_000_000;
/// The `kass_price` fixed-point scale (`KASS_PRICE_SCALE`), mirrored here so
/// tests compute the expected escrow without importing the program const.
pub const KASS_PRICE_SCALE: u128 = 1_000_000_000_000;

/// Expected challenger USDC escrow for `bond` KASS base units at
/// [`KASS_PRICE_TWAP`]: `bond × twap / KASS_PRICE_SCALE` (the on-chain formula).
pub fn required_escrow_usdc(bond: u64) -> u64 {
    (bond as u128 * KASS_PRICE_TWAP / KASS_PRICE_SCALE) as u64
}

/// Specification for one proposer to seed into a disputed oracle.
#[derive(Clone, Copy, Debug)]
pub struct ProposerSpec {
    /// The categorical option this proposer originally proposed.
    pub option: u8,
    /// KASS bond (base units) this proposer has locked.
    pub bond: u64,
}

/// A proposer that was seeded into an oracle, with everything later tests need
/// to act as that proposer (sign instructions, re-derive its PDA, read state).
pub struct SeededProposer {
    /// Signing authority for this proposer.
    pub authority: Keypair,
    /// The Proposer PDA holding this proposer's on-chain account.
    pub pda: Pubkey,
    /// Original proposed option.
    pub option: u8,
    /// Locked KASS bond (base units).
    pub bond: u64,
}

/// One seeded oracle and the bookkeeping needed to interact with it later.
pub struct SeededOracle {
    /// Oracle PDA.
    pub pda: Pubkey,
    /// Bump for the Oracle PDA.
    pub bump: u8,
    /// `nonce` used to derive the Oracle PDA.
    pub nonce: u64,
    /// Token account holding all KASS bonds; owner == [`SeededOracle::pda`].
    pub stake_vault: Pubkey,
    /// Proposers seeded into this oracle, in spec order.
    pub proposers: Vec<SeededProposer>,
}

/// The full set of `Protocol`-resident governable params for `set_config`,
/// now owned by the Rust SDK (`kassandra_oracles_sdk::ConfigParams`). Re-exported here
/// so existing test call sites (`ConfigParams::defaults()`, struct literals,
/// `to_payload()`) keep working unchanged.
pub use kassandra_oracles_sdk::ConfigParams;

/// LiteSVM-backed test context with KASS/USDC mints and helpers for seeding
/// disputed oracles directly into account storage.
pub struct TestCtx {
    pub svm: LiteSVM,
    pub payer: Keypair,
    pub kass_mint: Pubkey,
    pub usdc_mint: Pubkey,
    /// A KASS token account owned by the payer, funded and backed by mint supply
    /// (so a real `Burn` decrements both balance and supply). Used as the
    /// creator's burn source for `create_oracle`'s dynamic fee.
    pub payer_kass: Pubkey,
    pub program_id: Pubkey,
    /// Monotonic counter for fresh oracle nonces.
    next_nonce: u64,
    /// Seeded oracles keyed by their Oracle PDA, for later retrieval.
    oracles: HashMap<Pubkey, SeededOracle>,
    /// Whether the Protocol singleton has been initialized in this context.
    /// The real-flow builders ([`TestCtx::ensure_protocol`]) call `init_protocol`
    /// at most once per `TestCtx`; the singleton is then shared by every real
    /// oracle created afterward.
    protocol_initialized: bool,
    /// Records the compute units of every successful send, keyed by instruction.
    cu_meter: CuMeter,
}

impl TestCtx {
    /// Add `delta` base units to an existing SPL token account's balance,
    /// rewriting its account data in place.
    fn add_token_balance(&mut self, addr: Pubkey, delta: u64) {
        let acc = self
            .svm
            .get_account(&addr)
            .unwrap_or_else(|| panic!("token account {addr} not found"));
        let mut state = TokenAccount::unpack(&acc.data).expect("not a token account");
        state.amount = state.amount.checked_add(delta).expect("balance overflow");
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

    /// Remove `delta` base units from an existing SPL token account's balance
    /// (saturating), rewriting its account data in place. Used to model KASS that
    /// physically left a vault (e.g. the `settle_challenge` `kass_fee` payout).
    fn sub_token_balance(&mut self, addr: Pubkey, delta: u64) {
        let acc = self
            .svm
            .get_account(&addr)
            .unwrap_or_else(|| panic!("token account {addr} not found"));
        let mut state = TokenAccount::unpack(&acc.data).expect("not a token account");
        state.amount = state.amount.saturating_sub(delta);
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
    // ----- low-level fabrication ---------------------------------------------

    /// Write a program-owned account with rent-exempt lamports for `data.len()`.
    fn set_program_account(&mut self, key: Pubkey, data: Vec<u8>) {
        let lamports = self.svm.minimum_balance_for_rent_exemption(data.len());
        self.svm
            .set_account(
                key,
                Account {
                    lamports,
                    data,
                    owner: self.program_id,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
    }

    /// Fabricate an initialized SPL mint with the given decimals and authority.
    fn create_mint(&mut self, decimals: u8, authority: Pubkey) -> Pubkey {
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

    /// Increase an existing fabricated mint's `supply` by `delta`, rewriting its
    /// account data in place. Keeps fabricated token balances backed by supply
    /// so a real `Burn` does not underflow.
    /// Current SPL supply of the canonical KASS mint.
    pub fn kass_supply(&self) -> u64 {
        let acc = self.svm.get_account(&self.kass_mint).expect("kass mint");
        Mint::unpack(&acc.data).expect("not a mint").supply
    }

    /// The `reward_emission` `create_oracle` would mint RIGHT NOW: `(cap −
    /// supply)·EMISSION_NUM / EMISSION_DEN` (u128 floor), mirroring the program's
    /// `compute_reward_emission`. Emission is ON by default, so create-oracle
    /// tests use this to size the vault/pool/supply deltas. Call BEFORE the
    /// create (supply changes after the mint).
    pub fn expected_creation_emission(&self) -> u64 {
        let reservoir = TOTAL_SUPPLY_CAP.saturating_sub(self.kass_supply());
        ((reservoir as u128) * (EMISSION_NUM as u128) / (EMISSION_DEN as u128)) as u64
    }

    fn add_mint_supply(&mut self, mint: Pubkey, delta: u64) {
        let acc = self
            .svm
            .get_account(&mint)
            .unwrap_or_else(|| panic!("mint {mint} not found"));
        let mut state = Mint::unpack(&acc.data).expect("not a mint");
        state.supply = state.supply.checked_add(delta).expect("supply overflow");
        let mut data = vec![0u8; Mint::LEN];
        state.pack_into_slice(&mut data);
        self.svm
            .set_account(
                mint,
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

    /// Fabricate an initialized SPL token account holding `amount` of `mint`,
    /// owned (token authority) by `owner`. Returns its address.
    fn create_token_account(&mut self, mint: Pubkey, owner: Pubkey, amount: u64) -> Pubkey {
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
}

impl Default for TestCtx {
    fn default() -> Self {
        Self::new()
    }
}

mod ctx_setup;
mod ctx_governance;
mod ctx_oracle;
mod ctx_seeding;
mod ctx_accessors;
mod ctx_send;
#[allow(unused_imports)]
pub use ctx_send::*;
mod claim_seed;
#[allow(unused_imports)]
pub use claim_seed::*;
mod claim_ix;
mod ix_builders;
#[allow(unused_imports)]
pub use ix_builders::*;

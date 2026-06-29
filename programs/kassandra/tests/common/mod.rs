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
//!   `nonce: u64`; program = [`kassandra_program::ID`].
//! * **Proposer PDA** — seeds
//!   `[b"proposer", oracle_pubkey.as_ref(), authority_pubkey.as_ref()]`;
//!   program = [`kassandra_program::ID`].
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

use std::collections::HashMap;

use bytemuck::Zeroable;
use kassandra_program::config::{
    FLIP_SLASH_DEN, FLIP_SLASH_NUM, MARKET_THRESHOLD_DEN, MARKET_THRESHOLD_NUM, PHASE_WINDOW,
    PROPOSAL_WINDOW, THRESHOLD_DEN, THRESHOLD_NUM,
};
use kassandra_program::state::{AccountType, Oracle, Phase, Proposer, CLAIM_OPTION_NONE};
use litesvm::{types::TransactionResult, LiteSVM};
use solana_sdk::{
    account::Account,
    clock::Clock,
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    system_program,
    transaction::Transaction,
};
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

/// KASS mint decimals.
pub const KASS_DECIMALS: u8 = 9;
/// USDC mint decimals.
pub const USDC_DECIMALS: u8 = 6;

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
}

impl TestCtx {
    /// Build a fresh context: a funded payer plus KASS (9 dp) and USDC (6 dp)
    /// mints, both with the payer as mint authority, and the compiled
    /// `kassandra_program` deployed so tests can submit real transactions via
    /// [`TestCtx::send`].
    ///
    /// The `.so` is `include_bytes!`'d at compile time, so `just build`
    /// (`cargo build-sbf`) must run **before** `cargo test`.
    pub fn new() -> Self {
        let mut svm = LiteSVM::new();
        let payer = Keypair::new();
        svm.airdrop(&payer.pubkey(), 1_000_000_000_000).unwrap();

        let program_id = Pubkey::new_from_array(kassandra_program::ID);
        svm.add_program(
            program_id,
            include_bytes!("../../../../target/deploy/kassandra_program.so"),
        );

        let mut ctx = Self {
            svm,
            payer,
            kass_mint: Pubkey::default(),
            usdc_mint: Pubkey::default(),
            payer_kass: Pubkey::default(),
            program_id,
            next_nonce: 0,
            oracles: HashMap::new(),
            protocol_initialized: false,
        };

        let authority = ctx.payer.pubkey();
        ctx.kass_mint = ctx.create_mint(KASS_DECIMALS, authority);
        ctx.usdc_mint = ctx.create_mint(USDC_DECIMALS, authority);
        // Bankroll the payer with KASS backed by real mint supply so the
        // creation-fee burn reduces both the balance AND the supply.
        let payer = ctx.payer.pubkey();
        ctx.payer_kass = ctx.fund_kass_minted(payer, 1_000_000_000_000_000);
        ctx
    }

    // ----- seed-derivation helpers (part of the program contract) -----------

    /// Derive the Oracle PDA from a `nonce`: seeds `[b"oracle", nonce_le]`.
    pub fn oracle_pda(program_id: &Pubkey, nonce: u64) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[b"oracle", &nonce.to_le_bytes()], program_id)
    }

    /// Derive the Protocol singleton PDA: seeds `[b"protocol"]`.
    pub fn protocol_pda(program_id: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[b"protocol"], program_id)
    }

    /// Derive the stake-vault PDA for an oracle: seeds `[b"vault", oracle]`.
    /// The vault is an SPL token account on the KASS mint whose authority is the
    /// oracle PDA, created by `create_oracle`.
    pub fn stake_vault_pda(program_id: &Pubkey, oracle: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[b"vault", oracle.as_ref()], program_id)
    }

    /// Derive the Proposer PDA: seeds `[b"proposer", oracle, authority]`.
    pub fn proposer_pda(program_id: &Pubkey, oracle: &Pubkey, authority: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[b"proposer", oracle.as_ref(), authority.as_ref()],
            program_id,
        )
    }

    /// Derive the Fact PDA: seeds `[b"fact", oracle, content_hash]`.
    pub fn fact_pda(program_id: &Pubkey, oracle: &Pubkey, content_hash: &[u8; 32]) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[b"fact", oracle.as_ref(), content_hash.as_ref()],
            program_id,
        )
    }

    /// Derive the FactVote PDA: seeds `[b"vote", fact, voter]`.
    pub fn vote_pda(program_id: &Pubkey, fact: &Pubkey, voter: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[b"vote", fact.as_ref(), voter.as_ref()], program_id)
    }

    // ----- real instruction helpers -----------------------------------------

    /// Send a real `InitProtocol` instruction with `admin == payer`, recording
    /// the harness KASS/USDC mints. Returns the Protocol singleton PDA. The
    /// returned [`TransactionResult`] lets tests assert success or the
    /// double-init / wrong-PDA failure paths.
    #[allow(clippy::result_large_err)]
    pub fn init_protocol(&mut self) -> (Pubkey, TransactionResult) {
        let (protocol_pda, _) = Self::protocol_pda(&self.program_id);
        let ix = self.init_protocol_ix(protocol_pda);
        let res = self.send(ix, &[]);
        (protocol_pda, res)
    }

    /// Build an `InitProtocol` instruction targeting `protocol` (so tests can
    /// pass a deliberately wrong PDA). Admin = payer (fee payer signs).
    pub fn init_protocol_ix(&self, protocol: Pubkey) -> Instruction {
        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(protocol, false),
                AccountMeta::new(self.payer.pubkey(), true),
                AccountMeta::new_readonly(self.kass_mint, false),
                AccountMeta::new_readonly(self.usdc_mint, false),
                AccountMeta::new_readonly(system_program::ID, false),
            ],
            data: vec![kassandra_program::instruction::Ix::InitProtocol as u8],
        }
    }

    /// Stand-in governance keys for F1 tests: an arbitrary "Squads vault" PDA
    /// and an arbitrary "futarchy Dao" pubkey. F1's `set_governance` stores
    /// whatever is passed (the real Squads/futarchy setup is F6), so any
    /// distinct non-zero pubkeys suffice. Deterministic per call site via the
    /// supplied `tag`.
    pub fn stand_in_governance(tag: u8) -> (Pubkey, Pubkey) {
        let dao_authority = Pubkey::new_from_array([tag; 32]);
        let kass_dao = Pubkey::new_from_array([tag.wrapping_add(1).max(1); 32]);
        (dao_authority, kass_dao)
    }

    /// Send a real `SetGovernance` instruction signed by `authority`, recording
    /// `dao_authority` (Squads vault) + `kass_dao` (futarchy Dao) in the
    /// Protocol. Returns the Protocol PDA + result so tests can assert success
    /// or the authorization/one-shot rejection paths.
    #[allow(clippy::result_large_err)]
    pub fn set_governance(
        &mut self,
        authority: &Keypair,
        dao_authority: Pubkey,
        kass_dao: Pubkey,
    ) -> (Pubkey, TransactionResult) {
        let (protocol_pda, _) = Self::protocol_pda(&self.program_id);
        let ix = self.set_governance_ix(protocol_pda, authority.pubkey(), dao_authority, kass_dao);
        // The payer is always a signer; only co-sign `authority` when it differs.
        let res = if authority.pubkey() == self.payer.pubkey() {
            self.send(ix, &[])
        } else {
            self.send(ix, &[authority])
        };
        (protocol_pda, res)
    }

    /// Build a `SetGovernance` instruction. Exposes `protocol`/`authority` so
    /// tests can pass a wrong signer. Payload = `dao_authority ++ kass_dao`.
    pub fn set_governance_ix(
        &self,
        protocol: Pubkey,
        authority: Pubkey,
        dao_authority: Pubkey,
        kass_dao: Pubkey,
    ) -> Instruction {
        let mut data = Vec::with_capacity(1 + 64);
        data.push(kassandra_program::instruction::Ix::SetGovernance as u8);
        data.extend_from_slice(&dao_authority.to_bytes());
        data.extend_from_slice(&kass_dao.to_bytes());
        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(protocol, false),
                AccountMeta::new_readonly(authority, true),
            ],
            data,
        }
    }

    /// Send a real `CreateOracle` instruction with `creator == payer`, using the
    /// harness KASS/USDC mints and the protocol singleton. Returns the Oracle PDA
    /// derived from `nonce`. `init_protocol` must have been called first. The
    /// returned [`TransactionResult`] lets tests assert success or the various
    /// rejection paths.
    #[allow(clippy::result_large_err)]
    pub fn create_oracle(
        &mut self,
        nonce: u64,
        options_count: u8,
        deadline: i64,
        twap_window: i64,
        prompt_hash: [u8; 32],
    ) -> (Pubkey, TransactionResult) {
        let (oracle_pda, _) = Self::oracle_pda(&self.program_id, nonce);
        let ix = self.create_oracle_ix(
            nonce,
            options_count,
            deadline,
            twap_window,
            prompt_hash,
            oracle_pda,
            self.kass_mint,
            self.usdc_mint,
        );
        let res = self.send(ix, &[]);
        (oracle_pda, res)
    }

    /// Build a `CreateOracle` instruction. Exposes the oracle account and the
    /// KASS/USDC mints as parameters so tests can pass deliberately wrong values
    /// (mint spoof, etc.). Creator = payer (fee payer signs, pays rent).
    #[allow(clippy::too_many_arguments)]
    pub fn create_oracle_ix(
        &self,
        nonce: u64,
        options_count: u8,
        deadline: i64,
        twap_window: i64,
        prompt_hash: [u8; 32],
        oracle: Pubkey,
        kass_mint: Pubkey,
        usdc_mint: Pubkey,
    ) -> Instruction {
        let (protocol_pda, _) = Self::protocol_pda(&self.program_id);
        let (stake_vault, _) = Self::stake_vault_pda(&self.program_id, &oracle);

        let mut data = Vec::with_capacity(1 + 57);
        data.push(kassandra_program::instruction::Ix::CreateOracle as u8);
        data.extend_from_slice(&nonce.to_le_bytes());
        data.extend_from_slice(&prompt_hash);
        data.push(options_count);
        data.extend_from_slice(&deadline.to_le_bytes());
        data.extend_from_slice(&twap_window.to_le_bytes());

        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(protocol_pda, false),
                AccountMeta::new(oracle, false),
                AccountMeta::new(stake_vault, false),
                AccountMeta::new(self.payer.pubkey(), true),
                AccountMeta::new(kass_mint, false),
                AccountMeta::new_readonly(usdc_mint, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
                AccountMeta::new_readonly(system_program::ID, false),
                AccountMeta::new(self.payer_kass, false),
            ],
            data,
        }
    }

    /// Send a real `Propose` instruction registering `authority`'s proposal
    /// (`option` + KASS `bond`) against `oracle`. Airdrops the authority SOL for
    /// rent and funds it a KASS token account holding `bond` (the bond source).
    /// `authority` co-signs. Returns the Proposer PDA + result. The oracle must
    /// already be in [`Phase::Proposal`] (i.e. created via `create_oracle`).
    #[allow(clippy::result_large_err)]
    pub fn propose(
        &mut self,
        oracle: Pubkey,
        authority: &Keypair,
        option: u8,
        bond: u64,
    ) -> (Pubkey, TransactionResult) {
        // Fund the authority: SOL for the Proposer-PDA rent, and a KASS account
        // holding the bond. Fund at least 1 base unit so the bond==0 path still
        // has a valid source account.
        self.svm
            .airdrop(&authority.pubkey(), 1_000_000_000)
            .unwrap();
        let authority_kass = self.fund_kass(authority, bond.max(1));
        let (proposer_pda, _) = Self::proposer_pda(&self.program_id, &oracle, &authority.pubkey());
        let (vault, _) = Self::stake_vault_pda(&self.program_id, &oracle);
        let ix = self.propose_ix(
            oracle,
            proposer_pda,
            authority.pubkey(),
            authority_kass,
            vault,
            option,
            bond,
        );
        let res = self.send(ix, &[authority]);
        (proposer_pda, res)
    }

    /// Build a `Propose` instruction with the locked-in account order. Exposes
    /// the proposer/authority-KASS/vault accounts so tests can pass deliberately
    /// wrong values.
    #[allow(clippy::too_many_arguments)]
    pub fn propose_ix(
        &self,
        oracle: Pubkey,
        proposer: Pubkey,
        authority: Pubkey,
        authority_kass: Pubkey,
        vault: Pubkey,
        option: u8,
        bond: u64,
    ) -> Instruction {
        let mut data = Vec::with_capacity(1 + 9);
        data.push(kassandra_program::instruction::Ix::Propose as u8);
        data.push(option);
        data.extend_from_slice(&bond.to_le_bytes());

        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(oracle, false),
                AccountMeta::new(proposer, false),
                AccountMeta::new(authority, true),
                AccountMeta::new(authority_kass, false),
                AccountMeta::new(vault, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
                AccountMeta::new_readonly(system_program::ID, false),
            ],
            data,
        }
    }

    /// Build a `FinalizeProposals` instruction: `[0] oracle(w)` followed by the
    /// given proposer accounts as a READ-ONLY tail. Exposes the full proposer
    /// slice so tests can pass a subset, a duplicate, or a foreign-oracle account.
    pub fn finalize_proposals_ix(&self, oracle: Pubkey, proposers: &[Pubkey]) -> Instruction {
        let mut accounts = Vec::with_capacity(1 + proposers.len());
        accounts.push(AccountMeta::new(oracle, false));
        for p in proposers {
            accounts.push(AccountMeta::new_readonly(*p, false));
        }
        Instruction {
            program_id: self.program_id,
            accounts,
            data: vec![kassandra_program::instruction::Ix::FinalizeProposals as u8],
        }
    }

    // ----- real-flow builders ------------------------------------------------

    /// Ensure the Protocol singleton exists, calling `init_protocol` exactly
    /// once per `TestCtx` (idempotent thereafter). Many real oracles share one
    /// protocol, so this guards against a double-init `AlreadyInitialized`
    /// failure when several `create_real_oracle` calls run in the same context.
    /// Returns the Protocol PDA.
    pub fn ensure_protocol(&mut self) -> Pubkey {
        let (protocol_pda, _) = Self::protocol_pda(&self.program_id);
        if !self.protocol_initialized {
            let (_p, res) = self.init_protocol();
            assert!(res.is_ok(), "init_protocol should succeed: {res:?}");
            self.protocol_initialized = true;
        }
        protocol_pda
    }

    /// Real-flow oracle builder: `init_protocol` (once per ctx) + a real
    /// `create_oracle` with a near `deadline`, then warps to the deadline so the
    /// proposal window is open. Uses a fresh nonce from the internal counter and
    /// records the oracle in the bookkeeping map so [`TestCtx::seeded`],
    /// [`TestCtx::proposers`], and the stake-vault accessor work for it exactly
    /// like a `seed_disputed_oracle` oracle. Returns the Oracle PDA.
    pub fn create_real_oracle(&mut self, options_count: u8, twap_window: i64) -> Pubkey {
        self.ensure_protocol();
        let nonce = self.next_nonce;
        self.next_nonce += 1;
        let (oracle, bump) = Self::oracle_pda(&self.program_id, nonce);
        let deadline = self.now() + DEADLINE_DELTA;
        let (created, res) =
            self.create_oracle(nonce, options_count, deadline, twap_window, [0x42; 32]);
        assert!(res.is_ok(), "create_oracle should succeed: {res:?}");
        debug_assert_eq!(created, oracle);
        // Warp to the deadline: proposals open at `deadline`, window now open.
        self.warp(DEADLINE_DELTA);

        let (stake_vault, _) = Self::stake_vault_pda(&self.program_id, &oracle);
        self.oracles.insert(
            oracle,
            SeededOracle {
                pda: oracle,
                bump,
                nonce,
                stake_vault,
                proposers: Vec::new(),
            },
        );
        oracle
    }

    /// Real-flow proposer registration: funds a fresh authority and sends a real
    /// `propose` (`option` + KASS `bond`) against `oracle`. Records the proposer
    /// in the oracle's bookkeeping (so [`TestCtx::proposers`] and
    /// [`TestCtx::finalize_proposals_real`] see the full set) and returns the
    /// authority keypair + Proposer PDA. Panics if the propose fails.
    pub fn propose_real(&mut self, oracle: Pubkey, option: u8, bond: u64) -> (Keypair, Pubkey) {
        let authority = Keypair::new();
        let (pda, res) = self.propose(oracle, &authority, option, bond);
        assert!(
            res.is_ok(),
            "propose(option={option}) should succeed: {res:?}"
        );
        if let Some(seeded) = self.oracles.get_mut(&oracle) {
            seeded.proposers.push(SeededProposer {
                authority: authority.insecure_clone(),
                pda,
                option,
                bond,
            });
        }
        (authority, pda)
    }

    /// Real-flow proposal finalization: warp past the proposal window, then send
    /// a real `finalize_proposals` carrying the FULL tracked proposer set as the
    /// read-only tail. Returns the transaction result so callers can assert the
    /// resolve / open-dispute outcome (or a rejection).
    #[allow(clippy::result_large_err)]
    pub fn finalize_proposals_real(&mut self, oracle: Pubkey) -> TransactionResult {
        self.warp(PROPOSAL_WINDOW + 1);
        let proposers: Vec<Pubkey> = self.proposers(oracle).iter().map(|p| p.pda).collect();
        let ix = self.finalize_proposals_ix(oracle, &proposers);
        self.send(ix, &[])
    }

    /// Real-flow analogue of [`TestCtx::seed_disputed_oracle`]: drive
    /// create_oracle → propose (one per spec) → finalize_proposals through the
    /// genuine entry points, landing the oracle in [`Phase::FactProposal`] with
    /// the proposers registered and `dispute_bond_total` set. The specs MUST
    /// contain at least two DISTINCT options (otherwise finalize_proposals
    /// resolves instead of opening a dispute); this is asserted post-finalize.
    /// Returns the Oracle PDA.
    pub fn dispute_via_real_flow(&mut self, specs: &[ProposerSpec]) -> Pubkey {
        assert!(
            specs.len() >= 2,
            "a real-flow dispute needs at least two proposers"
        );
        let max_option = specs.iter().map(|s| s.option).max().unwrap();
        let options_count = (max_option as u16 + 1).max(2) as u8;
        let oracle = self.create_real_oracle(options_count, TWAP_WINDOW);
        for spec in specs {
            self.propose_real(oracle, spec.option, spec.bond);
        }
        let res = self.finalize_proposals_real(oracle);
        assert!(res.is_ok(), "finalize_proposals should succeed: {res:?}");
        assert_eq!(
            self.oracle(oracle).phase,
            Phase::FactProposal.as_u8(),
            "dispute_via_real_flow needs >=2 distinct options to open a dispute"
        );
        oracle
    }

    // ----- seeding -----------------------------------------------------------

    /// Fabricate an oracle already in [`Phase::FactProposal`] with one proposer
    /// per spec, plus a funded stake vault. Returns the Oracle PDA.
    ///
    /// All counts and balances are kept internally consistent:
    /// `proposer_count == surviving_count == specs.len()`,
    /// `total_oracle_stake == Σ bond == vault token balance`.
    pub fn seed_disputed_oracle(&mut self, specs: &[ProposerSpec]) -> Pubkey {
        assert!(!specs.is_empty(), "need at least one proposer to dispute");

        let nonce = self.next_nonce;
        self.next_nonce += 1;
        let (oracle_pda, bump) = Self::oracle_pda(&self.program_id, nonce);

        let total_stake: u64 = specs.iter().map(|s| s.bond).sum();
        let max_option = specs.iter().map(|s| s.option).max().unwrap();
        // At least 2 options (a dispute needs ≥2), and enough to index every
        // proposed option.
        let options_count = (max_option as u16 + 1).max(2) as u8;

        // Stake vault: SPL token account on KASS, owner == oracle PDA, holding
        // exactly the summed bonds.
        let stake_vault = self.create_token_account(self.kass_mint, oracle_pda, total_stake);

        // Build and write the Oracle account.
        let now = self.now();
        let mut oracle = Oracle::zeroed();
        oracle.account_type = AccountType::Oracle.as_u8();
        oracle.creator = self.payer.pubkey().to_bytes();
        oracle.kass_mint = self.kass_mint.to_bytes();
        oracle.usdc_mint = self.usdc_mint.to_bytes();
        oracle.stake_vault = stake_vault.to_bytes();
        oracle.deadline = now;
        oracle.phase_ends_at = now + WINDOW;
        oracle.twap_window = TWAP_WINDOW;
        oracle.options_count = options_count;
        oracle.set_phase(Phase::FactProposal);
        oracle.proposer_count = specs.len() as u16;
        oracle.surviving_count = specs.len() as u16;
        oracle.fact_count = 0;
        oracle.total_oracle_stake = total_stake;
        oracle.bond_pool = 0;
        // Fixed fact-quorum denominator: Σ proposer bonds at dispute start.
        oracle.dispute_bond_total = total_stake;
        oracle.settled_count = 0;
        oracle.bump = bump;
        oracle.prompt_hash = [0x11; 32];
        // F2: snapshot the governable behavioral params from the config consts,
        // exactly as `create_oracle` would (Protocol defaults == these consts),
        // so a fabricated oracle behaves identically to a real one.
        oracle.threshold_num = THRESHOLD_NUM;
        oracle.threshold_den = THRESHOLD_DEN;
        oracle.market_threshold_num = MARKET_THRESHOLD_NUM as u64;
        oracle.market_threshold_den = MARKET_THRESHOLD_DEN as u64;
        oracle.flip_slash_num = FLIP_SLASH_NUM;
        oracle.flip_slash_den = FLIP_SLASH_DEN;
        oracle.phase_window = PHASE_WINDOW;
        oracle.proposal_window = PROPOSAL_WINDOW;
        self.set_program_account(oracle_pda, bytemuck::bytes_of(&oracle).to_vec());

        // Build and write each Proposer account.
        let mut proposers = Vec::with_capacity(specs.len());
        for spec in specs {
            let authority = Keypair::new();
            let (pda, p_bump) =
                Self::proposer_pda(&self.program_id, &oracle_pda, &authority.pubkey());

            let mut proposer = Proposer::zeroed();
            proposer.account_type = AccountType::Proposer.as_u8();
            proposer.oracle = oracle_pda.to_bytes();
            proposer.authority = authority.pubkey().to_bytes();
            proposer.bond = spec.bond;
            proposer.original_option = spec.option;
            proposer.claim_option = CLAIM_OPTION_NONE;
            proposer.disqualified = 0;
            proposer.slashed = 0;
            proposer.flipped = 0;
            proposer.bump = p_bump;
            self.set_program_account(pda, bytemuck::bytes_of(&proposer).to_vec());

            proposers.push(SeededProposer {
                authority,
                pda,
                option: spec.option,
                bond: spec.bond,
            });
        }

        self.oracles.insert(
            oracle_pda,
            SeededOracle {
                pda: oracle_pda,
                bump,
                nonce,
                stake_vault,
                proposers,
            },
        );
        oracle_pda
    }

    /// Overwrite the phase byte of an already-seeded oracle. Lets tests stand
    /// up an oracle in a non-`FactProposal` phase (e.g. `FactVoting`) to drive
    /// wrong-phase paths, without a real phase-advance instruction.
    pub fn set_phase(&mut self, oracle: Pubkey, phase: Phase) {
        let mut o = self.oracle(oracle);
        o.set_phase(phase);
        self.set_program_account(oracle, bytemuck::bytes_of(&o).to_vec());
    }

    /// Overwrite the `dispute_bond_total` of a seeded oracle. Lets tests drive
    /// the defensive zero-denominator path in `finalize_facts`.
    pub fn set_dispute_bond_total(&mut self, oracle: Pubkey, value: u64) {
        let mut o = self.oracle(oracle);
        o.dispute_bond_total = value;
        self.set_program_account(oracle, bytemuck::bytes_of(&o).to_vec());
    }

    /// Mark a seeded Proposer account as disqualified. Lets tests drive the
    /// disqualified-submitter rejection in `submit_ai_claim` and the defensive
    /// already-disqualified branch in `finalize_ai_claims` without a real slash
    /// instruction from an earlier phase.
    pub fn set_proposer_disqualified(&mut self, proposer: Pubkey) {
        let mut p = self.proposer(proposer);
        p.disqualified = 1;
        self.set_program_account(proposer, bytemuck::bytes_of(&p).to_vec());
    }

    /// Overwrite a seeded Proposer's `claim_option`. Lets `finalize_oracle`
    /// tests stand up surviving proposers with chosen post-AI-claim votes
    /// without driving the full submit/finalize-AI-claim flow.
    pub fn set_proposer_claim_option(&mut self, proposer: Pubkey, option: u8) {
        let mut p = self.proposer(proposer);
        p.claim_option = option;
        self.set_program_account(proposer, bytemuck::bytes_of(&p).to_vec());
    }

    /// Overwrite an oracle's `surviving_count`. Lets `finalize_oracle` tests
    /// keep the count consistent with a hand-disqualified proposer set (e.g.
    /// the all-disqualified dead-end) without a real slash instruction.
    pub fn set_surviving_count(&mut self, oracle: Pubkey, count: u16) {
        let mut o = self.oracle(oracle);
        o.surviving_count = count;
        self.set_program_account(oracle, bytemuck::bytes_of(&o).to_vec());
    }

    /// Overwrite an oracle's `open_challenge_count`. Lets `finalize_oracle`
    /// tests drive the `ChallengesOutstanding` gate (an unsettled challenge
    /// market) without standing up a real MetaDAO challenge.
    pub fn set_open_challenge_count(&mut self, oracle: Pubkey, count: u16) {
        let mut o = self.oracle(oracle);
        o.open_challenge_count = count;
        self.set_program_account(oracle, bytemuck::bytes_of(&o).to_vec());
    }

    /// Fabricate a program-owned account at a fresh address holding `data`.
    /// Used by type-confusion tests to stand up an account with a wrong (or
    /// missing) `account_type` tag.
    pub fn seed_program_account(&mut self, data: Vec<u8>) -> Pubkey {
        let key = Pubkey::new_unique();
        self.set_program_account(key, data);
        key
    }

    /// Fabricate a program-owned account at a SPECIFIC address holding `data`.
    /// Lets tests stand up a PDA-addressed account (e.g. an `AiClaim` at its
    /// `[b"claim", oracle, proposer]` PDA) without the create/submit flow.
    pub fn seed_program_account_at(&mut self, key: Pubkey, data: Vec<u8>) {
        self.set_program_account(key, data);
    }

    /// Create an SPL token account on the KASS mint owned by `owner` and fund
    /// it with `amount` base units of KASS. Returns the token account address.
    /// Used to bankroll a fact submitter.
    pub fn fund_kass(&mut self, owner: &Keypair, amount: u64) -> Pubkey {
        self.create_token_account(self.kass_mint, owner.pubkey(), amount)
    }

    /// Like [`TestCtx::fund_kass`] but ALSO increases the KASS mint's `supply` by
    /// `amount`, so the fabricated balance is backed by real supply. A real
    /// SPL `Burn` checks-subtracts the mint supply, so a burn source that was
    /// only fabricated (supply still 0) would underflow; this keeps them
    /// consistent for the creation-fee burn tests.
    pub fn fund_kass_minted(&mut self, owner: Pubkey, amount: u64) -> Pubkey {
        let acct = self.create_token_account(self.kass_mint, owner, amount);
        self.add_mint_supply(self.kass_mint, amount);
        acct
    }

    /// Read an SPL mint's circulating `supply` (base units).
    pub fn mint_supply(&self, mint: Pubkey) -> u64 {
        let acc = self
            .svm
            .get_account(&mint)
            .unwrap_or_else(|| panic!("mint {mint} not found"));
        Mint::unpack(&acc.data).expect("not a mint").supply
    }

    /// Retrieve the bookkeeping for a previously seeded oracle.
    pub fn seeded(&self, oracle: Pubkey) -> &SeededOracle {
        self.oracles.get(&oracle).expect("oracle not seeded")
    }

    /// Convenience: the seeded proposers for an oracle (spec order).
    pub fn proposers(&self, oracle: Pubkey) -> &[SeededProposer] {
        &self.seeded(oracle).proposers
    }

    // ----- accessors ---------------------------------------------------------

    /// Read and decode an `Oracle` account.
    pub fn oracle(&self, key: Pubkey) -> Oracle {
        self.read_pod(key)
    }

    /// Read and decode a `Proposer` account.
    pub fn proposer(&self, key: Pubkey) -> Proposer {
        self.read_pod(key)
    }

    /// Read and decode a `Fact` account.
    pub fn fact(&self, key: Pubkey) -> kassandra_program::state::Fact {
        self.read_pod(key)
    }

    /// Read and decode a `FactVote` account.
    pub fn fact_vote(&self, key: Pubkey) -> kassandra_program::state::FactVote {
        self.read_pod(key)
    }

    /// Read and decode an `AiClaim` account.
    pub fn ai_claim(&self, key: Pubkey) -> kassandra_program::state::AiClaim {
        self.read_pod(key)
    }

    /// Read and decode the `Protocol` singleton account.
    pub fn protocol(&self, key: Pubkey) -> kassandra_program::state::Protocol {
        self.read_pod(key)
    }

    /// Read an SPL token account's `(mint, owner, amount)`, with `mint`/`owner`
    /// as raw 32-byte arrays so callers can compare against `Pubkey::to_bytes()`
    /// without crossing the `solana_program` / `solana_sdk` Pubkey type boundary.
    pub fn token_account(&self, key: Pubkey) -> ([u8; 32], [u8; 32], u64) {
        let acc = self
            .svm
            .get_account(&key)
            .unwrap_or_else(|| panic!("token account {key} not found"));
        let ta = TokenAccount::unpack(&acc.data).expect("not a token account");
        (ta.mint.to_bytes(), ta.owner.to_bytes(), ta.amount)
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

    /// Read a program account and reinterpret its data as a `Pod` struct `T`.
    ///
    /// Uses [`bytemuck::pod_read_unaligned`] so correctness does not depend on
    /// the alignment of the allocator-provided account-data buffer.
    pub fn read_pod<T: bytemuck::Pod>(&self, key: Pubkey) -> T {
        let acc = self
            .svm
            .get_account(&key)
            .unwrap_or_else(|| panic!("account {key} not found"));
        bytemuck::pod_read_unaligned::<T>(&acc.data)
    }

    // ----- transaction submission --------------------------------------------

    /// Sign and submit a single-instruction transaction, returning the LiteSVM
    /// result so tests can assert `Ok`/`Err` and introspect the
    /// [`TransactionError`](solana_sdk::transaction::TransactionError).
    ///
    /// The transaction is signed by the payer (fee payer) plus every keypair in
    /// `signers`. The blockhash is expired and re-fetched on each call so that
    /// two otherwise-identical transactions (same instruction + signers) get
    /// distinct signatures and never collide as duplicates.
    #[allow(clippy::result_large_err)]
    pub fn send(&mut self, ix: Instruction, signers: &[&Keypair]) -> TransactionResult {
        // Rotate the blockhash to guarantee signature uniqueness across calls.
        self.svm.expire_blockhash();
        let blockhash = self.svm.latest_blockhash();
        let mut all_signers: Vec<&Keypair> = Vec::with_capacity(signers.len() + 1);
        all_signers.push(&self.payer);
        all_signers.extend_from_slice(signers);
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&self.payer.pubkey()),
            &all_signers,
            blockhash,
        );
        self.svm.send_transaction(tx)
    }

    /// Sign and submit a MULTI-instruction transaction (e.g. a ComputeBudget
    /// prefix plus a CPI-heavy instruction). Mirrors [`TestCtx::send`] but takes
    /// a slice of instructions. Signed by the payer plus every keypair in
    /// `signers`; the blockhash is rotated for signature uniqueness.
    #[allow(clippy::result_large_err)]
    pub fn send_many(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> TransactionResult {
        self.svm.expire_blockhash();
        let blockhash = self.svm.latest_blockhash();
        let mut all_signers: Vec<&Keypair> = Vec::with_capacity(signers.len() + 1);
        all_signers.push(&self.payer);
        all_signers.extend_from_slice(signers);
        let tx = Transaction::new_signed_with_payer(
            ixs,
            Some(&self.payer.pubkey()),
            &all_signers,
            blockhash,
        );
        self.svm.send_transaction(tx)
    }

    /// Current on-chain unix timestamp from the `Clock` sysvar.
    pub fn now(&self) -> i64 {
        self.svm.get_sysvar::<Clock>().unix_timestamp
    }

    /// Advance the `Clock`: add `seconds` to `unix_timestamp` and bump `slot`
    /// by exactly **1** (not proportional to `seconds`). This is enough to
    /// cross `phase_ends_at`, which is keyed off `unix_timestamp`.
    ///
    /// NOTE: the later TWAP tasks (11-12) reason about *slots*, so they will
    /// likely need a `warp_slots` variant that advances the slot proportionally.
    /// Not built yet (YAGNI).
    pub fn warp(&mut self, seconds: i64) {
        let mut clock = self.svm.get_sysvar::<Clock>();
        clock.unix_timestamp += seconds;
        clock.slot += 1;
        self.svm.set_sysvar(&clock);
    }

    /// Advance the `Clock` by `seconds` of unix time AND `slots` of slot height.
    /// The TWAP tasks (11-12) reason about *slots* (the MetaDAO AMM records an
    /// observation only once per `ONE_MINUTE_IN_SLOTS == 150` slots and weights
    /// the aggregator by elapsed slots), so they need to move the slot height
    /// independently of wall-clock seconds.
    pub fn warp_slots(&mut self, seconds: i64, slots: u64) {
        let mut clock = self.svm.get_sysvar::<Clock>();
        clock.unix_timestamp += seconds;
        clock.slot += slots;
        self.svm.set_sysvar(&clock);
    }

    /// Read the current `Clock` slot height.
    pub fn slot(&self) -> u64 {
        self.svm.get_sysvar::<Clock>().slot
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

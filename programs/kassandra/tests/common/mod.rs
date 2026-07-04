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

use std::collections::{BTreeMap, HashMap};

use bytemuck::Zeroable;
use kassandra_program::config::{
    CHALLENGE_FAIL_USDC_FEE_DEN, CHALLENGE_FAIL_USDC_FEE_NUM, CHALLENGE_SUCCESS_KASS_FEE_DEN,
    CHALLENGE_SUCCESS_KASS_FEE_NUM, EMISSION_DEN, EMISSION_NUM, FLIP_SLASH_DEN, FLIP_SLASH_NUM,
    MARKET_THRESHOLD_DEN, MARKET_THRESHOLD_NUM, PHASE_WINDOW, PROPOSAL_WINDOW, THRESHOLD_DEN,
    THRESHOLD_NUM, TOTAL_SUPPLY_CAP,
};
use kassandra_program::cpi::metadao_v06 as md6;
use kassandra_program::instruction::Ix;
use kassandra_program::reward;
use kassandra_program::state::{
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
/// now owned by the Rust SDK (`kassandra_sdk::ConfigParams`). Re-exported here
/// so existing test call sites (`ConfigParams::defaults()`, struct literals,
/// `to_payload()`) keep working unchanged.
pub use kassandra_sdk::ConfigParams;

/// Human-readable name for an instruction discriminant (the first payload byte),
/// so the CU meter can label every metered transaction. Mirrors
/// [`kassandra_program::instruction::Ix`].
pub fn ix_name(disc: u8) -> &'static str {
    match disc {
        0 => "submit_fact",
        1 => "vote_fact",
        2 => "finalize_facts",
        3 => "submit_ai_claim",
        4 => "open_challenge",
        5 => "settle_challenge",
        6 => "finalize_oracle",
        7 => "advance_phase",
        8 => "finalize_ai_claims",
        9 => "init_protocol",
        10 => "create_oracle",
        11 => "propose",
        12 => "finalize_proposals",
        13 => "set_governance",
        14 => "set_config",
        15 => "resolve_deadend",
        16 => "kass_price",
        17 => "claim_proposer",
        18 => "claim_fact",
        19 => "claim_fact_vote",
        20 => "close_ai_claim",
        21 => "close_market",
        22 => "sweep_oracle",
        _ => "unknown",
    }
}

/// Compute-unit meter. Every successful transaction the harness sends via
/// [`TestCtx::send`] records `(instruction, compute_units_consumed)` here, keyed
/// by the instruction discriminant. A metering test can then report per-
/// instruction CU and guard against regressions ([`TestCtx::cu_report`] /
/// [`TestCtx::cu_max`]).
#[derive(Default)]
pub struct CuMeter {
    rows: Vec<(&'static str, u64)>,
}

impl CuMeter {
    fn record(&mut self, name: &'static str, cu: u64) {
        self.rows.push((name, cu));
    }

    /// The maximum CU observed for `name`, or `None` if it was never sent.
    pub fn max(&self, name: &str) -> Option<u64> {
        self.rows
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, c)| *c)
            .max()
    }

    /// Max CU per instruction seen (alphabetical, for a stable report).
    pub fn max_by_ix(&self) -> BTreeMap<&'static str, u64> {
        let mut m: BTreeMap<&'static str, u64> = BTreeMap::new();
        for (n, c) in &self.rows {
            let e = m.entry(*n).or_insert(0);
            *e = (*e).max(*c);
        }
        m
    }

    /// A human-readable table: instruction · max CU · number of calls.
    pub fn report(&self) -> String {
        use std::fmt::Write as _;
        let mut counts: BTreeMap<&'static str, u64> = BTreeMap::new();
        for (n, _) in &self.rows {
            *counts.entry(*n).or_insert(0) += 1;
        }
        let mut s = String::from("\n=== compute-unit metering (max CU per instruction) ===\n");
        for (n, cu) in self.max_by_ix() {
            let _ = writeln!(s, "  {n:<20} {cu:>7} CU   (x{})", counts[n]);
        }
        s.push_str("=======================================================\n");
        s
    }
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
    /// Records the compute units of every successful send, keyed by instruction.
    cu_meter: CuMeter,
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

        let program_id = Pubkey::new_from_array(kassandra_program::ID.to_bytes());
        svm.add_program(
            program_id,
            include_bytes!("../../../../target/deploy/kassandra_program.so"),
        )
        .unwrap();

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
            cu_meter: CuMeter::default(),
        };

        // Mint-authority bootstrap (Task S3): the KASS mint's authority MUST be
        // the program's mint-authority PDA `[b"mint_authority"]`, so the program
        // (and ONLY the program, via the emission `MintTo` CPI) can mint KASS.
        // This harness fabricates every token balance directly (`create_token_
        // account` writes the balance; `add_mint_supply` rewrites the mint supply
        // field) and burns via the creator (token-account owner) as the SPL Burn
        // authority — NONE of which uses the mint authority — so handing the mint
        // authority to the PDA leaves all existing test funding working while
        // enabling the program's emission mint. USDC's authority stays the payer
        // (no program ever mints USDC).
        let (mint_auth, _) = Self::mint_authority_pda(&ctx.program_id);
        ctx.kass_mint = ctx.create_mint(KASS_DECIMALS, mint_auth);
        ctx.usdc_mint = ctx.create_mint(USDC_DECIMALS, ctx.payer.pubkey());
        // Bankroll the payer with KASS backed by real mint supply so the
        // creation-fee burn reduces both the balance AND the supply.
        let payer = ctx.payer.pubkey();
        ctx.payer_kass = ctx.fund_kass_minted(payer, 1_000_000_000_000_000);
        ctx
    }

    // ----- seed-derivation helpers (thin wrappers over `kassandra_sdk::pda`) --
    // The seed conventions are the program's public contract; the SDK owns the
    // derivations so there is a single source of truth. These wrappers keep the
    // harness's `*_pda` names stable for the existing test call sites.

    /// Derive the Oracle PDA from a `nonce`: seeds `[b"oracle", nonce_le]`.
    pub fn oracle_pda(program_id: &Pubkey, nonce: u64) -> (Pubkey, u8) {
        kassandra_sdk::pda::oracle(program_id, nonce)
    }

    /// Derive the Protocol singleton PDA: seeds `[b"protocol"]`.
    pub fn protocol_pda(program_id: &Pubkey) -> (Pubkey, u8) {
        kassandra_sdk::pda::protocol(program_id)
    }

    /// Derive the KASS mint-authority PDA: seeds `[b"mint_authority"]`.
    pub fn mint_authority_pda(program_id: &Pubkey) -> (Pubkey, u8) {
        kassandra_sdk::pda::mint_authority(program_id)
    }

    /// Derive the stake-vault PDA for an oracle: seeds `[b"vault", oracle]`.
    pub fn stake_vault_pda(program_id: &Pubkey, oracle: &Pubkey) -> (Pubkey, u8) {
        kassandra_sdk::pda::stake_vault(program_id, oracle)
    }

    /// Derive the challenger USDC escrow vault PDA: seeds `[b"challenge_usdc", market]`.
    pub fn challenge_usdc_vault_pda(program_id: &Pubkey, market: &Pubkey) -> (Pubkey, u8) {
        kassandra_sdk::pda::challenge_usdc_vault(program_id, market)
    }

    /// Derive the Proposer PDA: seeds `[b"proposer", oracle, authority]`.
    pub fn proposer_pda(program_id: &Pubkey, oracle: &Pubkey, authority: &Pubkey) -> (Pubkey, u8) {
        kassandra_sdk::pda::proposer(program_id, oracle, authority)
    }

    /// Derive the Fact PDA: seeds `[b"fact", oracle, content_hash]`.
    pub fn fact_pda(program_id: &Pubkey, oracle: &Pubkey, content_hash: &[u8; 32]) -> (Pubkey, u8) {
        kassandra_sdk::pda::fact(program_id, oracle, content_hash)
    }

    /// Derive the FactVote PDA: seeds `[b"vote", fact, voter]`.
    pub fn vote_pda(program_id: &Pubkey, fact: &Pubkey, voter: &Pubkey) -> (Pubkey, u8) {
        kassandra_sdk::pda::vote(program_id, fact, voter)
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
        kassandra_sdk::ix::init_protocol(
            &self.program_id,
            protocol,
            self.payer.pubkey(),
            self.kass_mint,
            self.usdc_mint,
        )
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
    /// tests can pass a wrong signer. Account order (Task G1):
    /// `[0] protocol(w) [1] authority(signer) [2] kass_dao(ro)`. Payload =
    /// `dao_authority ++ kass_dao`. The `kass_dao` ACCOUNT is the same pubkey as
    /// the payload `kass_dao` (the hardened processor asserts they match).
    pub fn set_governance_ix(
        &self,
        protocol: Pubkey,
        authority: Pubkey,
        dao_authority: Pubkey,
        kass_dao: Pubkey,
    ) -> Instruction {
        kassandra_sdk::ix::set_governance(
            &self.program_id,
            protocol,
            authority,
            dao_authority,
            kass_dao,
        )
    }

    /// Derive the Squads v4 multisig **vault** PDA (the DAO execution authority)
    /// for a futarchy `Dao` pubkey, via the documented seed builders in
    /// [`md6`] and the real Squads v4 program id: the multisig's `create_key`
    /// IS the `Dao` (`[b"multisig", b"multisig", dao]`), then the vault at index
    /// 0 (`[b"multisig", multisig, b"vault", [0]]`). This is the value the
    /// hardened `set_governance` (Task G1) requires as `dao_authority`.
    pub fn squads_vault_for_dao(dao: &Pubkey) -> Pubkey {
        let squads_id = Pubkey::new_from_array(md6::SQUADS_V4_ID.to_bytes());
        let dao_arr = dao.to_bytes();
        let (multisig, _) =
            Pubkey::find_program_address(&md6::squads_multisig_seeds(&dao_arr.into()), &squads_id);
        let multisig_arr = multisig.to_bytes();
        let (vault, _) = Pubkey::find_program_address(
            &md6::squads_vault_seeds(&multisig_arr.into(), &[0u8]),
            &squads_id,
        );
        vault
    }

    /// Fabricate a real futarchy-owned `Dao` account (valid Anchor
    /// discriminator) at a fresh key and return `(kass_dao, derived vault PDA)`.
    /// The returned vault is exactly what the hardened `set_governance` requires
    /// as `dao_authority`, so `ctx.set_governance(&admin, vault, kass_dao)`
    /// records the REAL linkage and succeeds. The embedded TWAP fields are valid
    /// but arbitrary (these accept-path tests don't read the price).
    pub fn fabricate_dao_and_vault(&mut self) -> (Pubkey, Pubkey) {
        let kass_dao = Pubkey::new_unique();
        let owner = Pubkey::new_from_array(md6::FUTARCHY_ID.to_bytes());
        self.fabricate_owned_account(kass_dao, owner, build_dao_blob(1, 1_000_000, 0, 0));
        let vault = Self::squads_vault_for_dao(&kass_dao);
        (kass_dao, vault)
    }

    /// Directly write the DAO linkage into the `Protocol` singleton, BYPASSING
    /// the (Task G1-hardened) `set_governance` instruction. The gating tests for
    /// `set_config`/`resolve_deadend`/emissions need an ARBITRARY, SIGNABLE
    /// keypair recorded as `dao_authority` to exercise the accept path — which is
    /// impossible through the real handoff, since that now requires
    /// `dao_authority == squads_vault_for_dao(kass_dao)` (a PDA no keypair can
    /// sign). This mirrors the harness's existing direct account-seeding
    /// philosophy (see [`TestCtx::seed_disputed_oracle`]). Marks
    /// `governance_set = 1`. Requires the protocol to already exist.
    pub fn force_governance(&mut self, dao_authority: Pubkey, kass_dao: Pubkey) -> Pubkey {
        let (protocol_pda, _) = Self::protocol_pda(&self.program_id);
        let mut p = self.protocol(protocol_pda);
        p.dao_authority = dao_authority.to_bytes().into();
        p.kass_dao = kass_dao.to_bytes().into();
        p.governance_set = 1;
        self.set_program_account(protocol_pda, bytemuck::bytes_of(&p).to_vec());
        protocol_pda
    }

    /// Send a real `SetConfig` instruction signed by `authority`, overwriting
    /// the `Protocol`-resident governable params with `params`. Returns the
    /// Protocol PDA + result so tests can assert success / the
    /// `Unauthorized` / `InvalidConfig` rejection paths. `set_governance` must
    /// have recorded `authority` as the `dao_authority` first.
    #[allow(clippy::result_large_err)]
    pub fn set_config(
        &mut self,
        authority: &Keypair,
        params: ConfigParams,
    ) -> (Pubkey, TransactionResult) {
        let (protocol_pda, _) = Self::protocol_pda(&self.program_id);
        let ix = self.set_config_ix(protocol_pda, authority.pubkey(), params);
        let res = if authority.pubkey() == self.payer.pubkey() {
            self.send(ix, &[])
        } else {
            self.send(ix, &[authority])
        };
        (protocol_pda, res)
    }

    /// Build a `SetConfig` instruction. Exposes `protocol`/`authority` so tests
    /// can pass a wrong signer. Payload = the 144-byte packed `ConfigParams`.
    pub fn set_config_ix(
        &self,
        protocol: Pubkey,
        authority: Pubkey,
        params: ConfigParams,
    ) -> Instruction {
        kassandra_sdk::ix::set_config(&self.program_id, protocol, authority, &params)
    }

    /// Send a real `ResolveDeadend` instruction signed by `authority`, setting
    /// `option` as the final outcome of a dead-ended `oracle`. Returns the
    /// Protocol PDA + result so tests can assert success / the `Unauthorized` /
    /// `WrongPhase` / `InvalidOptionsCount` rejection paths. `set_governance`
    /// must have recorded `authority` as the `dao_authority` first.
    #[allow(clippy::result_large_err)]
    pub fn resolve_deadend(
        &mut self,
        oracle: Pubkey,
        authority: &Keypair,
        option: u8,
    ) -> (Pubkey, TransactionResult) {
        let (protocol_pda, _) = Self::protocol_pda(&self.program_id);
        let ix = self.resolve_deadend_ix(protocol_pda, oracle, authority.pubkey(), option);
        let res = if authority.pubkey() == self.payer.pubkey() {
            self.send(ix, &[])
        } else {
            self.send(ix, &[authority])
        };
        (protocol_pda, res)
    }

    /// Build a `ResolveDeadend` instruction. Exposes `protocol`/`oracle`/
    /// `authority` so tests can pass a wrong signer or a substituted protocol.
    /// Account order: `[0] protocol(ro)`, `[1] oracle(w)`, `[2] authority(signer)`.
    /// Payload = the single `option` byte.
    pub fn resolve_deadend_ix(
        &self,
        protocol: Pubkey,
        oracle: Pubkey,
        authority: Pubkey,
        option: u8,
    ) -> Instruction {
        kassandra_sdk::ix::resolve_deadend(&self.program_id, protocol, oracle, authority, option)
    }

    /// Build a `KassPrice` instruction (Task F5): reads the futarchy `Dao`
    /// account's spot TWAP. Account order: `[0] protocol(ro)`, `[1] kass_dao(ro)`.
    /// Exposes both accounts so tests can pass a substituted protocol or a
    /// wrong/foreign-owned `kass_dao`. No payload.
    pub fn kass_price_ix(&self, protocol: Pubkey, kass_dao: Pubkey) -> Instruction {
        kassandra_sdk::ix::kass_price(&self.program_id, protocol, kass_dao)
    }

    /// Fabricate an account at `key` owned by `owner` holding `data`. Used by F5
    /// to stand up a futarchy-owned `Dao` account carrying a hand-built spot
    /// `TwapOracle` (and the wrong-owner negative case).
    pub fn fabricate_owned_account(&mut self, key: Pubkey, owner: Pubkey, data: Vec<u8>) {
        let lamports = self.svm.minimum_balance_for_rent_exemption(data.len());
        self.svm
            .set_account(
                key,
                Account {
                    lamports,
                    data,
                    owner,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
    }

    /// Ensure the Protocol singleton exists and hand governance off with a
    /// hand-built futarchy `Dao` account whose embedded spot TWAP equals
    /// [`KASS_PRICE_TWAP`], recorded as `Protocol.kass_dao`. This makes
    /// `open_challenge`'s `kass_price` read return a deterministic value so the
    /// escrow size ([`required_escrow_usdc`]) is computable. Returns the
    /// `kass_dao` account key. One-shot per `TestCtx` (set_governance is
    /// one-shot).
    pub fn bless_kass_price(&mut self) -> Pubkey {
        self.ensure_protocol();
        let kass_dao = Pubkey::new_unique();
        let owner = Pubkey::new_from_array(md6::FUTARCHY_ID.to_bytes());
        // twap = aggregator / (last_updated - (created_at + start_delay)).
        // Pick a 1_000_000s window so aggregator = twap * 1e6 yields KASS_PRICE_TWAP.
        let last_updated: i64 = 1_000_000;
        let created_at: i64 = 0;
        let start_delay: u32 = 0;
        let aggregator: u128 = KASS_PRICE_TWAP * 1_000_000;
        self.fabricate_owned_account(
            kass_dao,
            owner,
            build_dao_blob(aggregator, last_updated, created_at, start_delay),
        );
        // The kass_price tests only read `kass_dao`; the recorded `dao_authority`
        // is irrelevant to them. Record the linkage DIRECTLY (force_governance)
        // rather than through the Task G1-hardened handoff, which would require a
        // matching derived Squads vault here for no test benefit.
        let (dao_authority, _) = Self::stand_in_governance(0x77);
        self.force_governance(dao_authority, kass_dao);
        kass_dao
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
        kassandra_sdk::ix::create_oracle(
            &self.program_id,
            nonce,
            options_count,
            deadline,
            twap_window,
            &prompt_hash,
            oracle,
            kass_mint,
            usdc_mint,
            self.payer.pubkey(),
            self.payer_kass,
        )
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
        kassandra_sdk::ix::propose(
            &self.program_id,
            oracle,
            proposer,
            authority,
            authority_kass,
            vault,
            option,
            bond,
        )
    }

    /// Build a `FinalizeProposals` instruction: `[0] oracle(w)` followed by the
    /// given proposer accounts as a READ-ONLY tail. Exposes the full proposer
    /// slice so tests can pass a subset, a duplicate, or a foreign-oracle account.
    pub fn finalize_proposals_ix(&self, oracle: Pubkey, proposers: &[Pubkey]) -> Instruction {
        kassandra_sdk::ix::finalize_proposals(&self.program_id, oracle, proposers)
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
        // exactly the summed bonds, BACKED by real mint supply. The backing is
        // required so a terminal InvalidDeadend burn (finalize_oracle /
        // finalize_no_facts burning the slashed `bond_pool` back to the reservoir)
        // has real supply to check-subtract — a real `Burn` underflows otherwise.
        // It mirrors reality (proposer bonds are circulating KASS) and is captured
        // by every supply-DELTA assertion (tests snapshot supply AFTER seeding).
        let stake_vault = self.create_token_account(self.kass_mint, oracle_pda, total_stake);
        self.add_mint_supply(self.kass_mint, total_stake);

        // Build and write the Oracle account.
        let now = self.now();
        let mut oracle = Oracle::zeroed();
        oracle.account_type = AccountType::Oracle.as_u8();
        oracle.creator = self.payer.pubkey().to_bytes().into();
        oracle.kass_mint = self.kass_mint.to_bytes().into();
        oracle.usdc_mint = self.usdc_mint.to_bytes().into();
        oracle.stake_vault = stake_vault.to_bytes().into();
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
        // Settlement-era (S1) snapshot fields. NOTE: deliberately kept at the
        // conservative pre-S1 defaults (no approve-voter slash, zero reward
        // weights) rather than the now-real `init_protocol`/`create_oracle`
        // defaults (1/2 slash, 2/1 weights). This keeps fabricated-oracle
        // `finalize_facts` behavior a pure counter (rejected facts add only the
        // submitter stake to `bond_pool`), so the existing `finalize_facts` /
        // conservation (`invariants.rs`) fixtures stay self-consistent. Tests
        // that exercise the approve-voter slash opt in via
        // [`TestCtx::set_fact_vote_slash`]. `fact_vote_slash_den` stays positive
        // so a settlement-era reader never divides by zero. The 3 S1 resolution
        // totals (`total_correct_proposer_stake` / `total_approved_fact_stake` /
        // `reward_pool`) stay 0 (their `zeroed()` default), correct pre-resolution.
        oracle.fact_vote_slash_num = 0;
        oracle.fact_vote_slash_den = 1;
        oracle.reward_proposer_weight = 0;
        oracle.reward_fact_weight = 0;
        // C1 challenge-fee config snapshot (matches init_protocol/create_oracle
        // defaults), so a fabricated oracle sizes/settles like a real one.
        oracle.challenge_fail_usdc_fee_num = CHALLENGE_FAIL_USDC_FEE_NUM;
        oracle.challenge_fail_usdc_fee_den = CHALLENGE_FAIL_USDC_FEE_DEN;
        oracle.challenge_success_kass_fee_num = CHALLENGE_SUCCESS_KASS_FEE_NUM;
        oracle.challenge_success_kass_fee_den = CHALLENGE_SUCCESS_KASS_FEE_DEN;
        self.set_program_account(oracle_pda, bytemuck::bytes_of(&oracle).to_vec());

        // Build and write each Proposer account.
        let mut proposers = Vec::with_capacity(specs.len());
        for spec in specs {
            let authority = Keypair::new();
            let (pda, p_bump) =
                Self::proposer_pda(&self.program_id, &oracle_pda, &authority.pubkey());

            let mut proposer = Proposer::zeroed();
            proposer.account_type = AccountType::Proposer.as_u8();
            proposer.oracle = oracle_pda.to_bytes().into();
            proposer.authority = authority.pubkey().to_bytes().into();
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

    /// Airdrop SOL lamports to `account` (so it exists as a funded system
    /// account, e.g. a rent recipient).
    pub fn airdrop(&mut self, account: &Keypair, lamports: u64) {
        self.svm.airdrop(&account.pubkey(), lamports).unwrap();
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

    /// Stamp a prior (flip) slash on a seeded proposer: `slashed_amount = amount`,
    /// `slashed = flipped = 1`, still surviving + NOT disqualified, and add
    /// `amount` to the oracle's `bond_pool` (keeping the per-proposer identity
    /// `slashed_amount == bond_pool contribution`). Lets `settle_challenge` tests
    /// stand up the finalize_ai_claims flip-slash → challenged → disqualified
    /// cross-path without driving that earlier phase.
    pub fn set_proposer_prior_slash(&mut self, oracle: Pubkey, proposer: Pubkey, amount: u64) {
        let mut p = self.proposer(proposer);
        p.slashed = 1;
        p.flipped = 1;
        p.slashed_amount = amount;
        self.set_program_account(proposer, bytemuck::bytes_of(&p).to_vec());
        let mut o = self.oracle(oracle);
        o.bond_pool += amount;
        self.set_program_account(oracle, bytemuck::bytes_of(&o).to_vec());
    }

    /// Overwrite a seeded oracle's `fact_vote_slash` snapshot (the rejected-fact
    /// approve-voter slash fraction `num/den`, the same field `create_oracle`
    /// snapshots from `Protocol` and `set_config` retunes). Lets `finalize_facts`
    /// tests drive the approve-voter aggregate slash without the full real flow.
    pub fn set_fact_vote_slash(&mut self, oracle: Pubkey, num: u64, den: u64) {
        let mut o = self.oracle(oracle);
        o.fact_vote_slash_num = num;
        o.fact_vote_slash_den = den;
        self.set_program_account(oracle, bytemuck::bytes_of(&o).to_vec());
    }

    /// Overwrite a seeded oracle's directional challenge-fee snapshot (the same
    /// fields `create_oracle` snapshots from `Protocol` and `set_config` retunes).
    /// Lets `settle_challenge` tests prove the fee is read from the per-oracle
    /// snapshot — i.e. a governance fee change flows into a new challenge's settle
    /// — without driving the full real create/propose/finalize/challenge flow.
    pub fn set_challenge_fees(
        &mut self,
        oracle: Pubkey,
        fail_usdc_num: u64,
        fail_usdc_den: u64,
        success_kass_num: u64,
        success_kass_den: u64,
    ) {
        let mut o = self.oracle(oracle);
        o.challenge_fail_usdc_fee_num = fail_usdc_num;
        o.challenge_fail_usdc_fee_den = fail_usdc_den;
        o.challenge_success_kass_fee_num = success_kass_num;
        o.challenge_success_kass_fee_den = success_kass_den;
        self.set_program_account(oracle, bytemuck::bytes_of(&o).to_vec());
    }

    /// Stamp a seeded oracle's `reward_emission` (the KASS minted at creation,
    /// Task S3) AND physically place that KASS in its `stake_vault`, backed by
    /// mint supply — so a `finalize_oracle` InvalidDeadend burn-back has real
    /// tokens + supply to subtract (no underflow). Lets `finalize_oracle` tests
    /// drive the emission fold-in / burn-back without the full create flow.
    pub fn set_reward_emission(&mut self, oracle: Pubkey, amount: u64) {
        let mut o = self.oracle(oracle);
        o.reward_emission = amount;
        let vault = Pubkey::new_from_array(o.stake_vault.to_bytes());
        self.set_program_account(oracle, bytemuck::bytes_of(&o).to_vec());
        self.add_token_balance(vault, amount);
        self.add_mint_supply(self.kass_mint, amount);
    }

    /// Overwrite the KASS mint's SPL `mint_authority` (a `COption<Pubkey>`).
    /// Lets the mint-authority-mismatch test point the canonical mint at a
    /// non-PDA authority so `create_oracle`'s emission mint is rejected with
    /// [`kassandra_program::error::KassandraError::BadMintAuthority`].
    pub fn set_kass_mint_authority(&mut self, authority: Pubkey) {
        let mint = self.kass_mint;
        let acc = self.svm.get_account(&mint).expect("kass mint not found");
        let mut state = Mint::unpack(&acc.data).expect("not a mint");
        state.mint_authority = COption::Some(authority);
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

    /// Build a `FinalizeOracle` instruction (Ix 6). Account order:
    /// `[0] oracle(w) [1] kass_mint(w) [2] stake_vault(w) [3] token program`
    /// followed by the read-only proposer tail. Payload = `oracle_nonce` LE
    /// (signs the InvalidDeadend emission burn-back). The oracle must be in the
    /// bookkeeping map (seeded or real-flow) so its nonce/vault are known.
    pub fn finalize_oracle_ix(&self, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
        let seeded = self.seeded(oracle);
        kassandra_sdk::ix::finalize_oracle(
            &self.program_id,
            oracle,
            self.kass_mint,
            seeded.stake_vault,
            seeded.nonce,
            tail,
        )
    }

    /// Build a `FinalizeFacts` instruction (Ix 2). Account order (mirrors
    /// `finalize_oracle`'s burn prefix): `[0] oracle(w) [1] kass_mint(w)
    /// [2] stake_vault(w) [3] token program` followed by a WRITABLE tail (the
    /// fact / proposer subset being settled). Payload = `oracle_nonce` LE (signs
    /// the no-facts dead-end `bond_pool` + emission burn). The oracle must be in
    /// the bookkeeping map (seeded or real-flow) so its nonce/vault are known.
    pub fn finalize_facts_ix(&self, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
        let seeded = self.seeded(oracle);
        kassandra_sdk::ix::finalize_facts(
            &self.program_id,
            oracle,
            self.kass_mint,
            seeded.stake_vault,
            seeded.nonce,
            tail,
        )
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
    /// it with `amount` base units of KASS, BACKED by real mint supply. Returns
    /// the token account address. Used to bankroll a fact submitter / voter /
    /// proposer bond source. The supply backing keeps the KASS that flows into a
    /// stake vault physically real, so a terminal InvalidDeadend burn of the
    /// slashed `bond_pool` (which may include rejected-fact stakes + approve-voter
    /// slashes) does not underflow the mint supply. Every emission-calc test
    /// snapshots supply right before its measured `create_oracle`, so this earlier
    /// funding is captured consistently and never skews the emission.
    pub fn fund_kass(&mut self, owner: &Keypair, amount: u64) -> Pubkey {
        let acct = self.create_token_account(self.kass_mint, owner.pubkey(), amount);
        self.add_mint_supply(self.kass_mint, amount);
        acct
    }

    /// Create an SPL token account on the USDC mint owned by `owner` and fund it
    /// with `amount` base units. Returns the token account address. Mirrors
    /// [`TestCtx::fund_kass`]; the challenge-escrow source for `open_challenge`.
    pub fn fund_usdc(&mut self, owner: &Keypair, amount: u64) -> Pubkey {
        self.create_token_account(self.usdc_mint, owner.pubkey(), amount)
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

    /// The `nonce` an oracle was created with (its PDA seed).
    pub fn oracle_nonce(&self, oracle: Pubkey) -> u64 {
        self.seeded(oracle).nonce
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
    /// [`TransactionError`](solana_transaction_error::TransactionError).
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
        // Label the CU record by the instruction discriminant (first payload byte).
        let name = ix_name(ix.data.first().copied().unwrap_or(u8::MAX));
        let mut all_signers: Vec<&Keypair> = Vec::with_capacity(signers.len() + 1);
        all_signers.push(&self.payer);
        all_signers.extend_from_slice(signers);
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&self.payer.pubkey()),
            &all_signers,
            blockhash,
        );
        let res = self.svm.send_transaction(tx);
        if let Ok(meta) = &res {
            self.cu_meter.record(name, meta.compute_units_consumed);
        }
        res
    }

    /// Send `ix` expecting success and return the compute units it consumed (also
    /// recorded in the CU meter). Convenience over `send(..).expect(..).compute_
    /// units_consumed` for metering call sites.
    pub fn send_cu(&mut self, ix: Instruction, signers: &[&Keypair]) -> u64 {
        self.send(ix, signers)
            .expect("send_cu: transaction should succeed")
            .compute_units_consumed
    }

    /// The maximum CU observed for an instruction (by name; see [`ix_name`]).
    pub fn cu_max(&self, name: &str) -> Option<u64> {
        self.cu_meter.max(name)
    }

    /// A human-readable per-instruction CU report over everything sent so far.
    pub fn cu_report(&self) -> String {
        self.cu_meter.report()
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

// ===========================================================================
// Terminal-oracle seeding for the S2 claim-and-close tests.
// ===========================================================================

/// One proposer to seed into a TERMINAL oracle (Task S2 claims).
#[derive(Clone, Copy, Debug)]
pub struct ClaimProposerSpec {
    pub bond: u64,
    /// Post-AI-claim vote; compared to `resolved_option` for the correct/wrong split.
    pub claim_option: u8,
    pub disqualified: bool,
    /// KASS already forfeited to `bond_pool` (only meaningful when disqualified).
    pub slashed_amount: u64,
}

/// One fact vote to seed under a fact.
#[derive(Clone, Copy, Debug)]
pub struct ClaimVoteSpec {
    pub stake: u64,
    pub kind: u8, // VOTE_APPROVE / VOTE_DUPLICATE
}

/// One fact (submitter + its votes) to seed into a TERMINAL oracle.
#[derive(Clone, Debug)]
pub struct ClaimFactSpec {
    pub stake: u64, // submitter stake
    pub agreed: bool,
    pub duplicate: bool,
    pub votes: Vec<ClaimVoteSpec>,
}

/// A seeded claimant account: the program-owned account to be closed, its
/// signing authority, the authority-owned KASS destination (starts at 0), and
/// the expected KASS entitlement per the matrix.
pub struct SeededClaim {
    pub account: Pubkey,
    pub authority: Keypair,
    pub dest_kass: Pubkey,
    pub expected: u64,
    pub kind: u8, // for votes; ignored otherwise
}

/// A seeded fact: its submitter claim plus the per-vote claims.
pub struct SeededFactClaim {
    pub submitter: SeededClaim,
    pub votes: Vec<SeededClaim>,
}

/// A fully-seeded terminal oracle and everything the claim tests need.
pub struct TerminalSeed {
    pub oracle: Pubkey,
    pub nonce: u64,
    pub bump: u8,
    pub stake_vault: Pubkey,
    pub vault_initial: u64,
    pub reward_pool: u64,
    pub total_correct_proposer_stake: u64,
    pub total_approved_fact_stake: u64,
    pub resolved_option: u8,
    pub proposers: Vec<SeededClaim>,
    pub facts: Vec<SeededFactClaim>,
}

/// Reward cohort weights the terminal seeder stamps (mirrors the real defaults).
const SEED_PW: u64 = kassandra_program::config::REWARD_PROPOSER_WEIGHT;
const SEED_FW: u64 = kassandra_program::config::REWARD_FACT_WEIGHT;

impl TestCtx {
    /// Fabricate an oracle in a TERMINAL phase (`Resolved` or `InvalidDeadend`)
    /// with the given proposers/facts/votes, a stake vault funded to the
    /// post-settlement balance, and self-consistent resolution stamps
    /// (`reward_pool`, `total_correct_proposer_stake`, `total_approved_fact_stake`).
    ///
    /// The "slashed pool" = Σ `slashed_amount` (disqualified OR flip-slashed) + Σ
    /// rejected fact submitter stake + Σ rejected-fact approve-voter slash (floor
    /// `num/den`). On **Resolved** it is the distributable `reward_pool` and the
    /// vault holds the full `gross` (Σ bonds + stakes), so a complete claim sweep
    /// drains it to floor-division dust. On **InvalidDeadend** it is instead the
    /// amount the finalize site BURNED out of the vault (a dead-end distributes
    /// nothing): the vault is funded with `gross − slashed_pool`, `reward_pool ==
    /// 0`, the slashed pool is recorded on `bond_pool`, and the claims (rejected
    /// submitters/voters forfeit, survivors get `bond − slashed_amount`) drain it
    /// to dust. Each claimant's `expected` entitlement is precomputed via the
    /// program's own [`reward`] helpers.
    ///
    /// `slash_num/slash_den` is the approve-voter slash fraction stamped on the
    /// oracle (`1/2` matches the real default). To keep the aggregate
    /// `bond_pool` counter equal to the per-voter physical slash (no dust gap),
    /// callers should give rejected-fact approve votes EVEN stakes when
    /// `slash_den == 2`.
    pub fn seed_terminal_oracle(
        &mut self,
        phase: Phase,
        resolved_option: u8,
        proposers: &[ClaimProposerSpec],
        facts: &[ClaimFactSpec],
        slash_num: u64,
        slash_den: u64,
    ) -> TerminalSeed {
        let resolved = phase == Phase::Resolved;

        let nonce = self.next_nonce;
        self.next_nonce += 1;
        let (oracle_pda, bump) = Self::oracle_pda(&self.program_id, nonce);

        // ----- totals + reward_pool (the resolution stamps) -----------------
        let mut total_correct: u64 = 0;
        for p in proposers {
            if resolved && !p.disqualified && p.claim_option == resolved_option {
                total_correct += p.bond;
            }
        }
        let mut total_approved: u64 = 0;
        for f in facts {
            if resolved && f.agreed {
                let approve: u64 = f
                    .votes
                    .iter()
                    .filter(|v| v.kind == VOTE_APPROVE)
                    .map(|v| v.stake)
                    .sum();
                total_approved += f.stake + approve;
            }
        }
        // The slashed pool = Σ proposer slashes + Σ rejected-fact (submitter stake
        // + floor approve-voter slash). On Resolved it is the distributable
        // `reward_pool`; on InvalidDeadend it is the amount the finalize site
        // BURNED back, so the vault is funded with `gross − slashed_pool` and the
        // counter is stamped on `bond_pool` (mirroring the on-chain post-burn
        // terminal state). This is the same formula on both phases.
        let mut slashed_pool: u64 = 0;
        for p in proposers {
            // ANY slashed_amount (disqualified OR flip-slashed-but-surviving).
            slashed_pool += p.slashed_amount;
        }
        for f in facts {
            if !f.agreed && !f.duplicate {
                // Rejected: submitter full forfeit + approve-voter floor slash.
                let approve: u64 = f
                    .votes
                    .iter()
                    .filter(|v| v.kind == VOTE_APPROVE)
                    .map(|v| v.stake)
                    .sum();
                slashed_pool += f.stake;
                slashed_pool +=
                    ((approve as u128) * (slash_num as u128) / (slash_den as u128)) as u64;
            }
        }
        let reward_pool: u64 = if resolved { slashed_pool } else { 0 };
        // On a dead-end the slashed pool was burned out of the vault at finalize.
        let burn_pool: u64 = if resolved { 0 } else { slashed_pool };

        let (proposer_bucket, fact_bucket) =
            reward::reward_buckets(reward_pool, SEED_PW, SEED_FW, total_correct, total_approved);

        // ----- vault balance: Σ all bonds + stakes, MINUS the dead-end burn -----
        let gross: u64 = proposers.iter().map(|p| p.bond).sum::<u64>()
            + facts
                .iter()
                .map(|f| f.stake + f.votes.iter().map(|v| v.stake).sum::<u64>())
                .sum::<u64>();
        let vault_initial: u64 = gross - burn_pool;
        let stake_vault = self.create_token_account(self.kass_mint, oracle_pda, vault_initial);
        // Back the vault KASS with real mint supply (a Burn elsewhere checks it).
        self.add_mint_supply(self.kass_mint, vault_initial);

        // ----- the Oracle account -------------------------------------------
        let now = self.now();
        let mut oracle = Oracle::zeroed();
        oracle.account_type = AccountType::Oracle.as_u8();
        oracle.creator = self.payer.pubkey().to_bytes().into();
        oracle.kass_mint = self.kass_mint.to_bytes().into();
        oracle.usdc_mint = self.usdc_mint.to_bytes().into();
        oracle.stake_vault = stake_vault.to_bytes().into();
        oracle.deadline = now;
        oracle.phase_ends_at = now;
        oracle.twap_window = TWAP_WINDOW;
        oracle.options_count = (resolved_option as u16 + 1).max(2) as u8;
        oracle.set_phase(phase);
        oracle.proposer_count = proposers.len() as u16;
        oracle.surviving_count = proposers.iter().filter(|p| !p.disqualified).count() as u16;
        // `total_oracle_stake` is the gross accumulator (never decremented by the
        // burn); the vault physically holds `gross − burn_pool`.
        oracle.total_oracle_stake = gross;
        oracle.dispute_bond_total = proposers.iter().map(|p| p.bond).sum();
        // On a dead-end the burned slashed pool is recorded on `bond_pool` (the
        // durable counter), matching the on-chain post-burn terminal state.
        oracle.bond_pool = burn_pool;
        oracle.bump = bump;
        oracle.resolved_option = if resolved {
            resolved_option
        } else {
            CLAIM_OPTION_NONE
        };
        // Reward config snapshot (the real defaults) + the chosen slash fraction.
        oracle.reward_proposer_weight = SEED_PW;
        oracle.reward_fact_weight = SEED_FW;
        oracle.fact_vote_slash_num = slash_num;
        oracle.fact_vote_slash_den = slash_den;
        // The resolution stamps the claims read.
        oracle.total_correct_proposer_stake = total_correct;
        oracle.total_approved_fact_stake = total_approved;
        oracle.reward_pool = reward_pool;
        self.set_program_account(oracle_pda, bytemuck::bytes_of(&oracle).to_vec());

        // ----- the Proposer accounts ----------------------------------------
        let mut seeded_proposers = Vec::with_capacity(proposers.len());
        for p in proposers {
            let authority = Keypair::new();
            self.svm
                .airdrop(&authority.pubkey(), 1_000_000_000)
                .unwrap();
            let dest_kass = self.create_token_account(self.kass_mint, authority.pubkey(), 0);

            let mut acct = Proposer::zeroed();
            acct.account_type = AccountType::Proposer.as_u8();
            acct.oracle = oracle_pda.to_bytes().into();
            acct.authority = authority.pubkey().to_bytes().into();
            acct.bond = p.bond;
            acct.original_option = p.claim_option;
            acct.claim_option = p.claim_option;
            acct.disqualified = p.disqualified as u8;
            acct.slashed = (p.slashed_amount > 0) as u8;
            acct.slashed_amount = p.slashed_amount;
            let account = self.seed_program_account(bytemuck::bytes_of(&acct).to_vec());

            // Mirrors on-chain `claim_proposer`: disqualified forfeits the whole
            // bond (base 0); survivor gets `bond − slashed_amount`; +reward iff
            // Resolved + surviving + correct.
            let base = if p.disqualified {
                0
            } else {
                p.bond.saturating_sub(p.slashed_amount)
            };
            let reward = if resolved && !p.disqualified && p.claim_option == resolved_option {
                reward::proposer_reward(p.bond, proposer_bucket, total_correct)
            } else {
                0
            };
            let expected = base + reward;

            seeded_proposers.push(SeededClaim {
                account,
                authority,
                dest_kass,
                expected,
                kind: 0,
            });
        }

        // ----- the Fact + FactVote accounts ---------------------------------
        let mut seeded_facts = Vec::with_capacity(facts.len());
        for f in facts {
            let approve_stake: u64 = f
                .votes
                .iter()
                .filter(|v| v.kind == VOTE_APPROVE)
                .map(|v| v.stake)
                .sum();
            let duplicate_stake: u64 = f
                .votes
                .iter()
                .filter(|v| v.kind == VOTE_DUPLICATE)
                .map(|v| v.stake)
                .sum();

            // Submitter.
            let submitter_auth = Keypair::new();
            self.svm
                .airdrop(&submitter_auth.pubkey(), 1_000_000_000)
                .unwrap();
            let submitter_dest =
                self.create_token_account(self.kass_mint, submitter_auth.pubkey(), 0);

            let mut fact = Fact::zeroed();
            fact.account_type = AccountType::Fact.as_u8();
            fact.oracle = oracle_pda.to_bytes().into();
            fact.proposer = submitter_auth.pubkey().to_bytes().into();
            fact.stake = f.stake;
            fact.approve_stake = approve_stake;
            fact.duplicate_stake = duplicate_stake;
            fact.agreed = f.agreed as u8;
            fact.duplicate = f.duplicate as u8;
            fact.settled = 1;
            let fact_account = self.seed_program_account(bytemuck::bytes_of(&fact).to_vec());

            // Disposition-based on BOTH terminal phases; reward only on Resolved.
            // A rejected submitter forfeits (0) on a dead-end too (its stake was
            // burned out of the vault at finalize).
            let submitter_expected = if f.agreed {
                f.stake
                    + if resolved {
                        reward::fact_reward(f.stake, fact_bucket, total_approved)
                    } else {
                        0
                    }
            } else if f.duplicate {
                f.stake
            } else {
                0
            };

            // Votes.
            let mut seeded_votes = Vec::with_capacity(f.votes.len());
            for v in &f.votes {
                let voter = Keypair::new();
                self.svm.airdrop(&voter.pubkey(), 1_000_000_000).unwrap();
                let voter_dest = self.create_token_account(self.kass_mint, voter.pubkey(), 0);

                let mut vote = FactVote::zeroed();
                vote.account_type = AccountType::FactVote.as_u8();
                vote.fact = fact_account.to_bytes().into();
                vote.voter = voter.pubkey().to_bytes().into();
                vote.stake = v.stake;
                vote.kind = v.kind;
                let vote_account = self.seed_program_account(bytemuck::bytes_of(&vote).to_vec());

                // Disposition-based on BOTH terminal phases; reward only on
                // Resolved. The rejected-fact approve-voter is slashed on a
                // dead-end too (its slashed fraction was burned at finalize).
                let approve = v.kind == VOTE_APPROVE;
                let expected = if approve && f.agreed {
                    // Approve-voter on an agreed fact earns the fact rate (Resolved
                    // only; 0 on InvalidDeadend since reward_pool == 0).
                    v.stake
                        + if resolved {
                            reward::fact_reward(v.stake, fact_bucket, total_approved)
                        } else {
                            0
                        }
                } else if approve && !f.duplicate {
                    // Approve-voter on a rejected fact is slashed CEIL(stake·num/den)
                    // (mirrors on-chain: ceil keeps the vault from running short
                    // against the floor-aggregate bond_pool credit).
                    let ceil =
                        ((v.stake as u128) * (slash_num as u128)).div_ceil(slash_den as u128);
                    v.stake - ceil as u64
                } else {
                    // Duplicate-voter, or approve-on-duplicate-dominant: full stake.
                    v.stake
                };

                seeded_votes.push(SeededClaim {
                    account: vote_account,
                    authority: voter,
                    dest_kass: voter_dest,
                    expected,
                    kind: v.kind,
                });
            }

            seeded_facts.push(SeededFactClaim {
                submitter: SeededClaim {
                    account: fact_account,
                    authority: submitter_auth,
                    dest_kass: submitter_dest,
                    expected: submitter_expected,
                    kind: 0,
                },
                votes: seeded_votes,
            });
        }

        TerminalSeed {
            oracle: oracle_pda,
            nonce,
            bump,
            stake_vault,
            vault_initial,
            reward_pool,
            total_correct_proposer_stake: total_correct,
            total_approved_fact_stake: total_approved,
            resolved_option,
            proposers: seeded_proposers,
            facts: seeded_facts,
        }
    }

    /// Fold a creation-time `reward_emission` into an already-seeded TERMINAL
    /// `Resolved` oracle, mirroring the `create_oracle` mint + `finalize_oracle`
    /// fold the real flow would produce: physically add `amount` KASS to the
    /// stake vault (backed by mint supply), stamp `reward_emission`, AND add it to
    /// the distributable `reward_pool` (the S3 `reward_pool = bond_pool +
    /// reward_emission`). The S2 claims then read the emission-boosted pool, so a
    /// correct proposer / approved fact staker's reward reflects the emission.
    /// (Use ONLY on a `Resolved` seed; on `InvalidDeadend` the emission would have
    /// been burned back, so it must not sit in the vault.)
    pub fn fold_reward_emission(&mut self, oracle: Pubkey, amount: u64) {
        let mut o = self.oracle(oracle);
        o.reward_emission = amount;
        o.reward_pool += amount;
        let vault = Pubkey::new_from_array(o.stake_vault.to_bytes());
        self.set_program_account(oracle, bytemuck::bytes_of(&o).to_vec());
        self.add_token_balance(vault, amount);
        self.add_mint_supply(self.kass_mint, amount);
    }

    /// Seed a SETTLED-challenge disqualification on a proposer of an oracle seeded
    /// via [`TestCtx::seed_disputed_oracle`]: mark it disqualified + slashed with
    /// `slashed_amount = bond − kass_fee` (the bond_pool contribution), credit
    /// `bond_pool += slashed_amount`, decrement `surviving_count`, and remove the
    /// `kass_fee` KASS from the stake vault (modelling `settle_challenge`'s payout
    /// of `kass_fee` to the challenger). Mirrors the on-chain post-settle state so
    /// the deadend-after-settled-challenge conservation test starts from reality.
    pub fn seed_challenge_disqualify(&mut self, oracle: Pubkey, proposer: Pubkey, kass_fee: u64) {
        let mut p = self.proposer(proposer);
        let slashed_amount = p.bond - kass_fee;
        p.disqualified = 1;
        p.slashed = 1;
        p.slashed_amount = slashed_amount;
        self.set_program_account(proposer, bytemuck::bytes_of(&p).to_vec());

        let mut o = self.oracle(oracle);
        o.bond_pool += slashed_amount;
        o.surviving_count -= 1;
        let vault = Pubkey::new_from_array(o.stake_vault.to_bytes());
        self.set_program_account(oracle, bytemuck::bytes_of(&o).to_vec());
        // The kass_fee physically left the vault to the challenger at settle time.
        self.sub_token_balance(vault, kass_fee);
    }

    /// Build a `ClaimProposer` instruction (Ix 17). Account order:
    /// `[0] oracle(ro) [1] proposer(w) [2] dest_kass(w) [3] stake_vault(w)
    /// [4] rent_recipient(w) [5] token program`. Payload = `oracle_nonce` LE.
    pub fn claim_proposer_ix(
        &self,
        oracle: Pubkey,
        nonce: u64,
        proposer: Pubkey,
        dest_kass: Pubkey,
        stake_vault: Pubkey,
        rent_recipient: Pubkey,
    ) -> Instruction {
        kassandra_sdk::ix::claim_proposer(
            &self.program_id,
            oracle,
            nonce,
            proposer,
            dest_kass,
            stake_vault,
            rent_recipient,
        )
    }

    /// Build a `ClaimFact` instruction (Ix 18). Same account order as
    /// `claim_proposer_ix` with the `Fact` account at index 1.
    pub fn claim_fact_ix(
        &self,
        oracle: Pubkey,
        nonce: u64,
        fact: Pubkey,
        dest_kass: Pubkey,
        stake_vault: Pubkey,
        rent_recipient: Pubkey,
    ) -> Instruction {
        kassandra_sdk::ix::claim_fact(
            &self.program_id,
            oracle,
            nonce,
            fact,
            dest_kass,
            stake_vault,
            rent_recipient,
        )
    }

    /// Build a `ClaimFactVote` instruction (Ix 19). Account order:
    /// `[0] oracle(ro) [1] fact_vote(w) [2] fact(ro) [3] dest_kass(w)
    /// [4] stake_vault(w) [5] rent_recipient(w) [6] token program`.
    #[allow(clippy::too_many_arguments)]
    pub fn claim_fact_vote_ix(
        &self,
        oracle: Pubkey,
        nonce: u64,
        fact_vote: Pubkey,
        fact: Pubkey,
        dest_kass: Pubkey,
        stake_vault: Pubkey,
        rent_recipient: Pubkey,
    ) -> Instruction {
        kassandra_sdk::ix::claim_fact_vote(
            &self.program_id,
            oracle,
            nonce,
            fact_vote,
            fact,
            dest_kass,
            stake_vault,
            rent_recipient,
        )
    }

    /// Lamports balance of any account (0 if it does not exist), for asserting
    /// rent reclamation.
    pub fn lamports(&self, key: Pubkey) -> u64 {
        self.svm.get_account(&key).map(|a| a.lamports).unwrap_or(0)
    }

    /// Whether an account is closed (gone / zero-lamports / zero-length data).
    pub fn is_closed(&self, key: Pubkey) -> bool {
        match self.svm.get_account(&key) {
            None => true,
            Some(a) => a.lamports == 0 || a.data.is_empty(),
        }
    }

    // ----- S4: account-closure (close_ai_claim / close_market) helpers -------

    /// Fabricate an [`AiClaim`] bound to `oracle` + `proposer`, stamping
    /// `authority` (the rent recipient close_ai_claim reads). Returns its
    /// (random) address.
    pub fn seed_ai_claim(&mut self, oracle: Pubkey, proposer: Pubkey, authority: Pubkey) -> Pubkey {
        let mut c = AiClaim::zeroed();
        c.account_type = AccountType::AiClaim.as_u8();
        c.oracle = oracle.to_bytes().into();
        c.proposer = proposer.to_bytes().into();
        c.authority = authority.to_bytes().into();
        self.seed_program_account(bytemuck::bytes_of(&c).to_vec())
    }

    /// Fabricate a [`Market`] bound to `oracle`, recording `challenger`,
    /// `challenger_usdc_vault`, and `settled`. Returns its (random) address.
    pub fn seed_market(
        &mut self,
        oracle: Pubkey,
        challenger: Pubkey,
        challenger_usdc_vault: Pubkey,
        settled: bool,
    ) -> Pubkey {
        let mut m = Market::zeroed();
        m.account_type = AccountType::Market.as_u8();
        m.oracle = oracle.to_bytes().into();
        m.challenger = challenger.to_bytes().into();
        m.challenger_usdc_vault = challenger_usdc_vault.to_bytes().into();
        m.settled = settled as u8;
        self.seed_program_account(bytemuck::bytes_of(&m).to_vec())
    }

    /// Fabricate a USDC escrow token account holding `amount`, with its token
    /// authority set to `owner` (the oracle PDA in the close_market tests).
    pub fn seed_usdc_escrow(&mut self, owner: Pubkey, amount: u64) -> Pubkey {
        self.create_token_account(self.usdc_mint, owner, amount)
    }

    /// Build a `CloseAiClaim` instruction (Ix 20). Account order:
    /// `[0] oracle(ro) [1] ai_claim(w) [2] rent_recipient(w)`. Empty payload.
    pub fn close_ai_claim_ix(
        &self,
        oracle: Pubkey,
        ai_claim: Pubkey,
        rent_recipient: Pubkey,
    ) -> Instruction {
        kassandra_sdk::ix::close_ai_claim(&self.program_id, oracle, ai_claim, rent_recipient)
    }

    /// Build a `CloseMarket` instruction (Ix 21). Account order:
    /// `[0] oracle(ro) [1] market(w) [2] challenger_usdc_vault(w)
    /// [3] rent_recipient(w) [4] token program`. Payload = `oracle_nonce` LE.
    pub fn close_market_ix(
        &self,
        oracle: Pubkey,
        nonce: u64,
        market: Pubkey,
        challenger_usdc_vault: Pubkey,
        rent_recipient: Pubkey,
    ) -> Instruction {
        kassandra_sdk::ix::close_market(
            &self.program_id,
            oracle,
            nonce,
            market,
            challenger_usdc_vault,
            rent_recipient,
        )
    }

    // ----- SW1: sweep_oracle (dust → treasury + terminal closure) helpers -----

    /// Overwrite a seeded oracle's `creator` (the rent recipient the sweep /
    /// closes refund to). Lets sweep tests point rent at a fresh keypair distinct
    /// from the fee payer, so an exact lamport-delta assertion is not confounded
    /// by transaction fees.
    pub fn set_creator(&mut self, oracle: Pubkey, creator: Pubkey) {
        let mut o = self.oracle(oracle);
        o.creator = creator.to_bytes().into();
        self.set_program_account(oracle, bytemuck::bytes_of(&o).to_vec());
    }

    /// Add `amount` KASS to a seeded oracle's `stake_vault` (backed by mint
    /// supply, mirroring the harness philosophy), modelling the residual dust /
    /// unclaimed principal a terminal vault retains after (or without) claims.
    pub fn fund_vault(&mut self, oracle: Pubkey, amount: u64) {
        let vault = Pubkey::new_from_array(self.oracle(oracle).stake_vault.to_bytes());
        self.add_token_balance(vault, amount);
        self.add_mint_supply(self.kass_mint, amount);
    }

    /// Derive the canonical KASS associated-token-account of `owner`
    /// (`ATA(owner, kass_mint)` under the ATA program) — the address the DAO
    /// treasury lives at.
    pub fn kass_ata(&self, owner: Pubkey) -> Pubkey {
        Pubkey::find_program_address(
            &[
                owner.as_ref(),
                TOKEN_PROGRAM_ID.as_ref(),
                self.kass_mint.as_ref(),
            ],
            &ATA_PROGRAM_ID,
        )
        .0
    }

    /// Fabricate the DAO treasury: an empty KASS token account AT the canonical
    /// `ATA(owner, kass_mint)` address, owned (token authority) by `owner`.
    /// Returns the ATA address.
    pub fn seed_kass_treasury(&mut self, owner: Pubkey) -> Pubkey {
        let ata = self.kass_ata(owner);
        let state = TokenAccount {
            mint: self.kass_mint,
            owner,
            amount: 0,
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
                ata,
                Account {
                    lamports,
                    data,
                    owner: TOKEN_PROGRAM_ID,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
        ata
    }

    /// Build a `SweepOracle` instruction (Ix 22). Account order:
    /// `[0] oracle(w) [1] stake_vault(w) [2] protocol(ro) [3] dao_treasury(w)
    /// [4] creator(w) [5] token program`. Payload = `oracle_nonce` LE. Exposes
    /// every account so tests can pass a wrong treasury / creator / vault.
    #[allow(clippy::too_many_arguments)]
    pub fn sweep_oracle_ix(
        &self,
        oracle: Pubkey,
        nonce: u64,
        stake_vault: Pubkey,
        protocol: Pubkey,
        dao_treasury: Pubkey,
        creator: Pubkey,
    ) -> Instruction {
        kassandra_sdk::ix::sweep_oracle(
            &self.program_id,
            oracle,
            nonce,
            stake_vault,
            protocol,
            dao_treasury,
            creator,
        )
    }
}

impl Default for TestCtx {
    fn default() -> Self {
        Self::new()
    }
}

/// Hand-build a futarchy `Dao` account blob with a `PoolState::Spot` embedded
/// spot `Pool` whose `TwapOracle` carries the given fields at the F0-documented
/// fixed offsets (mirrors `tests/kass_price.rs`). Used to give `open_challenge`
/// a deterministic `kass_price`.
pub fn build_dao_blob(
    aggregator: u128,
    last_updated: i64,
    created_at: i64,
    start_delay: u32,
) -> Vec<u8> {
    let mut data = vec![0u8; md6::DAO_SPOT_TWAP_MIN_LEN];
    data[0..8].copy_from_slice(&md6::DAO_ACCOUNT_DISCRIMINATOR);
    data[md6::DAO_POOLSTATE_TAG_OFFSET] = 0; // PoolState::Spot
    data[md6::DAO_SPOT_AGGREGATOR_OFFSET..md6::DAO_SPOT_AGGREGATOR_OFFSET + 16]
        .copy_from_slice(&aggregator.to_le_bytes());
    data[md6::DAO_SPOT_LAST_UPDATED_TS_OFFSET..md6::DAO_SPOT_LAST_UPDATED_TS_OFFSET + 8]
        .copy_from_slice(&last_updated.to_le_bytes());
    data[md6::DAO_SPOT_CREATED_AT_TS_OFFSET..md6::DAO_SPOT_CREATED_AT_TS_OFFSET + 8]
        .copy_from_slice(&created_at.to_le_bytes());
    data[md6::DAO_SPOT_START_DELAY_SECONDS_OFFSET..md6::DAO_SPOT_START_DELAY_SECONDS_OFFSET + 4]
        .copy_from_slice(&start_delay.to_le_bytes());
    data
}

// ---------------------------------------------------------------------------
// Shared dispute-core instruction builders.
//
// These are the raw-encoding builders for the dispute-core instructions
// (submit_fact / advance_phase / vote_fact / submit_ai_claim /
// finalize_ai_claims). They were previously copy-pasted, byte-for-byte, into
// ~8 separate integration-test files; hoisted here so every test shares one
// definition. Kept as an INDEPENDENT hand-encoding (not a wrapper over the Rust
// SDK) so the tests double as a cross-check that the on-chain layout matches.
// ---------------------------------------------------------------------------

/// Encode a `submit_fact` payload: `disc ++ content_hash[32] ++ stake_le[8] ++
/// uri_len_le[2] ++ uri`.
pub fn submit_fact_payload(content_hash: &[u8; 32], stake: u64, uri: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(1 + 32 + 8 + 2 + uri.len());
    data.push(Ix::SubmitFact as u8);
    data.extend_from_slice(content_hash);
    data.extend_from_slice(&stake.to_le_bytes());
    data.extend_from_slice(&(uri.len() as u16).to_le_bytes());
    data.extend_from_slice(uri);
    data
}

/// Build a `submit_fact` instruction with the locked-in account order.
pub fn submit_fact_ix(
    ctx: &TestCtx,
    oracle: Pubkey,
    fact: Pubkey,
    submitter: Pubkey,
    submitter_kass: Pubkey,
    vault: Pubkey,
    data: Vec<u8>,
) -> Instruction {
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(fact, false),
            AccountMeta::new(submitter, true),
            AccountMeta::new(submitter_kass, false),
            AccountMeta::new(vault, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data,
    }
}

/// Build an `advance_phase` instruction (single oracle account).
pub fn advance_phase_ix(ctx: &TestCtx, oracle: Pubkey) -> Instruction {
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![AccountMeta::new(oracle, false)],
        data: vec![Ix::AdvancePhase as u8],
    }
}

/// Encode a `vote_fact` payload: `disc ++ kind[1] ++ stake_le[8]`.
pub fn vote_payload(kind: u8, stake: u64) -> Vec<u8> {
    let mut data = Vec::with_capacity(1 + 1 + 8);
    data.push(Ix::VoteFact as u8);
    data.push(kind);
    data.extend_from_slice(&stake.to_le_bytes());
    data
}

/// Build a `vote_fact` instruction with the locked-in account order.
#[allow(clippy::too_many_arguments)]
pub fn vote_fact_ix(
    ctx: &TestCtx,
    oracle: Pubkey,
    fact: Pubkey,
    fact_vote: Pubkey,
    voter: Pubkey,
    voter_kass: Pubkey,
    vault: Pubkey,
    data: Vec<u8>,
) -> Instruction {
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(fact, false),
            AccountMeta::new(fact_vote, false),
            AccountMeta::new(voter, true),
            AccountMeta::new(voter_kass, false),
            AccountMeta::new(vault, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data,
    }
}

/// Encode a `submit_ai_claim` payload with fixed test hashes (model 0xAA,
/// params 0xBB, io 0xCC) + the chosen `option`.
pub fn submit_ai_payload(option: u8) -> Vec<u8> {
    let mut data = Vec::with_capacity(1 + 32 + 32 + 32 + 1);
    data.push(Ix::SubmitAiClaim as u8);
    data.extend_from_slice(&[0xAA; 32]); // model_id
    data.extend_from_slice(&[0xBB; 32]); // params_hash
    data.extend_from_slice(&[0xCC; 32]); // io_hash
    data.push(option);
    data
}

/// Build a `submit_ai_claim` instruction with the locked-in account order.
pub fn submit_ai_claim_ix(
    ctx: &TestCtx,
    oracle: Pubkey,
    proposer: Pubkey,
    claim: Pubkey,
    authority: Pubkey,
    data: Vec<u8>,
) -> Instruction {
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(proposer, false),
            AccountMeta::new(claim, false),
            AccountMeta::new(authority, true),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data,
    }
}

/// Build a `finalize_ai_claims` instruction (oracle + the proposer-set tail).
pub fn finalize_ai_claims_ix(ctx: &TestCtx, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
    let mut accounts = Vec::with_capacity(1 + tail.len());
    accounts.push(AccountMeta::new(oracle, false));
    for k in tail {
        accounts.push(AccountMeta::new(*k, false));
    }
    Instruction {
        program_id: ctx.program_id,
        accounts,
        data: vec![Ix::FinalizeAiClaims as u8],
    }
}

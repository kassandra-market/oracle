//! Fixed-size, zero-copy on-chain account layouts and the dispute phase enum.
//!
//! Every account struct is `#[repr(C)]`, `Pod` + `Zeroable`, and fully packed
//! (no implicit padding): fields are ordered and explicit `_pad` arrays are
//! inserted so each struct's `size_of` is a multiple of its 8-byte alignment.
//! This lets us read/write them straight out of account data with `bytemuck`.

use bytemuck::{Pod, Zeroable};
use pinocchio::cpi::Seed;

/// 32-byte Solana public key. Aliases pinocchio's `Address` — a
/// `#[repr(transparent)]` newtype over `[u8; 32]` that is `Pod`/`Zeroable` (via
/// solana-address's `bytemuck` feature), so the zero-copy account structs below
/// keep the exact same byte layout while gaining typed key comparisons.
pub type Pubkey = pinocchio::address::Address;

/// `Proposer.claim_option` sentinel: no AI claim submitted yet.
pub const CLAIM_OPTION_NONE: u8 = 0xFF;
/// `FactVote.kind`: approve vote.
pub const VOTE_APPROVE: u8 = 0;
/// `FactVote.kind`: duplicate vote.
pub const VOTE_DUPLICATE: u8 = 1;

/// On-chain account-type discriminator. Stored as the FIRST byte of every Pod
/// account (each struct's `account_type` field) so processors can reject
/// type-confusion: an attacker cannot pass a `Fact` where an `Oracle` is
/// expected because the tag won't match. `Uninitialized` (0) is what a freshly
/// `CreateAccount`'d, zeroed account carries before it is stamped.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AccountType {
    Uninitialized = 0,
    Oracle = 1,
    Proposer = 2,
    Fact = 3,
    FactVote = 4,
    AiClaim = 5,
    Market = 6,
    Protocol = 7,
    OracleMeta = 8,
}

impl AccountType {
    /// Encode this tag as its stored `u8` discriminant.
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Lifecycle phase of an oracle dispute. Stored on-chain as a `u8`
/// discriminant (see [`Oracle::phase`]) to keep account structs `Pod`.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// RESERVED / UNUSED: `create_oracle` (H1) initializes oracles directly into
    /// [`Phase::Proposal`], so no live oracle is ever in `Created`. Kept for ABI
    /// stability (the discriminant must not be renumbered); do not remove.
    Created = 0,
    Proposal = 1,
    FactProposal = 2,
    FactVoting = 3,
    AiClaim = 4,
    Challenge = 5,
    FinalRecompute = 6,
    Resolved = 7,
    InvalidDeadend = 8,
}

impl Phase {
    /// Safely convert a stored `u8` discriminant back into a `Phase`.
    pub fn from_u8(x: u8) -> Option<Self> {
        match x {
            0 => Some(Phase::Created),
            1 => Some(Phase::Proposal),
            2 => Some(Phase::FactProposal),
            3 => Some(Phase::FactVoting),
            4 => Some(Phase::AiClaim),
            5 => Some(Phase::Challenge),
            6 => Some(Phase::FinalRecompute),
            7 => Some(Phase::Resolved),
            8 => Some(Phase::InvalidDeadend),
            _ => None,
        }
    }

    /// Encode this phase as its stored `u8` discriminant.
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Top-level dispute account. `size_of == 392`.
///
/// # Governable params snapshot (Task F2)
/// The behavioral governable params (`threshold_*`, `market_threshold_*`,
/// `flip_slash_*`, `phase_window`, `proposal_window`, plus the settlement-era
/// reserved `fact_vote_slash_*` / reward weights) are SNAPSHOTTED from the
/// [`Protocol`] at `create_oracle` and read by the downstream processors from
/// the `Oracle` they already load. New oracles pick up the current `Protocol`
/// config; in-flight oracles keep their snapshot, so a mid-dispute governance
/// change can never move the goalposts. F2 defaults them (via `init_protocol`)
/// to the current `config.rs` consts, so behavior is unchanged.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Oracle {
    pub account_type: u8, // AccountType::Oracle
    pub _pad_hdr: [u8; 7],
    pub creator: Pubkey,
    pub kass_mint: Pubkey,
    pub usdc_mint: Pubkey,
    pub stake_vault: Pubkey, // PDA token account holding all KASS bonds/stakes
    pub deadline: i64,       // unix; proposals rejected before this
    pub phase_ends_at: i64,  // end of the current window
    pub twap_window: i64,    // per-oracle, seconds
    pub options_count: u8,   // number of categorical options
    pub phase: u8,           // Phase as u8
    pub proposer_count: u16,
    pub surviving_count: u16, // proposers not disqualified
    pub fact_count: u16,
    // Conservation accumulator; equals the `stake_vault` balance UNTIL a
    // challenge splits a proposer's bond into a MetaDAO conditional vault —
    // Task 13 conservation must also count conditional-vault-held KASS recorded
    // on the corresponding `Market` (`open_challenge` does NOT decrement this).
    pub total_oracle_stake: u64,
    pub bond_pool: u64,          // accumulated slashed KASS (base units)
    pub dispute_bond_total: u64, // Σ proposer bonds, fixed at dispute start; fact-quorum denominator
    pub settled_count: u16,      // facts settled so far (drives incremental finalize)
    pub ai_finalized_count: u16, // proposers ai-finalized so far (drives incremental finalize_ai_claims)
    pub bump: u8,
    // Final resolved categorical option, written by `finalize_oracle`. CONTRACT:
    // it is the winning option ONLY when `phase == Resolved`. On the terminal
    // [`Phase::InvalidDeadend`] (tie / no survivors) finalize_oracle stamps it
    // with the loud `CLAIM_OPTION_NONE` (0xFF) sentinel, so a consumer that
    // forgets to gate on `phase == Resolved` reads `0xFF` rather than a plausible
    // "option 0 won." Before finalize (any non-terminal phase) it is its zeroed
    // default and must not be read. (Originally absorbed the former `_pad1[1]`;
    // Oracle has since grown — see the struct docstring for the current LEN.)
    pub resolved_option: u8,
    // Number of OPEN (created-but-not-yet-settled) challenge decision markets.
    // `open_challenge` does `checked_add(1)` when it creates a Market;
    // `settle_challenge` does `checked_sub(1)` when it sets `market.settled`.
    // Task 12's `finalize_oracle` REQUIRES this == 0 before recomputing the
    // final plurality, so an unsettled challenged proposer can never be wrongly
    // counted as surviving. (Originally fit the former `_pad1`; Oracle has since
    // grown — see the struct docstring for the current LEN.)
    pub open_challenge_count: u16,
    // NOTE: the former `prompt_hash` [u8;32] lived here. It was write-only (never
    // read on-chain); the plaintext subject now lives on-chain in the companion
    // `[b"oracle_meta", oracle]` account, so the hash was removed. `threshold_num`
    // (8-aligned) follows `open_challenge_count` directly — the struct shrank by
    // 32 bytes with no padding (Pod derive enforces this).
    // ---- Governable params snapshotted from `Protocol` at create_oracle (F2) -
    // Read by the downstream processors instead of the `config.rs` consts; equal
    // to the consts by default so behavior is unchanged.
    pub threshold_num: u64, // fact-quorum supermajority (finalize_facts)
    pub threshold_den: u64, // fact-quorum supermajority (finalize_facts)
    pub market_threshold_num: u64, // slash-trigger margin (settle_challenge; widened to u128 on use)
    pub market_threshold_den: u64, // slash-trigger margin (settle_challenge; widened to u128 on use)
    pub flip_slash_num: u64,       // flip-slash fraction (finalize_ai_claims)
    pub flip_slash_den: u64,       // flip-slash fraction (finalize_ai_claims)
    pub phase_window: i64,         // dispute phase window seconds
    pub proposal_window: i64,      // proposal-registration window seconds
    // ---- Reserved (settlement-era; snapshotted but no on-chain reader yet) ---
    pub fact_vote_slash_num: u64,
    pub fact_vote_slash_den: u64,
    pub reward_proposer_weight: u64,
    pub reward_fact_weight: u64,
    // ---- Challenge-fee config snapshot (Task C1) -----------------------------
    // Directional challenge-market fees, snapshotted from `Protocol` at
    // create_oracle (so an in-flight market keeps its rates if governance
    // retunes). USDC fee on a FAILED challenge (→ proposer) and KASS fee on a
    // SUCCESSFUL challenge (→ challenger); consumed by settle (Task C2).
    pub challenge_fail_usdc_fee_num: u64,
    pub challenge_fail_usdc_fee_den: u64,
    pub challenge_success_kass_fee_num: u64,
    pub challenge_success_kass_fee_den: u64,
    // ---- Settlement resolution totals (Task S1) ------------------------------
    // Stamped at resolution for the per-staker S2 pull-claims to read; all 0
    // until then (and 0 at create). NO token movement is done in S1 — these are
    // pure accumulators/stamps the later claim instructions consume.
    //
    // `total_correct_proposer_stake`: Σ `bond` over SURVIVING proposers whose
    //   `claim_option == resolved_option`. Stamped by `finalize_oracle` on the
    //   Resolved branch (the pro-rata denominator for the proposer reward bucket).
    // `total_approved_fact_stake`: Σ (`fact.stake` + `fact.approve_stake`) over
    //   AGREED facts (submitter stake + approve-voter stake that earns the
    //   fact_rate). Accumulated incrementally by `finalize_facts` as facts settle.
    // `reward_pool`: the distributable reward pool finalized at resolution. On
    //   Resolved it is set to `bond_pool` (S3 will fold `reward_emission` in here:
    //   `reward_pool = bond_pool + reward_emission`). Left 0 on InvalidDeadend.
    pub total_correct_proposer_stake: u64,
    pub total_approved_fact_stake: u64,
    pub reward_pool: u64,
    // ---- Emission minted at creation (Task S3) -------------------------------
    // KASS minted into `stake_vault` by `create_oracle` from the supply reservoir
    // (`reward_emission = (total_supply_cap − kass_supply) · emission_num/den`,
    // computed AFTER the EMA fee burn so the burn boosts the same-tx reservoir),
    // recorded here. On the `Resolved` branch `finalize_oracle` folds it into
    // `reward_pool` (`reward_pool = bond_pool + reward_emission`); on
    // `InvalidDeadend` it is BURNED back from `stake_vault` to the reservoir so a
    // dead-end leaks no emission. 0 when emission is disabled (`total_supply_cap
    // == 0` or `emission_num == 0`) — the genesis/disabled default.
    pub reward_emission: u64,
}

impl Oracle {
    pub const LEN: usize = core::mem::size_of::<Oracle>();

    /// The oracle PDA seed prefix: the account lives at `[SEED_PREFIX, nonce_le]`.
    pub const SEED_PREFIX: &'static [u8] = b"oracle";

    /// Decode the stored phase discriminant.
    pub fn phase(&self) -> Option<Phase> {
        Phase::from_u8(self.phase)
    }

    /// Write the phase discriminant.
    pub fn set_phase(&mut self, p: Phase) {
        self.phase = p as u8;
    }

    /// The oracle PDA's program-signer seeds `[b"oracle", nonce_le, [bump]]` — the
    /// single source of truth every processor uses to sign token moves out of the
    /// oracle's vaults. The caller owns the `nonce_le` + `bump` buffers (they must
    /// outlive the returned `Seed`s).
    pub fn signer_seeds<'a>(nonce_le: &'a [u8; 8], bump: &'a [u8; 1]) -> [Seed<'a>; 3] {
        [
            Seed::from(Self::SEED_PREFIX),
            Seed::from(nonce_le.as_ref()),
            Seed::from(bump.as_ref()),
        ]
    }
}

/// A proposer's commitment within an oracle. `size_of == 96`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Proposer {
    pub account_type: u8, // AccountType::Proposer
    pub _pad_hdr: [u8; 7],
    pub oracle: Pubkey,
    pub authority: Pubkey,
    pub bond: u64,           // locked KASS
    pub original_option: u8, // value at proposal time (no proofs)
    // CONTRACT: `claim_option` MUST be initialized to `CLAIM_OPTION_NONE`
    // (0xFF) when a Proposer account is created — NOT left zeroed. A zeroed
    // value (0) would be misread as a valid claim for option 0, escaping the
    // no-show full-slash in `finalize_ai_claims` and counting as a real vote in
    // the Task 8 plurality. The proposer-registration / propose processor (not
    // yet built) must set it; the test harness already does.
    pub claim_option: u8, // value after AI claim; CLAIM_OPTION_NONE = not yet submitted
    pub disqualified: u8, // bool
    pub slashed: u8,      // bool
    pub flipped: u8,      // bool: claim_option != original_option
    pub bump: u8,
    pub ai_finalized: u8, // bool: settled by finalize_ai_claims (idempotency marker)
    pub _pad: [u8; 1],
    // KASS slashed from this proposer into the oracle's `bond_pool`. Set
    // authoritatively on EVERY slash path: `finalize_ai_claims` (no-show => bond;
    // flip => bond*FLIP_SLASH_NUM/FLIP_SLASH_DEN), `settle_challenge`
    // (challenge-fail => bond), and the `finalize_facts` no-facts dead-end
    // (=> bond). Invariant: a proposer's contribution to `bond_pool` always
    // equals its `slashed_amount`, so the deferred settlement layer (and Task 13
    // conservation) reconciles uniformly without a path-specific special case.
    pub slashed_amount: u64,
}

impl Proposer {
    pub const LEN: usize = core::mem::size_of::<Proposer>();

    pub fn is_disqualified(&self) -> bool {
        self.disqualified != 0
    }
    pub fn is_slashed(&self) -> bool {
        self.slashed != 0
    }
    pub fn is_flipped(&self) -> bool {
        self.flipped != 0
    }
    pub fn is_ai_finalized(&self) -> bool {
        self.ai_finalized != 0
    }
}

/// A fact submitted in support of an option. `size_of == 336`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Fact {
    pub account_type: u8, // AccountType::Fact
    pub _pad_hdr: [u8; 7],
    pub oracle: Pubkey,
    pub proposer: Pubkey, // who submitted the fact
    pub content_hash: [u8; 32],
    pub stake: u64,
    pub approve_stake: u64,   // running tally
    pub duplicate_stake: u64, // running tally of "duplicate" votes
    pub uri_len: u16,
    pub agreed: u8,    // set at finalize: 1 if accepted
    pub duplicate: u8, // set at finalize: 1 if duplicate-dominant
    pub settled: u8,   // bool
    pub bump: u8,
    pub _pad: [u8; 2],
    pub uri: [u8; 200],
}

impl Fact {
    pub const LEN: usize = core::mem::size_of::<Fact>();

    pub fn is_agreed(&self) -> bool {
        self.agreed != 0
    }
    pub fn is_duplicate(&self) -> bool {
        self.duplicate != 0
    }
    pub fn is_settled(&self) -> bool {
        self.settled != 0
    }
}

/// A stake-weighted vote on a fact. `size_of == 88`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct FactVote {
    pub account_type: u8, // AccountType::FactVote
    pub _pad_hdr: [u8; 7],
    pub fact: Pubkey,
    pub voter: Pubkey,
    pub stake: u64,
    pub kind: u8, // 0 = approve, 1 = duplicate
    pub bump: u8,
    pub _pad: [u8; 6],
}

impl FactVote {
    pub const LEN: usize = core::mem::size_of::<FactVote>();
}

/// A pinned-model AI claim for a proposer's option. `size_of == 208`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct AiClaim {
    pub account_type: u8, // AccountType::AiClaim
    pub _pad_hdr: [u8; 7],
    pub oracle: Pubkey,
    pub proposer: Pubkey,
    pub model_id: [u8; 32],    // hash/ident of pinned model
    pub params_hash: [u8; 32], // hash of declared params (temp, seed, ...)
    pub io_hash: [u8; 32],     // hash(prompt + agreed facts + raw response)
    pub option: u8,
    pub challenged: u8, // bool
    pub bump: u8,
    pub _pad: [u8; 5],
    // The proposer's HUMAN authority (== `proposer.authority`), stamped at submit
    // by `submit_ai_claim`. Recorded on the claim itself so the settlement-era
    // `close_ai_claim` (Task S4) routes the reclaimed rent to the authority
    // DIRECTLY — without loading the `Proposer` account, which `claim_proposer`
    // may have already closed. Makes the close ORDER-INDEPENDENT (rent never
    // stranded). Appended at offset 176 (clean ABI addition; all prior offsets
    // unchanged).
    pub authority: Pubkey,
}

impl AiClaim {
    pub const LEN: usize = core::mem::size_of::<AiClaim>();

    pub fn is_challenged(&self) -> bool {
        self.challenged != 0
    }
}

/// A challenge decision-market binding for one [`AiClaim`]. `size_of == 416`.
///
/// Created lazily by `open_challenge` only when a claim is actually challenged
/// — uncontested claims have NO `Market` account (markets are dormant by
/// default, design §6). It RECORDS the MetaDAO accounts the challenger composed
/// (a binary pass/fail `question` whose resolver is the Kassandra oracle PDA, a
/// KASS conditional vault, a USDC conditional vault, and the pass/fail AMMs),
/// the oracle-PDA-owned conditional-KASS destinations the proposer's bond was
/// split into, and the challenger's committed USDC — so `settle_challenge`
/// (Task 11) can read the TWAP, resolve the question, and redeem from the exact
/// recorded accounts (no off-chain bookkeeping). The security-critical bindings
/// (question.oracle, vault underlying mints, dest owner/mint) are verified at
/// creation; this struct is the durable record of that binding.
///
/// # Market PDA seeds (CONTRACT)
/// `[b"market", ai_claim_pubkey]`, program = [`crate::ID`].
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Market {
    pub account_type: u8, // AccountType::Market
    pub _pad_hdr: [u8; 7],
    pub oracle: Pubkey,
    pub ai_claim: Pubkey,
    pub proposer: Pubkey,
    pub challenger: Pubkey,
    pub question: Pubkey,   // MetaDAO binary question (resolver == oracle PDA)
    pub kass_vault: Pubkey, // MetaDAO conditional vault, underlying == oracle.kass_mint
    pub usdc_vault: Pubkey, // MetaDAO conditional vault, underlying == oracle.usdc_mint
    // DEFERRED-MUST-VERIFY-IN-TASK-11: only owner==AMM_ID was checked at
    // open_challenge; settle_challenge MUST verify each AMM is bound to this
    // market's pass/fail conditional (KASS,USDC) mint pair and that
    // pass_amm != fail_amm before reading its TWAP.
    pub pass_amm: Pubkey, // outcome-0 (pass) AMM
    pub fail_amm: Pubkey, // outcome-1 (fail) AMM
    // Oracle-PDA-owned conditional-KASS token accounts the proposer's bond was
    // split into (outcome 0 = pass, 1 = fail). Verified owner==oracle PDA and
    // mint==derived conditional KASS mint at creation; Task 11 redeems/settles
    // from exactly these.
    pub oracle_pass_kass: Pubkey,
    pub oracle_fail_kass: Pubkey,
    // Market-owned USDC escrow token account holding the challenger's staked
    // USDC (Task C1). SPL token account on `oracle.usdc_mint`, token authority =
    // the oracle PDA (mirrors `oracle.stake_vault`), at PDA
    // `[b"challenge_usdc", market]`. `open_challenge` creates + funds it;
    // settle (Task C2) returns it / carves the directional USDC fee.
    pub challenger_usdc_vault: Pubkey,
    pub twap_end: i64, // now + oracle.twap_window; settle allowed only after
    // Challenger's escrowed USDC (Task C1): computed on-chain at open_challenge
    // as `bond × kass_price` (raw USDC base units) and actually transferred into
    // `challenger_usdc_vault` — no longer an untrusted payload value.
    pub challenger_usdc: u64,
    pub settled: u8, // bool; set by settle_challenge (Task 11)
    pub bump: u8,
    pub _pad: [u8; 6],
}

impl Market {
    pub const LEN: usize = core::mem::size_of::<Market>();

    pub fn is_settled(&self) -> bool {
        self.settled != 0
    }
}

/// Protocol singleton: the program's global configuration record. `size_of == 368`.
///
/// Created once by `init_protocol` and never re-initialized. Pins the canonical
/// KASS/USDC mints (so `create_oracle`'s fee-burn cannot be spoofed with a fake
/// KASS mint) and carries the dynamic creation-fee EMA state used by Task H2.
///
/// # Governance linkage (Task F1)
/// `dao_authority` is the **Squads v4 multisig VAULT PDA** that gates the
/// privileged `set_config`/`resolve_deadend` instructions; `kass_dao` is the
/// futarchy `Dao` account whose embedded spot AMM is the KASS price source
/// (F5). Both are zero (unset) at `init_protocol` and recorded once by
/// `set_governance` (the one-time admin→DAO handoff). `governance_set` is the
/// one-shot flag (see `set_governance` for the trust model).
///
/// # Governable monetary params (Task F1)
/// The global monetary knobs (`emission_*`, `total_supply_cap`, and the fee-EMA
/// params) live here so `set_config` (F3) can retune them and `create_oracle`
/// can read them from state. F1 only ADDS them and defaults them to the current
/// `config.rs` consts so behavior is unchanged; the config-as-state migration
/// (wiring `create_oracle` to read these instead of the consts) is F2.
///
/// # Protocol PDA seeds (CONTRACT)
/// `[b"protocol"]` (singleton), program = [`crate::ID`].
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Protocol {
    pub account_type: u8, // AccountType::Protocol
    pub _pad_hdr: [u8; 7],
    pub admin: Pubkey,     // the initializer; gates the one-time set_governance
    pub kass_mint: Pubkey, // canonical KASS mint; oracles must match this
    pub usdc_mint: Pubkey, // canonical USDC mint; oracles must match this
    // Fixed-point EMA accumulator of recent oracle-creation activity. 0 at
    // genesis (first creation is free); rises with creation frequency and decays
    // when idle. Drives the dynamic creation fee in Task H2. Unused (always 0)
    // until then.
    pub fee_ema: u64,
    // Unix timestamp of the most recent oracle creation, for the EMA decay in
    // Task H2. 0 at genesis.
    pub last_creation_unix: i64,
    pub bump: u8,
    // 1 once `set_governance` has recorded `dao_authority`/`kass_dao`; 0 before
    // (the admin→DAO handoff is one-shot, see `set_governance`).
    pub governance_set: u8,
    pub _pad: [u8; 6],
    // Squads v4 multisig VAULT PDA — the signer that gates `set_config` (F3) and
    // `resolve_deadend` (F4). Zero until `set_governance` records it.
    pub dao_authority: Pubkey,
    // Futarchy `Dao` account; its embedded spot AMM is F5's KASS price source.
    // Zero until `set_governance` records it. STORED (not re-derived) because the
    // `Dao` account's post-`amm` fields sit at variable offsets (F0 finding).
    pub kass_dao: Pubkey,
    // ---- Governable monetary params (reserved by F1, retuned by F3) ----------
    // Emission rate as a fraction `emission_num / emission_den`. Settlement sets
    // the full semantics; F1 reserves the fields (defaulted 0/1 — no emission,
    // denominator never zero) so the layout and `set_config` plumbing exist now.
    pub emission_num: u64,
    pub emission_den: u64,
    // Hard cap on circulating KASS supply (settlement-era; F1 reserves it as 0).
    pub total_supply_cap: u64,
    // Mirror of the `config.rs` fee-EMA consts so `create_oracle` can later read
    // them from state (F2). F1 defaults them to the current consts (no behavior
    // change): `FEE_EMA_HALFLIFE_SECS`, `FEE_PER_EMA_UNIT`, `FEE_EMA_INCREMENT`.
    pub fee_ema_halflife: i64,
    pub fee_per_ema_unit: u64,
    pub fee_ema_increment: u64,
    // ---- Governable behavioral params (F2 — mutable source, set_config edits) -
    // Snapshotted onto each `Oracle` at `create_oracle`. `init_protocol` defaults
    // them to the current `config.rs` consts so behavior is unchanged. The
    // active ones are read by the downstream processors via the per-oracle
    // snapshot, never from `Protocol` directly.
    pub threshold_num: u64,        // fact-quorum supermajority (THRESHOLD_NUM)
    pub threshold_den: u64,        // fact-quorum supermajority (THRESHOLD_DEN)
    pub market_threshold_num: u64, // slash-trigger margin (MARKET_THRESHOLD_NUM; u128 on use)
    pub market_threshold_den: u64, // slash-trigger margin (MARKET_THRESHOLD_DEN; u128 on use)
    pub flip_slash_num: u64,       // flip-slash fraction (FLIP_SLASH_NUM)
    pub flip_slash_den: u64,       // flip-slash fraction (FLIP_SLASH_DEN)
    pub phase_window: i64,         // dispute phase window seconds (PHASE_WINDOW)
    pub proposal_window: i64,      // proposal-registration window seconds (PROPOSAL_WINDOW)
    // ---- Reserved (settlement-era; defaulted, no reader yet) -----------------
    pub fact_vote_slash_num: u64,
    pub fact_vote_slash_den: u64,
    pub reward_proposer_weight: u64,
    pub reward_fact_weight: u64,
    // ---- Challenge-fee config (Task C1; mutable source, snapshotted to Oracle)
    // USDC fee on a FAILED challenge (→ proposer) and KASS fee on a SUCCESSFUL
    // challenge (→ challenger), each a `num/den` fraction. Defaulted by
    // `init_protocol` (1/100 each), retuned by `set_config` (den>0, num≤den).
    pub challenge_fail_usdc_fee_num: u64,
    pub challenge_fail_usdc_fee_den: u64,
    pub challenge_success_kass_fee_num: u64,
    pub challenge_success_kass_fee_den: u64,
}

impl Protocol {
    pub const LEN: usize = core::mem::size_of::<Protocol>();

    /// Whether `set_governance` has recorded the DAO linkage.
    pub fn is_governance_set(&self) -> bool {
        self.governance_set != 0
    }
}

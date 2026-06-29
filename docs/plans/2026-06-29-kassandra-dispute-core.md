# Kassandra Dispute Core — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build and test the novel dispute-resolution core of Kassandra — fact proposal/voting, AI-claim resubmission, MetaDAO decision-market challenge with slash, and plurality recompute — all driven by LiteSVM tests, starting from a seeded disputed oracle.

**Architecture:** A single Pinocchio Solana program (no Anchor) with fixed-size, zero-copy (`bytemuck`) account layouts and a manual instruction dispatcher. Dispute phases are enforced by an on-chain phase enum + clock-gated windows. The decision market reuses MetaDAO's deployed `conditional-vault` + `amm` programs via hand-built CPI (Anchor sighash discriminators + Borsh args), loaded into LiteSVM from downloaded `.so` binaries. Upstream phases (create/propose) are NOT built here; tests seed a disputed oracle directly.

**Tech Stack:** Rust, `pinocchio`, `bytemuck`, `litesvm`, `solana-sdk` (test-only), `spl-token`, MetaDAO `conditional-vault`/`amm` programs.

**Source of truth:** `docs/plans/2026-06-29-kassandra-design.md` (design). Invariants in §9 of that doc are the fuzz targets here.

---

## Conventions

- **TDD always:** write the failing test, run it red, implement minimally, run it green, commit.
- **Commit message format:** `feat(scope): summary` / `test(scope): summary` / `chore(scope): summary`.
- **Run all tests:** `cargo test -p kassandra-program` unless a narrower target is given.
- **All on-chain accounts** are fixed-size `#[repr(C)]` `bytemuck::Pod` structs. Variable
  content (fact evidence) lives off-chain; on-chain we store a 32-byte content hash + a
  fixed 200-byte URI buffer + a `u16` URI length.
- **Amounts** are `u64` base units of KASS (9 decimals) / USDC (6 decimals).
- **PDAs:** seeds documented per account. Bumps stored in the account.

---

## Implementation deltas (live state — supersedes embedded draft snippets)

The code snippets in tasks below are the original draft. As of Task 4, the live code differs in these ways (later tasks MUST follow the live state, not the draft):

- **Pinocchio 0.8** (not 0.7). The workspace unifies on `pinocchio = "0.8"`; `pinocchio-pubkey` provides the `pubkey!` macro. `entrypoint!` is gated behind `#[cfg(not(feature = "no-entrypoint"))]`. `bytemuck` has `derive` + `min_const_generics`.
- **Account-type discriminator:** every Pod account starts with an 8-byte header `account_type: u8` + `_pad_hdr: [u8;7]` (`AccountType { Uninitialized=0, Oracle=1, Proposer=2, Fact=3, FactVote=4, AiClaim=5 }`). Set on init; assert on load.
- **Live struct sizes** (re-pinned in `tests/state_layout.rs`): Oracle **224**, Proposer **88**, Fact **336**, FactVote **88**, AiClaim **176**.
- **`Oracle.proposer_count` / `surviving_count` are `u16`** (not `u8`) — affects slash/plurality math in Tasks 6/7/11/12.
- **Program ID:** `KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY`.
- **Shared guards** (`src/processor/guards.rs`): `assert_owned_by_program`, `assert_signer`, `assert_key(ai, &expected)`, `load_oracle(ai, program_id) -> Result<Oracle>` (owner + len + tag + `pod_read_unaligned`), `create_pda(...)`. Reuse these in every processor.
- **Locked PDA seeds:** Oracle `[b"oracle", &nonce.to_le_bytes()]`; Proposer `[b"proposer", oracle, authority]`; Fact `[b"fact", oracle, content_hash]`.
- **KassandraError discriminants (append-only):** NotImplemented=0, WrongPhase=1, WindowClosed=2, WindowNotElapsed=3, Unauthorized=4, InvalidAccount=5, DuplicateFact=6, ZeroStake=7.
- **Harness** (`tests/common/mod.rs`): `seed_disputed_oracle`, `seed_program_account`, `fund_kass`, `set_phase`, `send` (rotates blockhash), `warp` (advances `unix_timestamp`, +1 slot only — a `warp_slots` variant is still needed for the TWAP tasks 11-12), accessors via `pod_read_unaligned`, public `oracle_pda`/`proposer_pda`.
- **Known deferred:** pre-funded-PDA griefing on fact creation (Allocate+Assign is the future fix); `stake == 0` facts are rejected (`ZeroStake`).
- **Quorum rule RESOLVED (affects Task 6):** the fact-approval denominator is a NEW fixed `Oracle.dispute_bond_total: u64` field (= sum of proposer bonds, set when the dispute starts; the harness seeds it = Σ bonds). A fact is agreed iff `approve_stake > duplicate_stake` AND `approve_stake * THRESHOLD_DEN >= dispute_bond_total * THRESHOLD_NUM`. Do NOT use `total_oracle_stake` as the denominator (it grows with vote stakes → dilution + griefing). `total_oracle_stake` remains the conservation accumulator (== vault balance). Task 6 must add this field (re-pinning ABI sizes) and the harness must set it.
- **As of Task 5:** `Ix::AdvancePhase=7`; `KassandraError::DuplicateVote=8`; FactVote PDA seeds `[b"vote", fact, voter]`; `config::PHASE_WINDOW: i64 = 3600`; guard `load_fact`.
- **As of Task 6:** `Oracle.dispute_bond_total` @184, `Oracle.settled_count: u16` @192 (Oracle::LEN **232**, prompt_hash @200). `config::THRESHOLD_NUM=2/THRESHOLD_DEN=3`. Errors `IncompleteFactSet=9, AlreadySettled=10, NoDisputeBond=11`. Guard `load_proposer`. `finalize_facts` is **incremental** (settles any subset; advances to AiClaim only when `settled_count==fact_count`; no-facts branch slashes proposer subsets until `surviving_count==0` → InvalidDeadend). Rejected fact → **full** `fact.stake` into `bond_pool` (counter; no token CPI). Harness: `set_dispute_bond_total`, seeds `settled_count=0`, `dispute_bond_total=Σ bonds`.
- **As of Task 7:** `Ix::FinalizeAiClaims=8`. Errors `DuplicateClaim=12, InvalidOption=13`. `Oracle.ai_finalized_count: u16` @194 (Oracle::LEN UNCHANGED **232**; bump now @196, `_pad1:[u8;3]`). **Proposer::LEN grew 88 → 96**: `ai_finalized: u8` @86 (`_pad:[u8;1]` @87), helper `is_ai_finalized()`, and `slashed_amount: u64` @88 (KASS this proposer contributed to `bond_pool`; bond offset still @72). `Proposer.claim_option` carries a CONTRACT doc: MUST init to `CLAIM_OPTION_NONE` (0xFF), else a zeroed value reads as a vote for option 0 and escapes no-show slashing / pollutes Task 8 plurality. `config::FLIP_SLASH_NUM=1/FLIP_SLASH_DEN=2` (50% flip slash). AiClaim PDA seeds `[b"claim", oracle, proposer]`. `submit_ai_claim` accounts: `[0]oracle(w),[1]proposer(w),[2]ai_claim(w,uninit),[3]authority(signer,w),[4]system`; payload `model_id[32]++params_hash[32]++io_hash[32]++option u8` (exact 97 bytes); phase/window gates run BEFORE semantic checks (phase-first convention). `finalize_ai_claims` is **incremental** like finalize_facts (oracle + proposer tail): no-show (`claim_option==NONE`) → FULL slash + disqualify + `surviving_count--`, `slashed_amount=bond`; flipped → PARTIAL slash `bond*1/2` into `bond_pool`, stays surviving, `slashed_amount=slash`; honest → no slash (`slashed_amount=0`). Invariant: each proposer's `bond_pool` contribution == its `slashed_amount`. Marks `ai_finalized`; advances to `Challenge` (window=now+PHASE_WINDOW) when `ai_finalized_count==proposer_count`. No token CPI. Disqualified-before-AiClaim proposers are marked+counted, not re-slashed (defensive). Harness: `set_proposer_disqualified`.
- **As of Task 8:** pure `plurality(&[u8]) -> Plurality{Winner(u8),Tie,NoSurvivors}` in `src/plurality.rs` (allocation-free; caller filters disqualified; `CLAIM_OPTION_NONE` must not be passed).
- **As of Task 9 (MetaDAO CPI):** IDs — conditional_vault `VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg` (v0.4.0), amm `AMMyu265tkBpRW21iGQxKGLaves3gKm2JcMUqfXNSpqD` (v0.4 standalone — **confirmed**; built-in TWAP matches §6). Fixtures committed + sha-pinned in `scripts/fetch-metadao.sh`. `src/cpi/metadao.rs`: discriminators, PDA seeds, no-alloc LE encoders, invoke wrappers. **Real-API facts:** `initialize_question(question_id[32], oracle:Pubkey, num_outcomes:u8)` (binary = num_outcomes==2); every ix is `#[event_cpi]` → trailing `event_authority` PDA `[b"__event_authority"]` + program before remaining accounts; `split_tokens` remaining accts `[mint_0..n, user_0..n]`; seeds Question `[b"question", question_id, oracle, [num_outcomes]]`, Vault `[b"conditional_vault", question, underlying_mint]`, cond-mint `[b"conditional_token", vault, [i]]`. Follow-ups for 10/11: (a) PDA-derivation fns untested → add a guard test deriving via the module fns; (b) `resolve_question` arg has `payout_numerators: Vec<u32>` (4-byte LE len prefix — NOT flat concat) → Task 11 needs a length-prefixed encoder; (c) `event_authority_pda` must be parameterized by program id (AMM needs its own).
- **As of Task 10:** `Ix::OpenChallenge=4` implemented; `AccountType::Market=6`; `KassandraError::AlreadyChallenged=14`. New `Market` Pod account (`Market::LEN` **320**, pinned in `tests/state_layout.rs`): 9 pubkeys after the 8-byte header (`oracle`@8, `ai_claim`@40, `proposer`@72, `challenger`@104, `question`@136, `kass_vault`@168, `usdc_vault`@200, `pass_amm`@232, `fail_amm`@264) then `twap_end:i64`@296, `challenger_usdc:u64`@304, `settled:u8`@312, `bump`@313, `_pad[6]`. **Market PDA seeds `[b"market", ai_claim]`** (created lazily — dormant by default; uncontested claims have NO Market account). `open_challenge` accounts (20): `[0]oracle(w, also split authority),[1]ai_claim(w),[2]proposer(w),[3]market(w,uninit),[4]challenger(signer,w,pays rent),[5]question(ro),[6]kass_vault(w),[7]usdc_vault(ro),[8]pass_amm(ro),[9]fail_amm(ro),[10]stake_vault(w),[11]kass_vault_underlying_ata(w),[12]pass_kass_mint(w),[13]fail_kass_mint(w),[14]oracle_pass_kass(w),[15]oracle_fail_kass(w),[16]conditional_vault prog,[17]token prog,[18]system prog,[19]cv_event_authority(ro)]`. Payload (16 B) = `challenger_usdc:u64 LE ++ oracle_nonce:u64 LE`. **Oracle nonce handling:** Oracle does NOT store its nonce (no ABI re-pin); the nonce is passed in the payload and verified by re-deriving `[b"oracle", nonce_le]` == oracle key with `bump == oracle.bump`, then used as the program-signed split authority seeds `[b"oracle", nonce_le, [bump]]`. The proposer's `bond` KASS is split from `oracle.stake_vault` into the KASS conditional vault (CPI `split_tokens`, oracle-PDA signed), minting pass/fail conditional KASS to two oracle-PDA-owned token accounts (Task 11 redeems them). `total_oracle_stake` is intentionally NOT decremented (the KASS is still in-system, now escrowed in the conditional vault recorded on the Market; Task 13 conservation must count conditional-vault-held KASS). **MetaDAO layout offsets verified from the deployed v0.4.0 source (`metaDAOproject/programs`, `declare_id!==VLTX1…`):** `Question` (8-byte disc): `oracle`@40, `payout_numerators:Vec<u32>` len@72 (== `num_outcomes`, since `initialize_question` sets `vec![0; num_outcomes]`); `ConditionalVault` (disc): `question`@8, `underlying_token_mint`@40, `underlying_token_account`@72, `conditional_token_mints:Vec`@104. Security bindings checked: question.oracle==oracle PDA, num_outcomes==2, KASS/USDC vault `question`==question + underlying mint==oracle.kass_mint/usdc_mint, kass_vault underlying ATA matches, pass/fail KASS mints == `conditional_token_mint_pda(kass_vault,0/1)`. **DEFERRED:** AMM binding is best-effort (only `owner==AMM_ID` — the standalone v0.4 AMM layout is not re-derivable from current source; deeper pass/fail-mint binding deferred to Task 11). **Task 9 follow-ups done:** (a) factored host-runnable seed-assembly helpers `metadao::{question,vault,conditional_token_mint,event_authority}_seeds` (the SBF-only `*_pda` wrappers reuse them); `metadao_cpi.rs` derives PDAs via the helpers (proven against the real binary) + a `seed_helpers_match_documented_layout` drift-guard test; (c) `event_authority_pda(program_id)` now parameterized by program id. Harness: `seed_program_account_at`, `send_many`. Guard `load_ai_claim`. No settle/TWAP/resolve logic (Task 11).
- **Task 10 DECISION (decompose):** challenger sets up the MetaDAO question (oracle = Kassandra oracle PDA so settle can resolve it; num_outcomes=2 → 0=pass,1=fail), the KASS + USDC conditional vaults, and the pass/fail AMMs in their own txs (SDK/test composes them, like Task 9). Kassandra `open_challenge` then: verifies those accounts are bound to this claim/oracle, does the **program-signed** split of the proposer's escrowed KASS (from `oracle.stake_vault`, signed by oracle PDA) into pass/fail conditional KASS, records a `Market` PDA (seeds `[b"market", ai_claim]`, `AccountType::Market`), marks `ai_claim.challenged=1`. AMM liquidity + trading exercised in tests; `resolve_question`-based settlement is Task 11.

---

## Task 0: Workspace scaffolding + LiteSVM smoke test

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `programs/kassandra/Cargo.toml`
- Create: `programs/kassandra/src/lib.rs`
- Create: `programs/kassandra/tests/smoke.rs`
- Create: `rust-toolchain.toml`

**Step 1: Write the workspace root `Cargo.toml`**

```toml
[workspace]
resolver = "2"
members = ["programs/kassandra"]

[workspace.dependencies]
pinocchio = "0.7"
pinocchio-system = "0.2"
pinocchio-token = "0.3"
bytemuck = { version = "1", features = ["derive"] }
```

**Step 2: Write `programs/kassandra/Cargo.toml`**

```toml
[package]
name = "kassandra-program"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "lib"]

[dependencies]
pinocchio = { workspace = true }
pinocchio-system = { workspace = true }
pinocchio-token = { workspace = true }
bytemuck = { workspace = true }

[dev-dependencies]
litesvm = "0.6"
solana-sdk = "2"
spl-token = { version = "6", features = ["no-entrypoint"] }

[features]
no-entrypoint = []
```

**Step 3: Write minimal `src/lib.rs`**

```rust
#![allow(unexpected_cfgs)]
use pinocchio::{
    account_info::AccountInfo, entrypoint, program_error::ProgramError,
    pubkey::Pubkey, ProgramResult,
};

pinocchio::nostd_panic_handler!();
entrypoint!(process_instruction);

pub const ID: Pubkey = pinocchio::pubkey::pubkey!("Kass1111111111111111111111111111111111111111");

pub fn process_instruction(
    _program_id: &Pubkey,
    _accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    if instruction_data.is_empty() {
        return Err(ProgramError::InvalidInstructionData);
    }
    Ok(())
}
```

**Step 4: Write `tests/smoke.rs`**

```rust
use litesvm::LiteSVM;

#[test]
fn program_loads() {
    let mut svm = LiteSVM::new();
    let bytes = include_bytes!("../../../target/deploy/kassandra_program.so");
    let program_id = solana_sdk::pubkey::Pubkey::new_from_array(kassandra_program::ID);
    svm.add_program(program_id, bytes);
    // Loading without panicking is the assertion.
}
```

**Step 5: Build the program SBF artifact**

Run: `cargo build-sbf --manifest-path programs/kassandra/Cargo.toml`
Expected: produces `target/deploy/kassandra_program.so`.

**Step 6: Run smoke test**

Run: `cargo test -p kassandra-program --test smoke`
Expected: PASS.

**Step 7: Commit**

```bash
git add .
git commit -m "chore(scaffold): cargo workspace, pinocchio program, litesvm smoke test"
```

> **NOTE for executor:** every subsequent task that changes on-chain code must re-run
> `cargo build-sbf ...` before `cargo test`, because LiteSVM loads the compiled `.so`.
> Add a `just build` or shell alias if helpful.

---

## Task 1: Account layouts + phase enum

**Files:**
- Create: `programs/kassandra/src/state.rs`
- Modify: `programs/kassandra/src/lib.rs` (add `pub mod state;`)
- Create: `programs/kassandra/tests/state_layout.rs`

**Step 1: Write the failing layout test**

```rust
use kassandra_program::state::*;
use core::mem::size_of;

#[test]
fn account_sizes_are_stable() {
    assert_eq!(size_of::<Oracle>(), Oracle::LEN);
    assert_eq!(size_of::<Proposer>(), Proposer::LEN);
    assert_eq!(size_of::<Fact>(), Fact::LEN);
    assert_eq!(size_of::<FactVote>(), FactVote::LEN);
    assert_eq!(size_of::<AiClaim>(), AiClaim::LEN);
    assert_eq!(Phase::Created as u8, 0);
    assert_eq!(Phase::InvalidDeadend as u8, 8);
}
```

**Step 2: Run it red**

Run: `cargo test -p kassandra-program --test state_layout`
Expected: FAIL (module/types missing).

**Step 3: Implement `state.rs`**

```rust
use bytemuck::{Pod, Zeroable};

pub type Pubkey = [u8; 32];

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
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

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Oracle {
    pub creator: Pubkey,
    pub kass_mint: Pubkey,
    pub usdc_mint: Pubkey,
    pub stake_vault: Pubkey,      // PDA token account holding all KASS bonds/stakes
    pub deadline: i64,            // unix; proposals rejected before this
    pub phase_ends_at: i64,       // end of the current window
    pub twap_window: i64,         // per-oracle, seconds
    pub options_count: u8,        // number of categorical options
    pub phase: u8,                // Phase
    pub proposer_count: u8,
    pub surviving_count: u8,      // proposers not disqualified
    pub fact_count: u16,
    pub _pad0: [u8; 2],
    pub total_oracle_stake: u64,  // quorum denominator
    pub bond_pool: u64,           // accumulated slashed KASS (base units)
    pub bump: u8,
    pub _pad1: [u8; 7],
    pub prompt_hash: [u8; 32],    // hash of fixed prompt + interpretation
}
impl Oracle { pub const LEN: usize = size_of_struct(); }

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Proposer {
    pub oracle: Pubkey,
    pub authority: Pubkey,
    pub bond: u64,                // locked KASS
    pub original_option: u8,      // value at proposal time (no proofs)
    pub claim_option: u8,         // value after AI claim; 0xFF = not yet submitted
    pub disqualified: u8,         // bool
    pub slashed: u8,              // bool
    pub flipped: u8,              // bool: claim_option != original_option
    pub bump: u8,
    pub _pad: [u8; 2],
}
impl Proposer { pub const LEN: usize = size_of_struct(); }

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Fact {
    pub oracle: Pubkey,
    pub proposer: Pubkey,         // who submitted the fact
    pub content_hash: [u8; 32],
    pub stake: u64,
    pub approve_stake: u64,       // running tally
    pub duplicate_stake: u64,     // running tally of "duplicate" votes
    pub uri_len: u16,
    pub agreed: u8,               // set at finalize: 1 if accepted
    pub duplicate: u8,            // set at finalize: 1 if duplicate-dominant
    pub settled: u8,              // bool
    pub bump: u8,
    pub _pad: [u8; 2],
    pub uri: [u8; 200],
}
impl Fact { pub const LEN: usize = size_of_struct(); }

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct FactVote {
    pub fact: Pubkey,
    pub voter: Pubkey,
    pub stake: u64,
    pub kind: u8,                 // 0 = approve, 1 = duplicate
    pub bump: u8,
    pub _pad: [u8; 6],
}
impl FactVote { pub const LEN: usize = size_of_struct(); }

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct AiClaim {
    pub oracle: Pubkey,
    pub proposer: Pubkey,
    pub model_id: [u8; 32],       // hash/ident of pinned model
    pub params_hash: [u8; 32],    // hash of declared params (temp, seed, ...)
    pub io_hash: [u8; 32],        // hash(prompt + agreed facts + raw response)
    pub option: u8,
    pub challenged: u8,           // bool
    pub bump: u8,
    pub _pad: [u8; 5],
}
impl AiClaim { pub const LEN: usize = size_of_struct(); }

// helper kept inline so each LEN equals exact struct size
const fn size_of_struct() -> usize { 0 } // replaced per-impl below
```

> **Implementation note:** `size_of_struct()` is a placeholder — instead set each
> `LEN` with `core::mem::size_of::<Self>()` via an associated const:
> `pub const LEN: usize = core::mem::size_of::<Oracle>();` etc. The layout test then
> verifies `size_of == LEN` (tautological but guards accidental `#[repr]` changes and
> documents on-chain sizes). Keep all structs `repr(C)` and padded to 8-byte alignment.

**Step 4: Run it green**

Run: `cargo test -p kassandra-program --test state_layout`
Expected: PASS.

**Step 5: Commit**

```bash
git add programs/kassandra/src/state.rs programs/kassandra/src/lib.rs programs/kassandra/tests/state_layout.rs
git commit -m "feat(state): fixed-size account layouts and phase enum"
```

---

## Task 2: Test harness for seeding a disputed oracle

This is the keystone fixture: every dispute test starts from here. It mints KASS, creates
the program-owned accounts directly (via `svm.set_account`), and returns handles.

**Files:**
- Create: `programs/kassandra/tests/common/mod.rs`
- Create: `programs/kassandra/tests/dispute_harness.rs`

**Step 1: Write the failing harness test**

```rust
mod common;
use common::*;

#[test]
fn seed_disputed_oracle_has_two_conflicting_proposers() {
    let mut ctx = TestCtx::new();
    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec { option: 0, bond: 1_000 },
        ProposerSpec { option: 1, bond: 1_000 },
    ]);
    let acc = ctx.oracle(oracle);
    assert_eq!(acc.phase, kassandra_program::state::Phase::FactProposal as u8);
    assert_eq!(acc.proposer_count, 2);
    assert_eq!(acc.total_oracle_stake, 2_000);
}
```

**Step 2: Run it red** — `cargo test -p kassandra-program --test dispute_harness` → FAIL.

**Step 3: Implement `common/mod.rs`**

Provide:
- `TestCtx { svm: LiteSVM, payer, kass_mint, usdc_mint, program_id }` with `new()` that
  creates mints (`spl-token`) and funds a payer.
- `struct ProposerSpec { option: u8, bond: u64 }`.
- `seed_disputed_oracle(&mut self, &[ProposerSpec]) -> Pubkey`:
  - derive Oracle PDA (`["oracle", nonce]`), create its `stake_vault` ATA-style PDA token
    account, mint total bonds into it,
  - build the `Oracle` struct (phase = `FactProposal`, `phase_ends_at` = now + window,
    `total_oracle_stake` = Σ bonds), write via `svm.set_account` with owner = program_id,
  - for each spec, derive Proposer PDA (`["proposer", oracle, authority]`) and write a
    `Proposer` (claim_option = 0xFF).
- Accessor helpers: `oracle(pubkey) -> Oracle`, `proposer(...)`, `fact(...)`, etc., that
  read account data and `bytemuck::from_bytes`.
- A `warp(seconds)` helper that advances LiteSVM's clock sysvar.
- A `send(ix, signers)` helper wrapping tx build/submit and returning the result.

> Use `bytemuck::bytes_of` to serialize structs into account data. Account data length =
> `T::LEN`. Set lamports to rent-exempt minimum via `svm.minimum_balance_for_rent_exemption`.

**Step 4: Run it green** — PASS.

**Step 5: Commit**

```bash
git add programs/kassandra/tests/common programs/kassandra/tests/dispute_harness.rs
git commit -m "test(harness): seed disputed oracle fixture in LiteSVM"
```

---

## Task 3: Instruction dispatch + window/clock helper

**Files:**
- Create: `programs/kassandra/src/instruction.rs` (discriminant enum + parsing)
- Create: `programs/kassandra/src/processor/mod.rs`
- Create: `programs/kassandra/src/clock.rs` (read `Clock` sysvar, phase-gate helper)
- Modify: `programs/kassandra/src/lib.rs` (route to processor)
- Create: `programs/kassandra/tests/dispatch.rs`

**Step 1: Failing test** — sending an unknown discriminant returns `InvalidInstructionData`;
sending a valid-but-unimplemented one returns a specific custom error (e.g. `NotImplemented`).

**Step 2: Red.**

**Step 3: Implement.**
- `instruction.rs`: `#[repr(u8)] enum Ix { SubmitFact=0, VoteFact=1, FinalizeFacts=2, SubmitAiClaim=3, OpenChallenge=4, SettleChallenge=5, FinalizeOracle=6 }`. First byte of `instruction_data` selects; rest is the Borsh/`bytemuck` payload.
- `clock.rs`: `fn now() -> i64` reading the Clock sysvar; `fn require_phase(o: &Oracle, p: Phase) -> ProgramResult`; `fn require_before_end(o: &Oracle) -> ProgramResult`; `fn require_after_end(o: &Oracle) -> ProgramResult`.
- `processor/mod.rs`: dispatch returning `NotImplemented` for all arms initially.
- Define a `KassandraError` enum mapped to `ProgramError::Custom(u32)`.

**Step 4: Green. Step 5: Commit** `feat(program): instruction dispatch + clock/phase gates`.

---

## Task 4: `submit_fact` (Fact Proposal window, disjoint from voting)

**Files:**
- Create: `programs/kassandra/src/processor/submit_fact.rs`
- Modify: dispatcher
- Create: `programs/kassandra/tests/submit_fact.rs`

**Behavior (design §4):**
- Allowed only in `FactProposal` phase and before `phase_ends_at`.
- Creates a `Fact` PDA (`["fact", oracle, content_hash]`); rejects exact-duplicate hash
  (account already exists).
- Transfers `stake` KASS from submitter ATA → oracle `stake_vault` (CPI `pinocchio-token`).
- Increments `oracle.fact_count` and `oracle.total_oracle_stake` by `stake`.

**Steps (TDD):**
1. Test: submit a fact in FactProposal → `Fact` exists, stake moved, counters updated. Red → implement → green.
2. Test: duplicate content_hash → fails. 
3. Test: submitting during `FactVoting` phase → fails with `WrongPhase`.
4. Test: submitting after `phase_ends_at` → fails with `WindowClosed`.
5. Commit `feat(facts): submit_fact with disjoint-window enforcement`.

> **Invariant touched:** #1 (disjoint windows), #3 (KASS conservation — stake leaves
> submitter, enters vault, `total_oracle_stake` reflects it).

---

## Task 5: Advance to FactVoting + `vote_fact` (approve / duplicate)

**Files:**
- Create: `programs/kassandra/src/processor/advance_phase.rs` (permissionless phase tick when window elapses)
- Create: `programs/kassandra/src/processor/vote_fact.rs`
- Create: `programs/kassandra/tests/vote_fact.rs`

**Behavior:**
- `advance_phase`: permissionless; if `now >= phase_ends_at` and phase is advanceable,
  move `FactProposal → FactVoting` (freeze set), set new `phase_ends_at`. Guards prevent
  skipping phases.
- `vote_fact`: only in `FactVoting`; stake-approve a fact with `kind ∈ {approve,
  duplicate}`. Creates `FactVote` PDA (`["vote", fact, voter]`) — one vote per voter per
  fact (re-vote rejected). Transfers stake to vault. **Non-exclusive:** a voter can vote
  on many facts; full stake counts on each. Updates `fact.approve_stake` /
  `fact.duplicate_stake` and `oracle.total_oracle_stake`. Open to any KASS holder.

**Steps (TDD):**
1. Test: advance FactProposal→FactVoting only after window end; before end → fails.
2. Test: approve vote increments `approve_stake`; stake moved.
3. Test: duplicate vote increments `duplicate_stake`.
4. Test: same voter voting twice on same fact → fails.
5. Test: one voter approving two different facts → both tallies get full stake (non-exclusive).
6. Test: voting in wrong phase → fails.
7. Commit `feat(facts): phase advance + approve/duplicate voting`.

> **Invariant touched:** #1, #6 (quorum tallies correct), #3.

---

## Task 6: `finalize_facts` (agreed set, settlement, no-facts dead-end)

**Files:**
- Create: `programs/kassandra/src/processor/finalize_facts.rs`
- Create: `programs/kassandra/tests/finalize_facts.rs`

**Behavior (design §4, §7):**
- Only in `FactVoting`, after window end.
- For each `Fact` (passed as remaining accounts, all facts of the oracle):
  - If `duplicate_stake > approve_stake` → mark `duplicate=1`, **ignored**, stakers **not
    slashed** (stake returned).
  - Else if `approve_stake >= threshold_num/threshold_den * total_oracle_stake` →
    `agreed=1`. (Use checked u128 math: `approve_stake * den >= total * num`.)
  - Else → rejected: partial-slash the fact's submitter stake to `bond_pool`.
- Settlement of approved-fact stakers: reward from bond pool + emissions stub (emissions
  can be a no-op counter for this milestone; real mint in tokenomics milestone).
- **No-facts case:** if `fact_count == 0` → mark all proposers `slashed`, move their bonds
  to `bond_pool`, set phase `InvalidDeadend`. Return early.
- Otherwise advance phase → `AiClaim`, set window.

**Steps (TDD):**
1. Test: fact above threshold → agreed=1.
2. Test: fact below threshold (non-duplicate) → rejected, submitter partially slashed, bond_pool grows.
3. Test: duplicate-dominant fact → ignored, stake returned, not slashed.
4. Test: zero facts → all proposers slashed, phase = InvalidDeadend.
5. Test: normal case advances to AiClaim.
6. Commit `feat(facts): finalize agreed set, settlement, no-facts deadend`.

> Threshold is **protocol-global** — define as a `const THRESHOLD_NUM/THRESHOLD_DEN`
> (default supermajority 2/3) in a `config.rs`. **Invariant touched:** #3, #6, #9.

---

## Task 7: `submit_ai_claim` (AiClaim window, full slash for no-show, partial for flip)

**Files:**
- Create: `programs/kassandra/src/processor/submit_ai_claim.rs`
- Create: `programs/kassandra/src/processor/finalize_ai_claims.rs`
- Create: `programs/kassandra/tests/ai_claim.rs`

**Behavior (design §5, §7):**
- `submit_ai_claim`: only in `AiClaim` phase, before window end, by a locked-in proposer.
  Creates `AiClaim` PDA (`["claim", oracle, proposer]`) with model_id, params_hash,
  io_hash, option. Sets `proposer.claim_option = option`; if `option != original_option`,
  set `flipped=1`.
- `finalize_ai_claims`: after window end. Any proposer with `claim_option == 0xFF`
  (no-show) → **fully slashed** (`slashed=1`, `disqualified=1`, bond → bond_pool,
  `surviving_count--`). Any `flipped` proposer → **partial slash** (keeps reduced stake,
  remains surviving). Then advance → `Challenge`.

**Steps (TDD):**
1. Test: proposer submits claim → AiClaim account, claim_option set.
2. Test: claim option != original → flipped=1.
3. Test: claim in wrong phase / after window → fails.
4. Test: finalize fully slashes no-show proposer; surviving_count decremented.
5. Test: finalize partially slashes flipped proposer (still surviving).
6. Commit `feat(claims): ai-claim submission + finalize with slash rules`.

> **Invariant touched:** #3, #4 (no withdrawal — bonds only move via slash), #9.

---

## Task 8: Plurality computation over surviving proposers

**Files:**
- Create: `programs/kassandra/src/plurality.rs` (pure fn, unit-tested without SVM)
- Create: `programs/kassandra/tests/plurality.rs` (or `#[cfg(test)]` in module)

**Behavior (design §5, §7):**
- `fn plurality(options: &[(u8 /*option*/, bool /*surviving*/)]) -> PluralityResult`
  where result is `Winner(option)` or `Tie`.
- One proposer = one vote for their `claim_option`; only surviving proposers count.

**Steps (TDD):**
1. Unit test: clear winner.
2. Unit test: two-way tie → `Tie`.
3. Unit test: all disqualified (empty surviving) → `Tie`/`NoSurvivors` sentinel.
4. Commit `feat(plurality): pure plurality over surviving proposers`.

> Pure function → fast unit tests, no SBF build needed. **Invariant touched:** #7.

---

## Task 9: MetaDAO CPI groundwork — load programs, build conditional vault

**Files:**
- Create: `scripts/fetch-metadao.sh` (downloads `conditional_vault.so`, `amm.so` to `tests/fixtures/`)
- Create: `programs/kassandra/src/cpi/metadao.rs` (discriminators, account-meta builders, arg structs)
- Create: `programs/kassandra/tests/metadao_cpi.rs`
- Create: `tests/fixtures/.gitkeep`

**Behavior:**
- `fetch-metadao.sh`: use `solana program dump <PROGRAM_ID> tests/fixtures/conditional_vault.so`
  against mainnet for MetaDAO's conditional-vault and amm program IDs (document IDs in the
  script header). Committed binaries make tests hermetic.
  - **REQUIREMENT: use the latest released/deployed version of MetaDAO's programs.**
    Determine the current mainnet program IDs at execution time from MetaDAO's official
    source (their docs/GitHub `futarchy` repo or deployed program registry) — do NOT
    hardcode a stale ID from memory. Record the resolved program IDs, version/commit, and
    dump slot in the script header so the fixtures are reproducible.
- `cpi/metadao.rs`: define the exact instructions we need (`initialize_question`,
  `initialize_conditional_vault`, `split_tokens`, `merge_tokens`, `redeem_tokens`, and the
  AMM `create_amm`/`swap`/TWAP read). For each: the 8-byte Anchor sighash
  (`sha256("global:<name>")[..8]`), the ordered `AccountMeta` list, and a `#[repr(C)]`
  Borsh-serializable args struct. Provide a `invoke_signed` wrapper using Pinocchio.

**Steps (TDD):**
1. Test: `fetch-metadao.sh` output present; LiteSVM loads both programs without panic.
2. Test: via CPI from a tiny test-only instruction (or directly building the MetaDAO ix in
   the test first to confirm wire format), initialize a conditional vault over the KASS
   mint and split a proposer's locked KASS into pass-KASS/fail-KASS; assert conditional
   token balances.
3. Commit `feat(cpi): metadao program loading + conditional vault split`.

> **HIGH-RISK TASK.** Validate the wire format by first constructing the MetaDAO
> instruction *directly in the test* (not via our program) to confirm discriminators/args,
> then move it behind our CPI wrapper. Pin exact program IDs and a known slot/version of
> the `.so` in the script header.

---

## Task 10: `open_challenge` (challenger USDC, instantiate pass/fail markets)

**Files:**
- Create: `programs/kassandra/src/processor/open_challenge.rs`
- Create: `programs/kassandra/tests/open_challenge.rs`

**Behavior (design §6):**
- Only in `Challenge` phase, before window end, against a surviving, non-disqualified
  proposer's `AiClaim`.
- Challenger deposits **USDC** (split into pass-USDC/fail-USDC via vault); the proposer's
  **already-locked KASS** is split into pass-KASS/fail-KASS (program-signed, since the
  vault holds it). Seed pass and fail AMM pools (CPI `create_amm` + initial liquidity).
- Mark `ai_claim.challenged = 1`. Record market handles in a `Market` PDA
  (`["market", ai_claim]`) storing the two pool addresses + `twap_window` end.

**Steps (TDD):**
1. Test: open challenge → markets created, claim.challenged=1, USDC moved from challenger.
2. Test: challenging an already-disqualified proposer → fails.
3. Test: challenging after window → fails.
4. Test: dormant by default — no challenge means no market, zero cost (assert no Market PDA).
5. Commit `feat(challenge): open decision market via metadao cpi`.

> **Invariant touched:** #3 (USDC accounting), and the "0 trade = 0 cost" property (#test 4).

---

## Task 11: `settle_challenge` (TWAP read, slash trigger, incremental state)

**Files:**
- Create: `programs/kassandra/src/processor/settle_challenge.rs`
- Create: `programs/kassandra/tests/settle_challenge.rs`

**Behavior (design §6):**
- Callable after a market's `twap_window` elapses. Reads pass/fail TWAP from the AMM.
- If `fail_twap > pass_twap + THRESHOLD` (protocol-global threshold) → disqualify: proposer
  KASS (via vault redemption of fail side) → `bond_pool`; settle fail-side bettors in their
  favor (vault redeem). Set `proposer.disqualified=1`, `surviving_count--`.
- Else → claim survives; redeem pass side; challenger forfeits per market rules.
- **Incremental:** each settlement updates oracle state immediately.

**Steps (TDD):**
1. Test: simulate fail-favored TWAP (drive pool prices in test via swaps) → proposer disqualified, bond_pool grows.
2. Test: pass-favored / below threshold → proposer survives.
3. Test: settle before twap end → fails.
4. Commit `feat(challenge): settle via twap, slash trigger, incremental update`.

> TWAP manipulation resistance is the point — test includes a "last-block swap" that should
> NOT flip the outcome because TWAP averages over the window. **Invariant touched:** #2, #3, #8.

---

## Task 12: `finalize_oracle` (final recompute, terminal state)

**Files:**
- Create: `programs/kassandra/src/processor/finalize_oracle.rs`
- Create: `programs/kassandra/tests/finalize_oracle.rs`

**Behavior (design §6, §7):**
- Only in `Challenge`/`FinalRecompute` after the last market's window. Recompute plurality
  over surviving proposers (Task 8).
- `Winner(option)` → `Phase::Resolved`, write result; return surviving bonds; mint
  emissions (stub counter).
- `Tie` or zero survivors → `Phase::InvalidDeadend`; **return all bonds/stakes**; creator
  fee remains burned (no-op — already burned upstream).
- Close `AiClaim` accounts (reclaim rent) on resolution.

**Steps (TDD):**
1. Test: one survivor → Resolved with that option; bond returned.
2. Test: tie among survivors → InvalidDeadend; all bonds returned.
3. Test: all disqualified → InvalidDeadend.
4. Test: AiClaim accounts closed (lamports → 0, data zeroed).
5. Commit `feat(resolve): final recompute + terminal states + account closure`.

> **Invariant touched:** #7, #9 (terminal exclusivity), #10 (closure).

---

## Task 13: Invariant fuzz harness

**Files:**
- Create: `programs/kassandra/tests/invariants.rs`
- Add dev-dep: `proptest = "1"`

**Behavior:** Drive randomized but phase-legal action sequences against a seeded disputed
oracle and assert the design §9 invariants after every step:
- #2 termination (a full random dispute always reaches a terminal state within the bounded
  single round),
- #3 KASS conservation (`Σ in == Σ returned + Σ bond_pool + Σ burned + Σ emitted`) —
  track a ledger in the test and reconcile against on-chain vault + bond_pool,
- #7 plurality correctness vs an independent reference implementation in the test,
- #9 terminal exclusivity (exactly one of Resolved/InvalidDeadend).

**Steps (TDD):**
1. Write a `proptest!` strategy generating proposer/fact/vote/challenge sequences.
2. Implement the reconciliation oracle (reference model) in the test.
3. Run: `cargo test -p kassandra-program --test invariants` (allow longer timeout).
4. Commit `test(invariants): proptest fuzz of dispute-core invariants`.

---

## Out of scope for this plan (follow-on milestones)

- Happy path: `create_oracle` (dynamic EMA burn fee), `propose`, uncontested resolution.
- Real KASS emissions mint + decay schedule; bond-pool reward distribution math.
- Adversarial/economic **simulations** (separate sim crate; Schelling-bloc, thin-liquidity,
  fee-EMA, Sybil) — design §10.
- `runner/` AI runner, `sdk/`, `app/`.
- End-to-end via **surfpool + runner** — design §10.
- MetaDAO **futarchy** governance wiring for InvalidDeadend resolution.

---

## Execution note

After each task: `cargo build-sbf ...` then `cargo test`, confirm green, then commit.
Never proceed to the next task with a red bar. Use `superpowers:executing-plans`.

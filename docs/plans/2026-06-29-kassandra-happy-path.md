# Kassandra Happy Path + Proposer Registration — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build the oracle entry point the dispute core currently fakes: `init_protocol`, `create_oracle` (dynamic KASS burn fee, deadline gate, program-created stake vault), `propose` (proposer registration with bond + the on-chain `MAX_PROPOSERS` cap), and `finalize_proposals` (resolve-if-all-agree, else open the dispute by setting `dispute_bond_total` + phase `FactProposal`). This makes the full lifecycle real end-to-end and hands the conflict path off to the already-built dispute core.

**Architecture:** Extends the existing Pinocchio (0.8, no Anchor) program. Reuses the established processor template (slice-destructure accounts → `assert_*`/`load_*` guards → phase/window gate → semantic checks → CPI → checked write-back), the account-type discriminator, the locked PDA seeds, and the counter-only settlement convention (bonds/fees move physically only where required — the fee is BURNED; bonds are escrowed into the stake vault; per-staker returns/rewards remain the deferred settlement milestone).

**Tech Stack:** Rust, `pinocchio` 0.8, `bytemuck`, `litesvm`, `solana-sdk` (test-only), `spl-token`.

**Source of truth:** `docs/plans/2026-06-29-kassandra-design.md` (design §3 lifecycle, §8 tokenomics) and `docs/plans/2026-06-29-kassandra-dispute-core.md` — esp. its **"Implementation deltas (live state)"** section (authoritative running record of live types/sizes/guards/seeds/errors/harness). FOLLOW THE LIVE STATE.

---

## Conventions (unchanged from dispute core)

- **TDD always:** failing test → red → minimal impl → green → commit. `just build` (cargo build-sbf) BEFORE `cargo test` (tests `include_bytes!` the `.so`). `cargo clippy --all-targets` + `cargo fmt` clean before commit.
- **Commit messages:** `feat(scope): ...` / `test(...)` / `fix(...)`, with the `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>` trailer, git author `Kassandra <hexadecifish@gmail.com>`.
- rust-analyzer emits stale false positives (mid-edit module/Fixture errors) — rely on real cargo/just runs.
- **Append-only** `Ix` and `KassandraError` discriminants. **Re-pin** `tests/state_layout.rs` for any layout change.

## Live-state facts this milestone builds on

- `Oracle` (LEN 232): `account_type`, `creator`, `kass_mint`, `usdc_mint`, `stake_vault`, `deadline:i64`, `phase_ends_at:i64`, `twap_window:i64`, `options_count:u8`, `phase:u8` (`phase()`/`set_phase()`), `proposer_count:u16`, `surviving_count:u16`, `fact_count:u16`, `total_oracle_stake:u64`, `bond_pool:u64`, `dispute_bond_total:u64`, `settled_count:u16`, `ai_finalized_count:u16`, `bump`, `resolved_option:u8`, `open_challenge_count:u16`, `prompt_hash:[u8;32]`.
- `Proposer` (LEN 96): `account_type`, `oracle`, `authority`, `bond:u64`, `original_option:u8`, `claim_option:u8` (**MUST init to `CLAIM_OPTION_NONE`**), `disqualified/slashed/flipped/bump/ai_finalized`, `slashed_amount:u64`.
- `AccountType { Uninitialized=0, Oracle=1, Proposer=2, Fact=3, FactVote=4, AiClaim=5, Market=6 }` — APPEND `Protocol=7`.
- `Phase { Created=0, Proposal=1, FactProposal=2, ... Resolved=7, InvalidDeadend=8 }`.
- `Ix { ... FinalizeAiClaims=8 }` — APPEND `InitProtocol=9, CreateOracle=10, Propose=11, FinalizeProposals=12`.
- `KassandraError` up to `ChallengesOutstanding=16` — append new variants only.
- Locked PDA seeds: Oracle `[b"oracle", &nonce.to_le_bytes()]`; Proposer `[b"proposer", oracle, authority]`. **NEW (lock in):** Protocol `[b"protocol"]` (singleton); stake vault `[b"vault", oracle]`.
- Guards (`src/processor/guards.rs`): `assert_owned_by_program`, `assert_signer`, `assert_key`, `load_oracle/fact/proposer/ai_claim`, `create_pda`. Reuse + add `load_protocol`.
- `config.rs`: `PHASE_WINDOW=3600`, thresholds. `MAX_PROPOSERS = 60` currently lives in `finalize_oracle.rs` — **promote it to `config.rs`** so `propose` and `finalize_oracle` share one const.
- Harness `tests/common/mod.rs`: `seed_disputed_oracle`, `fund_kass`, `set_phase`, `warp`/`warp_slots`, `send`, accessors, `oracle_pda`/`proposer_pda`, mints (kass 9dp / usdc 6dp). The program is deployed in `TestCtx::new()`.

---

## Task H0: Protocol state account + `init_protocol`

**Files:** `src/state.rs` (add `Protocol`), `src/processor/init_protocol.rs`, `src/processor/mod.rs`, `src/instruction.rs` (`InitProtocol=9`), `src/error.rs`, `tests/common/mod.rs` (helper), `tests/init_protocol.rs`, `tests/state_layout.rs`.

**`Protocol` Pod account** (PDA `[b"protocol"]`, singleton; `AccountType::Protocol=7`): fields `account_type` + `_pad_hdr`, `admin:Pubkey` (the initializer; for later governance), `kass_mint:Pubkey`, `usdc_mint:Pubkey` (canonical mints — oracles must match these, so fee-burn can't be spoofed with a fake KASS mint), `fee_ema:u64` (fixed-point EMA accumulator of recent creation activity; 0 at genesis), `last_creation_unix:i64` (for EMA decay), `bump:u8`, padding. Pin its `LEN`/offsets.

**`init_protocol`** (route `Ix::InitProtocol`): one-time. Accounts: `[0] protocol PDA(w,uninit)`, `[1] admin(signer,w; pays rent)`, `[2] kass_mint`, `[3] usdc_mint`, `[4] system program`. Verify the protocol PDA address `[b"protocol"]`; reject if already initialized (`AlreadyInitialized`, append). `create_pda` it, stamp `account_type=Protocol`, `admin`, mints, `fee_ema=0`, `last_creation_unix=0`, `bump`.

**Tests:** init once → Protocol exists with fields; second init → fails; wrong PDA → fails.

Steps: write failing test → red → implement → green → commit `feat(protocol): protocol state account + init_protocol`.

**H0 DELTA (live state, done):** `AccountType::Protocol=7`; `Ix::InitProtocol=9`; `KassandraError::AlreadyInitialized=17`. New `Protocol` Pod (`Protocol::LEN=128`, pinned in `tests/state_layout.rs`): `account_type`@0, `_pad_hdr[7]`, `admin`@8, `kass_mint`@40, `usdc_mint`@72, `fee_ema:u64`@104 (0 at genesis, used by H2), `last_creation_unix:i64`@112 (EMA decay, H2), `bump`@120, `_pad[7]`. Protocol PDA seeds `[b"protocol"]` (singleton). Guard `load_protocol(ai, program_id)` added to `guards.rs` (owner+len+tag) for H1/H2. `init_protocol` (`src/processor/init_protocol.rs`): accounts `[0]protocol PDA(w,uninit),[1]admin(signer,w,pays rent),[2]kass_mint,[3]usdc_mint,[4]system`; verifies PDA `[b"protocol"]`, admin signer, system id, mints owned by token program; rejects re-init (`lamports!=0 || !data_is_empty()` → `AlreadyInitialized`); stamps fields with `fee_ema=0`/`last_creation_unix=0` (NO fee logic yet — that's H2). Harness: `TestCtx::protocol_pda()`, `init_protocol()`/`init_protocol_ix(protocol)`, `protocol(key)` accessor. Tests `tests/init_protocol.rs` (3, green): init-once, double-init→AlreadyInitialized, wrong-PDA→InvalidAccount.

---

## Task H1: `create_oracle` (no fee yet)

**Files:** `src/processor/create_oracle.rs`, `mod.rs`, `src/instruction.rs` (`CreateOracle=10`), `src/error.rs`, `config.rs` (proposal window const + promote `MAX_PROPOSERS`), `tests/create_oracle.rs`, `tests/state_layout.rs` if needed.

**Behavior (design §3):** creates an Oracle in `Phase::Proposal` with a future `deadline`; proposals are gated to open at the deadline (enforced in `propose`). Creates the program-controlled **stake vault** (KASS token account at PDA `[b"vault", oracle]`, authority = oracle PDA). NO fee yet (Task H2 adds it).

- **Add `config::PROPOSAL_WINDOW: i64`** (e.g. 3600) and **move `MAX_PROPOSERS` into `config.rs`** (update `finalize_oracle.rs` to use it).
- **Payload** after disc: `nonce:u64 LE` (oracle PDA seed) ++ `prompt_hash:[u8;32]` ++ `options_count:u8` (≥2, else `InvalidOption`/new err) ++ `deadline:i64 LE` (must be ≥ now, else `InvalidDeadline`) ++ `twap_window:i64 LE` (>0).
- **Accounts:** `[0] protocol(ro)` (to pin canonical mints), `[1] oracle PDA(w,uninit)`, `[2] stake_vault PDA(w,uninit)`, `[3] creator(signer,w; pays rent)`, `[4] kass_mint`, `[5] usdc_mint`, `[6] token program`, `[7] system program`. (Plus rent sysvar if needed.)
- **Validations:** `load_protocol`; `kass_mint == protocol.kass_mint`, `usdc_mint == protocol.usdc_mint`; oracle PDA address `[b"oracle", nonce_le]`; vault PDA address `[b"vault", oracle]`; creator signer; program ids; `options_count >= 2`; `deadline >= now`; `twap_window > 0`.
- **Create the stake vault:** `create_pda` the token account (space = `spl_token::state::Account::LEN` = 165, owner = token program, program-signed with vault seeds), then CPI `spl-token` `InitializeAccount3` with `mint = kass_mint`, `owner = oracle PDA`. (Verify the pinocchio-token init instruction name/shape against the installed crate; mirror the CPI style used in `open_challenge.rs`/`submit_fact.rs`.)
- **Init Oracle:** `account_type=Oracle`, creator, mints, `stake_vault` = the vault PDA, `deadline`, `phase=Proposal`, `phase_ends_at = deadline + PROPOSAL_WINDOW`, `twap_window`, `options_count`, all counts 0, `total_oracle_stake=0`, `bond_pool=0`, `dispute_bond_total=0`, `resolved_option=0`, `prompt_hash`, `bump`. Write back.
- NO fee burn (Task H2).

**Tests:** happy (Oracle created with all fields, vault is a KASS token account with authority = oracle PDA, balance 0, phase=Proposal); options_count<2 → fails; deadline in past → fails; mint mismatch vs protocol → fails; duplicate oracle (same nonce) → fails. Commit `feat(oracle): create_oracle + program-created stake vault`.

---

## Task H2: Dynamic EMA creation fee (KASS, burned)

**Files:** `src/processor/create_oracle.rs` (extend), `config.rs` (fee consts), `tests/create_oracle.rs` (fee tests).

**Behavior (design §8):** the creation fee is paid in KASS and **burned**; it is **proportional to an EMA of recent oracle creations** — 0 at genesis, rises with creation frequency, decays when idle.

- **Add `config` fee consts** with a documented fixed-point EMA model, e.g.: `FEE_EMA_HALFLIFE_SECS` (decay), `FEE_COEFF` (fee per unit EMA, in KASS base units), `FEE_EMA_INCREMENT` (EMA bump per creation). Use integer fixed-point (e.g. EMA scaled by 1e9) — document precisely; no floats.
- **In `create_oracle`** (protocol now writable): (1) decay `protocol.fee_ema` by the elapsed time since `last_creation_unix` (halving/exponential approximation in integer math); (2) `fee = FEE_COEFF * fee_ema_current` (≥0; 0 when ema 0 → genesis is free); (3) if `fee > 0`, CPI `spl-token` `Burn` of `fee` KASS from the creator's KASS token account (authority = creator, who signs) against `kass_mint`; (4) bump `protocol.fee_ema += FEE_EMA_INCREMENT` (checked) and set `last_creation_unix = now`; write protocol back.
- **Accounts add:** `[..] creator_kass_token_account(w)` (source of the burn) + `kass_mint(w)` must be the protocol's KASS mint. The protocol account becomes writable.
- Genesis: first-ever creation has `fee_ema == 0` → `fee == 0` → no burn.

**Tests:** genesis create → fee 0, no burn, ema bumped; several rapid creations → fee strictly increases (assert creator KASS burned + mint supply decreased); idle gap (warp) before a creation → fee lower than the no-gap case (decay); fee computed proportional to ema. Commit `feat(oracle): dynamic EMA creation fee burned in KASS`.

> Keep the EMA math simple and overflow-safe (u128 intermediates); the exact curve is governance-tunable — document the model in `config.rs`.

---

## Task H3: `propose` (proposer registration)

**Files:** `src/processor/propose.rs`, `mod.rs`, `src/instruction.rs` (`Propose=11`), `src/error.rs`, `tests/propose.rs`.

**Behavior (design §3):** after the deadline, anyone registers a proposal = a categorical value + KASS bond. One Proposer PDA per (oracle, authority). Enforces the `MAX_PROPOSERS` cap ON-CHAIN (closes final-review gating item #2).

- **Payload** after disc: `option:u8` (< oracle.options_count) ++ `bond:u64 LE` (>0, else `ZeroStake`/reuse).
- **Accounts:** `[0] oracle(w)`, `[1] proposer PDA(w,uninit)` `[b"proposer", oracle, authority]`, `[2] authority(signer,w; pays rent)`, `[3] authority KASS token account(w)`, `[4] stake_vault(w, == oracle.stake_vault)`, `[5] token program`, `[6] system program`.
- **Gating:** `load_oracle`; `require_phase(Proposal)`; `now >= oracle.deadline` (else `DeadlineNotReached`, append). Window logic (design §3 + the user's "wait for window end, or if nobody proposed, wait for first proposal"):
  - If `now < oracle.phase_ends_at`: normal — accept.
  - If `now >= oracle.phase_ends_at` AND `oracle.proposer_count == 0`: this is the seeding first-proposal after an empty window — accept AND extend `oracle.phase_ends_at = now + PROPOSAL_WINDOW` (re-open so others can still conflict).
  - If `now >= oracle.phase_ends_at` AND `oracle.proposer_count > 0`: window closed → `ProposalWindowClosed` (append); caller must `finalize_proposals`.
- **Cap:** reject if `oracle.proposer_count >= MAX_PROPOSERS` (`TooManyProposers`, append) — the on-chain liveness guarantee for the one-shot `finalize_oracle`.
- **Validations:** option range; bond>0; proposer PDA address; duplicate proposal (PDA already exists) → `DuplicateProposer` (append); authority signer; vault == oracle.stake_vault; program ids.
- **Logic:** Transfer `bond` KASS authority→stake_vault (authority signs). `create_pda` the Proposer; init `account_type=Proposer`, oracle, authority, bond, `original_option=option`, **`claim_option=CLAIM_OPTION_NONE`**, flags 0, bump, `slashed_amount=0`. `oracle.proposer_count += 1` (checked), `oracle.surviving_count += 1` (checked), `oracle.total_oracle_stake += bond` (checked). Write back.

**Tests:** before deadline → `DeadlineNotReached`; happy register (Proposer created, claim_option==CLAIM_OPTION_NONE, vault += bond, counts++); second by same authority → `DuplicateProposer`; option out of range → fails; bond 0 → fails; `MAX_PROPOSERS+1` th → `TooManyProposers`; empty-window seeding (warp past phase_ends_at with 0 proposers → first propose accepted + window extended); window-closed-with-proposers → `ProposalWindowClosed`. Commit `feat(propose): proposer registration + bond + MAX_PROPOSERS cap`.

---

## Task H4: `finalize_proposals` (resolve uncontested | open dispute)

**Files:** `src/processor/finalize_proposals.rs`, `mod.rs`, `src/instruction.rs` (`FinalizeProposals=12`), `tests/finalize_proposals.rs`.

**Behavior (design §3, §7):** at proposal-window end, either resolve (all agree) or open the dispute (conflict) — handing off to the existing dispute core.

- **Gating:** `load_oracle`; `require_phase(Proposal)`; `now >= oracle.phase_ends_at` (else `WindowNotElapsed`); `oracle.proposer_count >= 1` (else there is nothing to finalize — stays open for the first proposal; `IncompleteFactSet`/new `NoProposals`).
- **Accounts:** `[0] oracle(w)` + ALL proposer accounts (read-only tail; exact `proposer_count`, distinct, owner+type==Proposer+belongs-to-oracle — mirror `finalize_oracle`'s full-set proof). Cap by `MAX_PROPOSERS`.
- **Decision:** collect each proposer's `original_option`.
  - **All equal** → **Resolved**: `oracle.resolved_option = that option`, `set_phase(Resolved)`. (Bonds are returned via the deferred settlement layer; counter-only here — no token CPI. Document.)
  - **≥2 distinct** → **open dispute**: set `oracle.dispute_bond_total = oracle.total_oracle_stake` (Σ bonds, the fixed fact-quorum denominator the dispute core expects), `set_phase(FactProposal)`, `oracle.phase_ends_at = now + PHASE_WINDOW`. This is the seam into the already-built dispute core (`submit_fact` onward).
- No token CPI; idempotent (terminal/next-phase makes re-entry fail `require_phase(Proposal)`).

**Tests:** all-agree (3 proposers same option) → Resolved + resolved_option, vault untouched; conflict (2 distinct) → phase FactProposal, dispute_bond_total == total_oracle_stake; window still open → WindowNotElapsed; wrong phase → WrongPhase; subset/ count mismatch → InvalidAccount; idempotency (second call) → WrongPhase. Commit `feat(propose): finalize_proposals — resolve uncontested or open dispute`.

---

## Task H5: End-to-end lifecycle test + harness real-flow builder

**Files:** `tests/common/mod.rs` (add a real-flow builder), `tests/lifecycle_e2e.rs`.

- Add `TestCtx::create_real_oracle(...)` and `propose_real(...)` helpers that drive `init_protocol`/`create_oracle`/`propose`/`finalize_proposals` via real instructions, returning the same handle shape as `seed_disputed_oracle` (so dispute-core paths can be exercised from the genuine entry point).
- **E2E happy:** init_protocol → create_oracle → propose×3 (same option) → warp past window → finalize_proposals → assert Resolved + resolved_option.
- **E2E dispute hand-off:** create_oracle → propose×2 (distinct options) → warp → finalize_proposals (→ FactProposal, dispute_bond_total set) → then drive the EXISTING dispute core (submit_fact → advance_phase → vote_fact → finalize_facts → submit_ai_claim → finalize_ai_claims → finalize_oracle) to a terminal state. Asserts the real entry point composes with the dispute core end-to-end.
- Assert KASS conservation across create+propose: `stake_vault balance == total_oracle_stake == Σ bonds`, and the burned fee reduced creator balance + mint supply.

Commit `test(e2e): full lifecycle from create_oracle through terminal state`.

---

## Task H6: Proposal-phase invariant fuzz arm

**Files:** `tests/invariants.rs` (extend).

Add a proptest arm that generates N (≤ MAX_PROPOSERS) proposers with random options and bonds, drives create→propose→finalize_proposals, and asserts: termination (Resolved iff all options equal, else FactProposal with dispute_bond_total == Σ bonds); conservation (`stake_vault == total_oracle_stake == Σ bonds`); the cap (proposing beyond MAX_PROPOSERS fails, never bricks). Keep case count modest. Commit `test(invariants): proposal-phase termination + conservation arm`.

---

## Out of scope (later milestones)

- Physical settlement: per-staker bond return/reward, MetaDAO `redeem_tokens`, bond-pool distribution, emissions mint, `close_ai_claim`. (Bonds/fees that MUST move here — the burned fee, bond escrow — are done; returns/rewards are deferred.)
- KASS bootstrapping (participation emissions); the runner, SDK, app; MetaDAO futarchy governance for dead-ends.

## Execution note
After each task: `just build` → `cargo test -p kassandra-program` → clippy/fmt, confirm green, commit. Never proceed on a red bar. Keep the plan's deltas current (append a live-state line per task) and re-pin `state_layout.rs` on any layout change.

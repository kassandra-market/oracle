# Kassandra KASS Futarchy Governance — Design + Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the Kassandra protocol **governed by a MetaDAO v0.6 futarchy KASS DAO**. Governance can (a) update protocol config and (b) resolve `InvalidDeadend` oracles. The DAO's KASS/USDC spot market (Meteora DAMM v2) exposes a **TWAP price** (`kass_price`) that the *next* milestone (challenge-market rework) will consume. This is the first step of the dependency-first roadmap: **KASS futarchy → challenge-market rework → staker settlement** (see `2026-06-29-kassandra-settlement-economics.md`).

**Architecture:** Extends the existing Pinocchio program (no Anchor). Reuses **MetaDAO futarchy v0.6** (its governance program + v0.6 conditional vault + **Meteora DAMM v2** AMM) via dumped fixtures + hand-built CPI / LiteSVM, the same way the dispute core reused the v0.4 vault/AMM. NOTE: v0.6 is a **separate, newer stack** than the dispute core's pinned v0.4 vault/AMM (v0.5+ migrated the AMM to Meteora DAMM v2) — this milestone integrates that newer stack.

**Tech Stack:** Rust, `pinocchio` 0.8, `bytemuck`, `litesvm`, `solana-sdk` (test-only), `spl-token`, MetaDAO futarchy **v0.6** + v0.6 conditional vault + **Meteora DAMM v2**.

**Source of truth:** design `docs/plans/2026-06-29-kassandra-design.md`; the dispute-core deltas in `2026-06-29-kassandra-dispute-core.md` ("Implementation deltas (live state)" — authoritative live types/sizes/guards/seeds/errors/Ix); the happy-path milestone (now merged: `init_protocol`/`Protocol`, `create_oracle`, `propose`, `finalize_proposals`, EMA fee, emission consts pending settlement). FOLLOW THE LIVE STATE.

---

## Validated design (brainstormed)

### Governance seam
- `Protocol` gains `dao_authority: Pubkey` and `kass_dao: Pubkey`. Two privileged instructions, each **gated to require `dao_authority` as signer**: `set_config`, `resolve_deadend`. A *passed* v0.6 proposal CPIs into them — no privileged key; governance-by-market end to end.
- **F0 FINDING #1 — `dao_authority` is a Squads v4 multisig VAULT PDA**, not a futarchy PDA. MetaDAO v0.6 futarchy executes passed proposals through a Squads v4 multisig. Seeds (under `SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf`): multisig `[b"multisig", b"multisig", dao]`, vault `[b"multisig", multisig, b"vault", [0]]`. F1 stores the resolved vault key in `Protocol.dao_authority` at bootstrap (the `Dao` account has variable-offset fields, so store, don't re-derive). `kass_dao` = the futarchy `Dao` account (whose embedded spot AMM is the price source — see Price oracle).

### Governable config — snapshot-at-creation
- Governable params live **mutable on `Protocol`** (edited by `set_config`) and are **snapshotted onto each `Oracle` at `create_oracle`**. Downstream processors read them from the `Oracle` they already load (no new account threading). New oracles pick up new config; in-flight oracles keep their snapshot (a mid-dispute governance change cannot move the goalposts).
  - **Snapshot onto `Oracle`** (per-oracle behavioral): `THRESHOLD_NUM/DEN`, `MARKET_THRESHOLD_NUM/DEN`, `FLIP_SLASH_NUM/DEN`, `FACT_VOTE_SLASH_NUM/DEN` (settlement-era; reserve the field now if cheap), reward-bucket weights `PW/FW` (settlement-era; reserve), window durations `PHASE_WINDOW`/`PROPOSAL_WINDOW`. (`twap_window` already per-oracle.)
  - **Global on `Protocol`** (monetary, used by `create_oracle` which loads `Protocol`): fee-EMA params, emission rate, `TOTAL_SUPPLY_CAP`.
  - **Fixed `const` (NOT governable):** `MAX_PROPOSERS` (tx-size/liveness constraint), anything affecting account layout.
- `set_config` updates only the `Protocol`-resident governable fields, bounds-checked (denominators > 0, fractions ≤ 1, windows > 0); never retroactively touches existing oracles.

### Dead-end resolution
- `resolve_deadend(oracle, option)` gated to `dao_authority`: `require_phase(InvalidDeadend)`, `option < options_count` → set `Phase::Resolved` + `resolved_option`. The **economic settlement** of a governance-resolved dead-end is deferred to the settlement milestone (likely stakes returned, no rewards) — this milestone only sets the terminal outcome.

### Price oracle
- **F0 FINDING #2 — the price source is the futarchy program's embedded spot `TwapOracle` (`Dao.amm`), NOT Meteora.** Meteora cp-amm (DAMM v2) exposes only an instantaneous `sqrt_price`, no TWAP. F0 implemented + validated `metadao_v06::futarchy_spot_twap` over the futarchy spot oracle (offsets aggregator u128@9, last_updated i64@25, created_at i64@33, start_delay u32@105; `twap = aggregator/(last_updated−(created_at+start_delay))`).
- `kass_price` reads that **futarchy spot TWAP** from the canonical KASS DAO's spot AMM, asserting the passed account == `Protocol.kass_dao`'s spot-AMM (governance-blessed; prevents attacker substitution). Ships as a validated primitive with **no on-chain consumer yet** (the challenge-market rework consumes it next milestone) — expected, not dead code.

### Bootstrapping
- An init/setup step records `dao_authority` (the Squads vault PDA) + `kass_dao` (the futarchy `Dao` account) in `Protocol` and confirms the KASS mint authority is the program PDA. (Emission mint authority stays the program PDA per the settlement design; the DAO governs the emission *rate*, not direct minting.)

---

## Conventions (unchanged)
- TDD; `just build` (cargo build-sbf) BEFORE `cargo test`; clippy + fmt clean before commit. Commit trailer `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`, git author `Kassandra <hexadecifish@gmail.com>`. Append-only `Ix`/`KassandraError` discriminants. Re-pin `tests/state_layout.rs` on any layout change. rust-analyzer false positives — rely on real cargo runs.

## Live-state entry points (post happy-path merge)
- `Protocol` (LEN 128): account_type, admin, kass_mint@40, usdc_mint@72, fee_ema:u64@104, last_creation_unix:i64@112, bump. `load_protocol` (owner+len+tag+ PDA-address pin) exists. PDA `[b"protocol"]`; create-or-adopt init (Allocate+Assign) tolerant of pre-funding.
- `Oracle` (LEN 232) with `resolved_option`@197, `open_challenge_count`@198, phase/windows/counts/`dispute_bond_total`. `Phase{...Resolved=7, InvalidDeadend=8}`.
- `Ix` up to `FinalizeProposals=12`. `KassandraError` up to `NoProposals=24`. Guards: `assert_*`, `load_oracle/fact/proposer/ai_claim/protocol`, `create_pda`. `config.rs` consts incl. `MAX_PROPOSERS=60`, windows, thresholds, fee/flip params.
- Existing MetaDAO v0.4 integration in `src/cpi/metadao.rs` + `tests/fixtures/` (do NOT disturb; v0.6 is additive — consider `src/cpi/metadao_v06.rs` + `tests/fixtures/` additions).

---

## Tasks

### F0 — MetaDAO v0.6 + Meteora DAMM v2 recon + CPI groundwork (HIGH RISK)
Mirror Task 9's rigor. STEP 0: verify mainnet reachability; resolve **authoritative latest** program IDs for the v0.6 futarchy/governance program, the v0.6 conditional vault, and **Meteora DAMM v2** from MetaDAO's official source (`declare_id!` + on-chain), REJECTING web-guessed IDs. If blocked (no mainnet / can't resolve), STOP and report. Then: `scripts/fetch-metadao-v06.sh` dumps the binaries to `tests/fixtures/` (sha-pin in the header). `src/cpi/metadao_v06.rs`: program IDs, discriminators (sha256("global:<name>")[..8]), PDA seeds, no-alloc arg encoders, invoke wrappers for the instructions we need. Document the REAL layouts: the proposal account, the DAO account, the **DAO execution-authority PDA**, and the **Meteora DAMM v2 pool + TWAP** field offsets (determined from real source/binary, NOT guessed). `tests/metadao_v06_cpi.rs`: load all v0.6 + Meteora fixtures into LiteSVM without panic; validate a minimal CPI (e.g. read a Meteora pool's TWAP, or initialize a DAO) against the real binary. Report the resolved IDs/versions/sources, the layouts, and what was validated vs deferred.

### F1 — Protocol governance state + DAO linkage
**Per F0 finding #1:** `dao_authority` = the **Squads v4 multisig vault PDA**; `kass_dao` = the futarchy `Dao` account. Add to `Protocol` (re-pin layout): `dao_authority: Pubkey`, `kass_dao: Pubkey`, and the global governable params (fee-EMA params, emission rate, `TOTAL_SUPPLY_CAP` — reserve fields even if settlement sets their semantics later). Add a `set_governance` instruction (or extend `init_protocol`) that records the resolved `dao_authority` (Squads vault) + `kass_dao` and confirms/sets the KASS mint authority = program PDA. STORE the vault key (don't re-derive from the variable-offset `Dao` bytes). Tests: setup records the fields; values pinned. (Bootstrapping/trust note: in v1 the `admin` from `init_protocol` sets these once, handing control to the DAO — document the trust assumption, and whether `set_governance` is one-shot or itself dao-gated after handoff.)

### F2 — Config-as-state refactor (largest churn)
Add the snapshot fields to `Oracle` (re-pin layout). `create_oracle` snapshots the current global governable per-oracle params from `Protocol`/config into the `Oracle`. Switch every processor that reads a snapshotted param from `config::X` to `oracle.x` (finalize_facts, vote_fact, submit_fact, finalize_ai_claims, settle_challenge, finalize_oracle, advance_phase, propose, finalize_proposals — wherever a snapshotted const is used). Keep `MAX_PROPOSERS` + layout sizes `const`. All existing tests must still pass (behavior identical when config == defaults). Re-pin layouts; update the conservation/invariant assumptions only if needed.

### F3 — `set_config` (Ix append; gated)
Gated to `Protocol.dao_authority` (signer). Updates the `Protocol`-resident global governable fields, bounds-checked. Does NOT touch existing oracles. Tests: dao_authority can set; non-authority → Unauthorized; out-of-bounds → error; a subsequently-created oracle snapshots the new values.

### F4 — `resolve_deadend` (Ix append; gated)
Gated to `dao_authority`. `require_phase(InvalidDeadend)`, `option < oracle.options_count` → `set_phase(Resolved)` + `resolved_option`. Document that economic settlement is deferred. Tests: dao_authority resolves a dead-end → Resolved+option; non-authority → Unauthorized; wrong phase → WrongPhase; option out of range → error.

### F5 — `kass_price` (futarchy spot `TwapOracle` read)
**Per F0 finding #2:** read the KASS/USDC TWAP from the **futarchy program's embedded spot `TwapOracle`** (`Dao.amm`), NOT Meteora (cp-amm has no TWAP). Reuse `metadao_v06::futarchy_spot_twap` (already implemented + validated in F0). The read asserts the passed account corresponds to `Protocol.kass_dao`'s spot AMM (governance-blessed; reject substitution). Test: a hand-built/real spot-oracle blob yields a sane TWAP; wrong account → rejected; zero-aggregator/no-observation → handled. No on-chain consumer yet (next milestone). (Meteora `sqrt_price` is NOT the price source; only relevant later if conditional-market liquidity needs it.)

### F6 — v0.6 futarchy proposal→execute integration (+ seam fallback)
Drive a governance proposal carrying a `set_config` (or `resolve_deadend`) CPI through the v0.6 futarchy: create proposal → conditional pass/fail KASS markets (v0.6 vault + Meteora DAMM v2) → trade to a pass verdict → execute → assert the config changed / oracle resolved, with execution signed by `dao_authority`. **Fallback (document honestly):** if driving the full v0.6+Meteora flow in LiteSVM is impractical, test the **seam** directly (privileged instructions accept the real `dao_authority` PDA as signer and reject others; CPI shapes validated against the dumped binaries) and integration-test the v0.6 execution path as far as LiteSVM allows.

---

## Out of scope (later milestones)
- Challenge-market rework (bond-as-AMM-liquidity + directional fees) consuming `kass_price` — NEXT.
- Staker settlement (returns/rewards/emissions/closure) — see the settlement-economics note.
- Full DAO treasury spending; migrating the dispute-core challenge markets from v0.4 to v0.6/Meteora.

## Execution note
After each task: `just build` → `cargo test -p kassandra-program` → clippy/fmt, confirm green, commit. Never proceed on a red bar. Keep an "Implementation deltas (F0–F6)" running log appended here. F0 is the highest risk (resolve real v0.6/Meteora IDs + layouts first); F2 is the largest churn; F6 is the hardest test (with the documented seam fallback).

---

## Implementation deltas (F0–F6) — live state

### F0 — MetaDAO v0.6 + Meteora DAMM v2 recon + CPI groundwork (DONE 2026-06-29)

**Environment.** solana-cli 3.1.7 (Agave), cargo 1.94.1; mainnet-beta reachable (`solana cluster-version -u m` → 4.0.3). Step 0 UNBLOCKED.

**Resolved program IDs (authoritatively sourced, verified on-chain mainnet-beta 2026-06-29):**

| program | id | version / source |
|---|---|---|
| futarchy (v0.6 governance, replaces `autocrat`) | `FUTARELBfJfQ8RDGhg1wdhddq1odMAJUePHFuBYfUxKq` | v0.6.0 — `declare_id!` in `metaDAOproject/programs` `programs/futarchy/src/lib.rs` @ tag `v0.6.0` + `Anchor.toml [programs.localnet].futarchy`; Cargo `version = "0.6.0"`. On-chain: slot 423005106, 1243500 bytes, upgrade auth `6awyHMsh…` (same MetaDAO authority as v0.4). |
| conditional_vault (v0.6 line) | `VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg` | UNCHANGED from v0.4 — `declare_id!` @ `v0.6.0` identical to v0.4. On-chain: slot 399213625, 424952 bytes. Split/merge/redeem + init/resolve_question discriminators are byte-for-byte the v0.4 ones (re-verified). |
| Meteora DAMM v2 (cp-amm) | `cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG` | `declare_id!` in `MeteoraAg/damm-v2` `programs/cp-amm/src/lib.rs` @ main; cross-confirmed as mainnet deployment in `MeteoraAg/damm-v2-sdk`. MetaDAO's `programs/damm_v2_cpi` shim (v0.6 tree) `declare_id!`s the same address. On-chain: slot 428936648, 2174352 bytes, upgrade auth `JADaUV8k…` (Meteora's). |
| Squads v4 (DAO execution-authority host) | `SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf` | `declare_id!` in `Squads-Protocol/v4` @ rev `6d5235da621a2e9b7379ea358e48760e981053be` (the rev `futarchy/Cargo.toml` pins). Seeds from `state/seeds.rs`. |

**Two recon findings that REVISE the design:**
1. **DAO execution authority = a Squads v4 multisig vault, NOT a futarchy PDA.** `initialize_dao` CPIs into Squads to create a multisig whose `create_key` is the `Dao` PDA; a passed proposal carries a `squads_proposal` and executes through the Squads **vault** PDA. So `Protocol.dao_authority` (signer of `set_config`/`resolve_deadend`, F1/F3/F4/F6) is the Squads vault PDA. Seeds (under `SQDS4…`): multisig `[b"multisig", b"multisig", dao]`; vault `[b"multisig", multisig, b"vault", [0u8]]`. DAO PDA (under futarchy) `[b"dao", dao_creator, nonce_le8]`. Proposal PDA `[b"proposal", squads_proposal]`.
2. **Meteora cp-amm has NO TWAP oracle.** Its zero-copy `Pool` exposes only an INSTANTANEOUS `sqrt_price: u128` (Q64.64) + cumulative *fee* accumulators — no cumulative price observation. The manipulation-resistant KASS/USDC TWAP the design's `kass_price` (F5) needs is the futarchy program's **embedded** `FutarchyAmm` spot-pool `TwapOracle` (`Dao.amm`), not Meteora. **F5 should read the futarchy spot TWAP**, not a Meteora pool. The `kass_usdc_pool`/`kass_price` design language should be re-read as "the futarchy DAO's embedded spot market".

**Layouts documented (from v0.6 / Meteora / Squads source, in `src/cpi/metadao_v06.rs`):**
- **Futarchy spot TWAP (the real F5 source)** — the spot `Pool` is the first payload element of both `PoolState` variants, so its offsets are FIXED regardless of variant: in a `Dao` account, byte 8 = PoolState tag (0=Spot,1=Futarchy), byte 9 = spot `Pool`/`TwapOracle` start: aggregator u128@9, last_updated_timestamp i64@25, created_at_timestamp i64@33, last_price u128@41, last_observation u128@57, max_observation_change_per_update u128@73, initial_observation u128@89, start_delay_seconds u32@105. `get_twap = aggregator / (last_updated_ts − (created_at_ts + start_delay_seconds))`, price scaled ×1e12. Implemented + validated as `futarchy_spot_twap`.
- **`Dao` account** — field order documented; CAUTION `amm.state` is a Borsh enum (Spot=1+132, Futarchy=1+3×132 bytes) so ALL fields after it (`squads_multisig_vault`, mints, params…) are at VARIABLE offsets. F1 must store the vault key in `Protocol` at bootstrap rather than re-derive from `Dao` bytes.
- **`Proposal` account** — `number` u32@8, `proposer` Pubkey@12, `timestamp_enqueued` i64@44, `state` enum tag@52 (0=Draft{+8},1=Pending,2=Passed,3=Failed); fields after `state` are variable-offset.
- **Meteora cp-amm `Pool`** — full field ORDER documented; the load-bearing field is `sqrt_price: u128`. Exact byte offset NOT hand-pinned (nested zero-copy `PoolFeesStruct` C-padding is error-prone by hand); deferred to F5 to pin against a LIVE pool dump / published IDL IF F5 ends up reading Meteora at all (it likely won't — finding #2).

**Discriminators (computed `sha256("global:<name>")[..8]` / `account:<Type>`, in the module):** futarchy initialize_dao/initialize_proposal/launch_proposal/finalize_proposal/update_dao/spot_swap/conditional_swap; account Dao/Proposal; Meteora initialize_pool/swap/add_liquidity + account Pool. Vault discs reused from `cpi::metadao` (unchanged).

**Files (all additive; v0.4 integration untouched):**
- `scripts/fetch-metadao-v06.sh` — documents IDs/versions/sources/slots + sha256 pins; dumps `metadao_futarchy_v06.so` (1243500 B), `metadao_conditional_vault_v06.so` (424952 B), `meteora_damm_v2.so` (2174352 B) into `programs/kassandra/tests/fixtures/`.
- `programs/kassandra/src/cpi/metadao_v06.rs` (+ `pub mod metadao_v06;`) — IDs, discriminators, PDA seed builders + derivation, the `futarchy_spot_twap` reader, no-alloc arg encoders (`initialize_dao_data_no_limit` for the `None` spending-limit case; complex/variable-length params STUBBED + documented), invoke wrappers.
- `programs/kassandra/tests/metadao_v06_cpi.rs` — 5 tests, all green.

**Validated vs deferred:**
- VALIDATED against real binaries: all 3 fixtures load + executable; a FULL conditional_vault split (initialize_question → initialize_conditional_vault → split_tokens) against the v0.6-dumped vault binary; the COMPUTED futarchy `initialize_dao` discriminator dispatches into the real futarchy binary's `InitializeDao` handler (asserted via the `Instruction: InitializeDao` program log) while a bogus discriminator does not; the futarchy spot-TWAP offset map + `get_twap` math against a hand-built `Dao` blob.
- DEFERRED to F5/F6: a full `initialize_dao` success (needs Squads v4 program + mints loaded), driving a proposal to pass/execute, and reading a live Meteora cp-amm `sqrt_price` (cp-amm has no TWAP — see finding #2).

Build: `just build` (SBF) clean. Tests: `cargo test -p kassandra-program` all pass (incl. the 5 new). `cargo clippy --all-targets` clean; `cargo fmt` applied.

### F1 — Protocol governance state + DAO linkage (DONE 2026-06-29)

**`Protocol` re-pinned — LEN 128 → 240** (`state.rs`, re-pinned in `tests/state_layout.rs`). Existing offsets unchanged; the old `bump@120 + _pad[7]` tail became `bump@120`, the new fields, and the governable-param block. Pod-valid (align 8, no implicit padding). Offsets:

| field | off | field | off |
|---|---|---|---|
| account_type | 0 | dao_authority (Pubkey) | 128 |
| admin | 8 | kass_dao (Pubkey) | 160 |
| kass_mint | 40 | emission_num (u64) | 192 |
| usdc_mint | 72 | emission_den (u64) | 200 |
| fee_ema (u64) | 104 | total_supply_cap (u64) | 208 |
| last_creation_unix (i64) | 112 | fee_ema_halflife (i64) | 216 |
| bump (u8) | 120 | fee_per_ema_unit (u64) | 224 |
| governance_set (u8) | 121 | fee_ema_increment (u64) | 232 |
| _pad[6] | 122 | | |

**Monetary param defaults (no behavior change).** `init_protocol` sets `emission_num=0`, `emission_den=1` (denominator never zero), `total_supply_cap=0` (all settlement-era, reserved), and the fee-EMA mirror to the current `config.rs` consts: `fee_ema_halflife = FEE_EMA_HALFLIFE_SECS (86400)`, `fee_per_ema_unit = FEE_PER_EMA_UNIT (1e9)`, `fee_ema_increment = FEE_EMA_INCREMENT (1e9)`. `create_oracle` STILL reads the consts (config-as-state wiring is F2); F1 only adds + defaults the fields. `dao_authority`/`kass_dao` zeroed, `governance_set=0`.

**`Ix::SetGovernance = 13`** (appended; `from_u8` arm added; dispatched in `processor/mod.rs`). New processor `processor/set_governance.rs`.
- Accounts: `[0] protocol(w)`, `[1] authority(signer)`. Payload: `dao_authority:[u8;32] ++ kass_dao:[u8;32]` (64 bytes), both validated non-zero (else `InvalidAccount`).
- **Trust model (implemented + tested): admin-sets-once → DAO rotates.** While `governance_set==0`, only `Protocol.admin` may call it (else `Unauthorized`) — the one-time bootstrap handoff. Once `governance_set==1`, only the current `Protocol.dao_authority` may rotate it (else new `KassandraError::GovernanceAlreadySet = 25`); the old admin is rejected. The DAO can rotate its own linkage but control never returns to the admin.
- Stores the passed pubkeys verbatim (F0 finding #1: `dao_authority` = Squads v4 vault PDA, `kass_dao` = futarchy `Dao`; real setup is F6).

**Mint-authority PDA seed** defined as `config::MINT_AUTHORITY_SEED = b"mint_authority"` (PDA `[b"mint_authority"]`, program = `crate::ID`). F1 only DEFINES it; the binding `kass_mint.mint_authority == mint_authority_pda` is asserted at first emission (settlement), since verifying it requires threading the mint account (and the test-harness KASS mint authority is the payer, not the PDA).

**Error appended:** `KassandraError::GovernanceAlreadySet = 25`.

**Tests** (`tests/governance_setup.rs`, 4, all green): admin handoff records linkage + monetary defaults; non-admin → `Unauthorized`; one-shot (post-handoff admin → `GovernanceAlreadySet`, dao_authority rotates OK); zero dao_authority/kass_dao → `InvalidAccount`. Harness (`tests/common/mod.rs`): `stand_in_governance(tag)`, `set_governance`/`set_governance_ix` helpers; the existing `protocol` accessor exposes the new fields. All prior tests (incl. 4 init_protocol, layout pin) still pass.

Build: `just build` (SBF) clean. Tests: `cargo test -p kassandra-program` all pass. `cargo clippy --all-targets` clean; `cargo fmt` applied.

### F2 — Config-as-state refactor (DONE 2026-06-29)

Behavior-preserving: the snapshot defaults == the current `config.rs` consts, so the full suite passes UNCHANGED. The consts stay in `config.rs` as the DEFAULT source (`init_protocol` reads them); `MAX_PROPOSERS` + layout-size consts stay `const`.

**`Protocol` re-pinned — LEN 240 → 336** (`state.rs`, re-pinned in `tests/state_layout.rs`). Appended 12 governable behavioral params after `fee_ema_increment@232` (all u64/i64, align 8, no padding). New offsets:

| field | off | field | off |
|---|---|---|---|
| threshold_num (u64) | 240 | phase_window (i64) | 288 |
| threshold_den (u64) | 248 | proposal_window (i64) | 296 |
| market_threshold_num (u64) | 256 | fact_vote_slash_num (u64) RES | 304 |
| market_threshold_den (u64) | 264 | fact_vote_slash_den (u64) RES | 312 |
| flip_slash_num (u64) | 272 | reward_proposer_weight (u64) RES | 320 |
| flip_slash_den (u64) | 280 | reward_fact_weight (u64) RES | 328 |

**`Oracle` re-pinned — LEN 232 → 328** (`state.rs`, re-pinned in `tests/state_layout.rs`). Same 12 fields appended after `prompt_hash@200` (which ends at 232). New offsets:

| field | off | field | off |
|---|---|---|---|
| threshold_num (u64) | 232 | phase_window (i64) | 280 |
| threshold_den (u64) | 240 | proposal_window (i64) | 288 |
| market_threshold_num (u64) | 248 | fact_vote_slash_num (u64) RES | 296 |
| market_threshold_den (u64) | 256 | fact_vote_slash_den (u64) RES | 304 |
| flip_slash_num (u64) | 264 | reward_proposer_weight (u64) RES | 312 |
| flip_slash_den (u64) | 272 | reward_fact_weight (u64) RES | 320 |

`MARKET_THRESHOLD_*` are `u128` consts but stored as `u64` on both structs (values fit; widened back to u128 on read in `settle_challenge`). Both structs stay Pod-valid (no implicit padding).

**`init_protocol` defaults** all 12 Protocol fields to the consts: `threshold_num/den = THRESHOLD_NUM/DEN`, `market_threshold_num/den = MARKET_THRESHOLD_NUM/DEN as u64`, `flip_slash_num/den = FLIP_SLASH_NUM/DEN`, `phase_window = PHASE_WINDOW`, `proposal_window = PROPOSAL_WINDOW`. Reserved fields: `fact_vote_slash_num=0, fact_vote_slash_den=1` (divisor never zero), `reward_proposer_weight=0, reward_fact_weight=0`.

**`create_oracle` snapshots** all 12 params from the loaded `Protocol` into the new `Oracle` fields at init, and now seeds `phase_ends_at = deadline + protocol.proposal_window` (was `+ PROPOSAL_WINDOW`).

**Processor migrations (`config::X` → `oracle.x`):**
- `finalize_facts`: `is_agreed` takes `oracle.threshold_num/den` (fact quorum); AiClaim window `now + oracle.phase_window`.
- `advance_phase`: FactVoting window `now + oracle.phase_window`.
- `settle_challenge`: slash-trigger margin `oracle.market_threshold_num/den` (widened to u128).
- `finalize_ai_claims`: flip slash `oracle.flip_slash_num/den`; Challenge window `now + oracle.phase_window`.
- `finalize_proposals`: dispute-handoff window `now + oracle.phase_window`.
- `propose`: empty-window seeding `now + oracle.proposal_window`.
- `create_oracle`: `deadline + protocol.proposal_window` (snapshot source).

**Reserved-only (NO active reader, settlement-era):** `fact_vote_slash_num/den`, `reward_proposer_weight/fact_weight` on both structs — snapshotted by `create_oracle` but not wired.

**Test harness:** `tests/common/mod.rs::seed_disputed_oracle` (which fabricates oracles directly, bypassing `create_oracle`) now sets the 8 active snapshot fields to the config consts, so a fabricated oracle behaves identically to a real one (else zeroed denominators would divide-by-zero / change behavior). Grep confirms no remaining `config::{THRESHOLD,MARKET_THRESHOLD,FLIP_SLASH,PHASE_WINDOW,PROPOSAL_WINDOW}` reads in processor active paths (only doc-comments + the `init_protocol` default source remain).

Build: `just build` (SBF) clean. Tests: `cargo test -p kassandra-program` all pass UNCHANGED. `cargo clippy --all-targets` clean; `cargo fmt` applied.

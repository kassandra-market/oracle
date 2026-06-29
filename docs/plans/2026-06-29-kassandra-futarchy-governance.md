# Kassandra KASS Futarchy Governance — Design + Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the Kassandra protocol **governed by a MetaDAO v0.6 futarchy KASS DAO**. Governance can (a) update protocol config and (b) resolve `InvalidDeadend` oracles. The DAO's KASS/USDC spot market (Meteora DAMM v2) exposes a **TWAP price** (`kass_price`) that the *next* milestone (challenge-market rework) will consume. This is the first step of the dependency-first roadmap: **KASS futarchy → challenge-market rework → staker settlement** (see `2026-06-29-kassandra-settlement-economics.md`).

**Architecture:** Extends the existing Pinocchio program (no Anchor). Reuses **MetaDAO futarchy v0.6** (its governance program + v0.6 conditional vault + **Meteora DAMM v2** AMM) via dumped fixtures + hand-built CPI / LiteSVM, the same way the dispute core reused the v0.4 vault/AMM. NOTE: v0.6 is a **separate, newer stack** than the dispute core's pinned v0.4 vault/AMM (v0.5+ migrated the AMM to Meteora DAMM v2) — this milestone integrates that newer stack.

**Tech Stack:** Rust, `pinocchio` 0.8, `bytemuck`, `litesvm`, `solana-sdk` (test-only), `spl-token`, MetaDAO futarchy **v0.6** + v0.6 conditional vault + **Meteora DAMM v2**.

**Source of truth:** design `docs/plans/2026-06-29-kassandra-design.md`; the dispute-core deltas in `2026-06-29-kassandra-dispute-core.md` ("Implementation deltas (live state)" — authoritative live types/sizes/guards/seeds/errors/Ix); the happy-path milestone (now merged: `init_protocol`/`Protocol`, `create_oracle`, `propose`, `finalize_proposals`, EMA fee, emission consts pending settlement). FOLLOW THE LIVE STATE.

---

## Validated design (brainstormed)

### Governance seam
- `Protocol` gains `dao_authority: Pubkey` (the v0.6 DAO execution-authority PDA) and `kass_usdc_pool: Pubkey` (the canonical Meteora DAMM v2 KASS/USDC pool). Two privileged instructions, each **gated to require `dao_authority` as signer**: `set_config`, `resolve_deadend`. A *passed* v0.6 proposal CPIs into them — no privileged key; governance-by-market end to end.

### Governable config — snapshot-at-creation
- Governable params live **mutable on `Protocol`** (edited by `set_config`) and are **snapshotted onto each `Oracle` at `create_oracle`**. Downstream processors read them from the `Oracle` they already load (no new account threading). New oracles pick up new config; in-flight oracles keep their snapshot (a mid-dispute governance change cannot move the goalposts).
  - **Snapshot onto `Oracle`** (per-oracle behavioral): `THRESHOLD_NUM/DEN`, `MARKET_THRESHOLD_NUM/DEN`, `FLIP_SLASH_NUM/DEN`, `FACT_VOTE_SLASH_NUM/DEN` (settlement-era; reserve the field now if cheap), reward-bucket weights `PW/FW` (settlement-era; reserve), window durations `PHASE_WINDOW`/`PROPOSAL_WINDOW`. (`twap_window` already per-oracle.)
  - **Global on `Protocol`** (monetary, used by `create_oracle` which loads `Protocol`): fee-EMA params, emission rate, `TOTAL_SUPPLY_CAP`.
  - **Fixed `const` (NOT governable):** `MAX_PROPOSERS` (tx-size/liveness constraint), anything affecting account layout.
- `set_config` updates only the `Protocol`-resident governable fields, bounds-checked (denominators > 0, fractions ≤ 1, windows > 0); never retroactively touches existing oracles.

### Dead-end resolution
- `resolve_deadend(oracle, option)` gated to `dao_authority`: `require_phase(InvalidDeadend)`, `option < options_count` → set `Phase::Resolved` + `resolved_option`. The **economic settlement** of a governance-resolved dead-end is deferred to the settlement milestone (likely stakes returned, no rewards) — this milestone only sets the terminal outcome.

### Price oracle
- `kass_price` reads the **canonical KASS/USDC Meteora DAMM v2 pool TWAP** (layout from F0 recon), asserting the passed pool == `Protocol.kass_usdc_pool` (governance-blessed; prevents attacker-pool substitution). Ships as a validated primitive with **no on-chain consumer yet** (the challenge-market rework consumes it next milestone) — expected, not dead code.

### Bootstrapping
- An init/setup step records `dao_authority` + `kass_usdc_pool` in `Protocol` and confirms the KASS mint authority is the program PDA. (Emission mint authority stays the program PDA per the settlement design; the DAO governs the emission *rate*, not direct minting.)

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
Add to `Protocol` (re-pin layout): `dao_authority: Pubkey`, `kass_usdc_pool: Pubkey`, and the global governable params (fee-EMA params, emission rate, `TOTAL_SUPPLY_CAP` — reserve fields even if settlement sets their semantics later). Add an init/setup instruction (or extend `init_protocol`) — gated appropriately — that records `dao_authority` + `kass_usdc_pool` and confirms/sets the KASS mint authority = program PDA. Tests: setup records the fields; the canonical pool is pinned. (Bootstrapping note: in v1, who sets `dao_authority` initially — the `admin` from `init_protocol`, transferring to the DAO — document the trust assumption.)

### F2 — Config-as-state refactor (largest churn)
Add the snapshot fields to `Oracle` (re-pin layout). `create_oracle` snapshots the current global governable per-oracle params from `Protocol`/config into the `Oracle`. Switch every processor that reads a snapshotted param from `config::X` to `oracle.x` (finalize_facts, vote_fact, submit_fact, finalize_ai_claims, settle_challenge, finalize_oracle, advance_phase, propose, finalize_proposals — wherever a snapshotted const is used). Keep `MAX_PROPOSERS` + layout sizes `const`. All existing tests must still pass (behavior identical when config == defaults). Re-pin layouts; update the conservation/invariant assumptions only if needed.

### F3 — `set_config` (Ix append; gated)
Gated to `Protocol.dao_authority` (signer). Updates the `Protocol`-resident global governable fields, bounds-checked. Does NOT touch existing oracles. Tests: dao_authority can set; non-authority → Unauthorized; out-of-bounds → error; a subsequently-created oracle snapshots the new values.

### F4 — `resolve_deadend` (Ix append; gated)
Gated to `dao_authority`. `require_phase(InvalidDeadend)`, `option < oracle.options_count` → `set_phase(Resolved)` + `resolved_option`. Document that economic settlement is deferred. Tests: dao_authority resolves a dead-end → Resolved+option; non-authority → Unauthorized; wrong phase → WrongPhase; option out of range → error.

### F5 — `kass_price` (Meteora DAMM v2 TWAP read)
A read (instruction or pure helper over a passed pool account) that returns the KASS/USDC TWAP from the canonical Meteora DAMM v2 pool, asserting `pool == Protocol.kass_usdc_pool`. Layout from F0. Test: a cranked Meteora pool yields a sane TWAP; wrong pool → rejected. No on-chain consumer yet (next milestone).

### F6 — v0.6 futarchy proposal→execute integration (+ seam fallback)
Drive a governance proposal carrying a `set_config` (or `resolve_deadend`) CPI through the v0.6 futarchy: create proposal → conditional pass/fail KASS markets (v0.6 vault + Meteora DAMM v2) → trade to a pass verdict → execute → assert the config changed / oracle resolved, with execution signed by `dao_authority`. **Fallback (document honestly):** if driving the full v0.6+Meteora flow in LiteSVM is impractical, test the **seam** directly (privileged instructions accept the real `dao_authority` PDA as signer and reject others; CPI shapes validated against the dumped binaries) and integration-test the v0.6 execution path as far as LiteSVM allows.

---

## Out of scope (later milestones)
- Challenge-market rework (bond-as-AMM-liquidity + directional fees) consuming `kass_price` — NEXT.
- Staker settlement (returns/rewards/emissions/closure) — see the settlement-economics note.
- Full DAO treasury spending; migrating the dispute-core challenge markets from v0.4 to v0.6/Meteora.

## Execution note
After each task: `just build` → `cargo test -p kassandra-program` → clippy/fmt, confirm green, commit. Never proceed on a red bar. Keep an "Implementation deltas (F0–F6)" running log appended here. F0 is the highest risk (resolve real v0.6/Meteora IDs + layouts first); F2 is the largest churn; F6 is the hardest test (with the documented seam fallback).

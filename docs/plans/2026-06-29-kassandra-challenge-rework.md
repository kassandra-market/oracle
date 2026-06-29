# Kassandra Challenge-Market Rework — Design + Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Complete the challenge decision-market economics: a **conditional market on the proposer's stake** (pass-KASS/fail-KASS priced in conditional USDC — traders price the bond's value conditional on the claim surviving vs being disqualified), with the **challenger escrowing USDC** (sized via `kass_price`), **physical settlement** (the deferred `redeem_tokens`), and **directional fees**. Keeps the v0.4 AMM + TWAP slash trigger (already built). Second step of the roadmap: KASS futarchy ✅ → **challenge-market rework** → staker settlement.

**Architecture:** Extends the existing Pinocchio program. The challenge market reuses MetaDAO **v0.4** conditional vault + AMM (the v0.4 AMM has the built-in TWAP the slash reads; Meteora has none). Bond stays clean-slashable; the conditional-stake model is preserved.

**Tech Stack:** Rust, `pinocchio` 0.8, `bytemuck`, `litesvm`, `solana-sdk` (test-only), `spl-token`, MetaDAO v0.4 conditional vault + AMM, `kass_price` (futarchy spot TWAP, from the merged futarchy milestone).

**Source of truth:** the recon findings `docs/plans/2026-06-29-challenge-rework-recon.md`; the dispute-core + happy-path + futarchy deltas (live state); `docs/plans/2026-06-29-kassandra-settlement-economics.md` (the broader settlement note). FOLLOW THE LIVE STATE.

---

## Validated design (brainstormed + recon-grounded)

### The conditional-stake market (NOT plain KASS/USDC)
- `open_challenge` splits the proposer's **bond** into **pass-KASS / fail-KASS** conditional tokens (as it does today). The pass/fail AMMs price pass-KASS in pass-USDC and fail-KASS in fail-USDC — i.e. **traders price a unit of the proposer's stake conditional on the claim surviving (pass) vs being disqualified (fail).** pass/fail-KASS are fungible across participants, so the TWAP reflects the conditional value of the stake regardless of whose tokens trade.
- **The bond's conditional tokens stay IDLE (never LP'd)** → no impermanent loss on the bond (recon finding: LP'ing the bond makes it unrecoverable; holding idle + redeeming is the clean "escrow/idealized" model — and it's what's built). 
- **Market liquidity is the CHALLENGER's** (+ traders'): their conditional KASS + conditional USDC seed the pools (out-of-band, as the current tests do) — their IL, never the bond's.
- **Slash trigger (unchanged):** TWAP of fail-stake-price vs pass-stake-price; disqualify iff `fail_twap * DEN > pass_twap * (DEN + NUM)` (the `oracle.market_threshold_*` snapshot). `pass_twap == 0` → survive (no counter-trading).

### Challenger USDC stake
- `open_challenge` **escrows the challenger's USDC** into a market-owned USDC vault, amount sized via `kass_price` (≈ the bond's KASS value, so both sides have comparable skin-in-the-game). This escrow is the source of the USDC directional fee and is returned (minus fee) at settle.

### Physical settlement + directional fees (settle_challenge)
Implements the previously-deferred `redeem_tokens` + adds fees:
- **Redeem the bond's idle conditional tokens** 1:1 on the resolved (winning) side → underlying KASS into `stake_vault`.
- **Survives (challenge failed):** bond stays the proposer's (no slash); **USDC fee** = `challenger_usdc × fail_usdc_fee_num/den` → proposer; remaining challenger USDC escrow → returned to challenger.
- **Disqualified (challenge succeeded):** bond → `bond_pool` **minus a KASS fee** = `bond × success_kass_fee_num/den` → challenger; challenger's USDC escrow returned in full. (`slashed_amount` accounting stays consistent: the proposer's bond_pool contribution = bond − kass_fee; document the fee as a carve-out, and keep the per-proposer identity.)
- Directional-fee rates are **governable config** (new snapshot fields).

### Invariants
- Bond is never AMM liquidity → clean slashing + KASS conservation preserved (extended to count the market USDC escrow + the redeemed conditional KASS).
- Challenger USDC escrow is conserved: returned to challenger + fee to proposer == escrowed amount.

---

## Conventions (unchanged)
TDD; `just build` before `cargo test`; clippy + fmt clean; commit trailer `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`, git author `Kassandra <hexadecifish@gmail.com>`; append-only Ix/error discriminants; re-pin `tests/state_layout.rs` on layout change. rust-analyzer false positives — rely on real cargo runs.

## Live-state entry points
- `Ix` up to `KassPrice=16`; `KassandraError` up to `InvalidConfig=26`. `Protocol` LEN 336, `Oracle` LEN 328 (governable params snapshot). `Market` LEN 384 (records oracle/ai_claim/proposer/challenger/question/kass_vault/usdc_vault/pass_amm/fail_amm/oracle_pass_kass/oracle_fail_kass/twap_end/challenger_usdc/settled/bump).
- `open_challenge` (Ix=4): splits bond→idle conditional KASS, records Market (incl. `challenger_usdc` amount, currently NOT escrowed), `challenged=1`. `settle_challenge` (Ix=5): TWAP slash + `resolve_question`; **physical redeem + fees DEFERRED** (this milestone).
- `kass_price(protocol, kass_dao_ai) -> u128` (futarchy spot TWAP), anchored to `Protocol.kass_dao`. `assert_dao_authority`, `set_config` (governable params), `load_protocol/oracle/...`, `create_pda`. v0.4 CPI in `src/cpi/metadao.rs` (split/merge/redeem discriminators incl. `redeem_tokens` `f6 62 86 29 98 21 78 45`, add/remove-liquidity shapes documented in the recon doc).

---

## Tasks (C0 recon DONE)

### C1 — Challenger USDC escrow + fee config
- **Add governable fee fields** to `Protocol` + `Oracle` (snapshot; re-pin both layouts): `challenge_fail_usdc_fee_num/den` (USDC fee on a failed challenge), `challenge_success_kass_fee_num/den` (KASS fee on a successful challenge). Default to sensible config consts (e.g. 1/100 = 1%). `init_protocol` defaults them; `create_oracle` snapshots; `set_config` updates them with bounds (den>0, num≤den) — extend its payload + bounds. Update the F3 set_config payload length/tests.
- **`Market`** gains a `challenger_usdc_vault: Pubkey` (the market-owned USDC escrow token account) — re-pin Market layout. (Or reuse an existing field if cleaner; document.)
- **`open_challenge`:** add accounts for `protocol` + `kass_dao` (to call `kass_price`) + the challenger's USDC source token account + the market USDC escrow vault (created here, owned by the market/oracle PDA). Compute the required escrow = `bond_kass × kass_price` converted across KASS 9dp / USDC 6dp / the TWAP scale (DOCUMENT the exact conversion + scale; use u128, overflow-safe). Transfer that USDC challenger→escrow (challenger signs). Reject if the challenger's `challenger_usdc` payload disagrees with the computed size beyond a tolerance, OR just compute it and ignore the payload field (document). Keep the existing bond split + market binding + `challenged=1`.
- Tests: open_challenge escrows the right USDC amount (sized by a known kass_price); under/over-funded challenger → fails; fee config snapshotted onto the oracle; set_config updates fee rates (bounds enforced).

### C2 — settle_challenge: physical redeem + directional fees
- **Implement `redeem_tokens`** (the deferred CPI): after `resolve_question`, redeem the bond's idle pass/fail conditional KASS (`oracle_pass_kass`/`oracle_fail_kass`) → underlying KASS into `stake_vault`, program-signed by the oracle PDA. Winning side 1:1, losing side → 0 (recon-confirmed). Net: the full bond KASS lands back in `stake_vault`.
- **Directional fees + routing:**
  - **Survives** (pass-win): bond stays the proposer's (already counted as surviving). Take `fail_usdc_fee = challenger_usdc × fail_usdc_fee_num/den` from the escrow → transfer to the proposer (or the proposer's claimable balance — match how staker settlement will claim; for now, transfer to a proposer-controlled account OR credit a counter — DOCUMENT, keeping it consistent with the deferred staker-settlement milestone). Return `challenger_usdc − fee` → challenger.
  - **Disqualified** (fail-win): bond → `bond_pool` (already counted) **minus** `success_kass_fee = bond × success_kass_fee_num/den` → challenger (transfer KASS from `stake_vault`, program-signed); adjust `bond_pool`/`slashed_amount` so the proposer's contribution == bond − kass_fee (keep the per-proposer identity; the fee is a carve-out to the challenger, documented). Return the full `challenger_usdc` escrow → challenger.
- Update conservation: the market USDC escrow is fully accounted (challenger return + proposer fee == escrow); the redeemed bond KASS is in `stake_vault`.
- Tests: fraud path (disqualified) → bond−kass_fee to bond_pool, kass_fee to challenger, full USDC returned, conditional KASS redeemed; honest path (survives) → bond intact, usdc_fee to proposer, USDC remainder returned; conservation asserted (KASS + USDC).

### C3 — End-to-end + conservation/invariant update
- E2E challenge test driving the REAL v0.4 AMM: open_challenge (with USDC escrow) → challenger seeds liquidity + swaps to drive the TWAP → crank → warp → settle_challenge → assert the full physical settlement + fees for BOTH outcomes.
- Extend the invariant fuzz / conservation assertions to cover the challenge path (KASS: stake_vault + bond_pool reconciles incl. the kass_fee carve-out; USDC: escrow == challenger_return + proposer_fee).
- Remove or fold in the throwaway `tests/recon_lp_resolution.rs` recon test (keep if it documents the IL finding usefully; else drop).

---

## Out of scope (later)
- Staker settlement (per-staker claim/return/reward, emissions) — the broader settlement note. The challenger KASS-fee / proposer USDC-fee land where that milestone's claim model expects (document the hand-off precisely).
- Migrating challenge markets to v0.6/Meteora (kept on v0.4 for the TWAP).

## Execution note
After each task: `just build` → `cargo test` → clippy/fmt, green, commit. Re-pin layouts on change. The kass_price→USDC sizing conversion (decimals/scale) and the redeem_tokens CPI are the two trickiest spots — validate against the real binary. Append a C1/C2/C3 delta log here.

---

## C1 delta log — challenger USDC escrow + governable challenge fees (DONE)

### kass_price units/scale + the USDC conversion (the load-bearing bit)
- `kass_price(&Protocol, kass_dao_ai) -> u128` returns the futarchy spot TWAP =
  `aggregator / seconds_elapsed`, which is a price in **raw quote units per raw
  base unit, scaled by `KASS_PRICE_SCALE = 1e12`** (`futarchy_spot_twap`'s
  `PRICE_SCALE`). For the KASS DAO base = KASS (9dp), quote = USDC (6dp), so the
  value is **raw-USDC per raw-KASS × 1e12** — the cross-decimal (9dp↔6dp)
  adjustment is ALREADY folded into the raw price, so NO extra `10^Δdec` factor
  is needed.
- **Conversion (overflow-safe, u128 intermediate):**
  `required_usdc (USDC base units) = bond_kass (KASS base units) × twap / KASS_PRICE_SCALE`,
  then checked back into `u64`. `bond_kass == proposer.bond`.
- **Worked example (the test price):** KASS at $0.50 → `twap = 500_000_000`; a
  1 KASS bond (`1e9` base units) escrows `1e9 × 5e8 / 1e12 = 500_000` USDC base
  units = $0.50. Dimensionally `[KASS_raw] × [USDC_raw/KASS_raw] = [USDC_raw]`.
- New const `config::KASS_PRICE_SCALE = 1_000_000_000_000`.

### New config consts (`config.rs`)
- `KASS_PRICE_SCALE = 1e12`.
- `CHALLENGE_FAIL_USDC_FEE_NUM/DEN = 1/100` (1% USDC fee on a failed challenge → proposer; routed at C2).
- `CHALLENGE_SUCCESS_KASS_FEE_NUM/DEN = 1/100` (1% KASS fee on a successful challenge → challenger; routed at C2).

### Layout re-pins (each adds fields; `tests/state_layout.rs` updated)
- **Protocol** `336 → 368`: appended 4 × u64 after `reward_fact_weight@328`:
  `challenge_fail_usdc_fee_num@336`, `_den@344`, `challenge_success_kass_fee_num@352`, `_den@360`.
- **Oracle** `328 → 360`: appended the same 4 × u64 after `reward_fact_weight@320`:
  `challenge_fail_usdc_fee_num@328`, `_den@336`, `challenge_success_kass_fee_num@344`, `_den@352`.
  (`init_protocol` defaults the Protocol copies; `create_oracle` snapshots them onto the Oracle.)
- **Market** `384 → 416`: inserted `challenger_usdc_vault: Pubkey @360` (after
  `oracle_fail_kass@328`); shifting `twap_end@392`, `challenger_usdc@400`,
  `settled@408`, `bump@409`, `_pad[6]@410`.

### Escrow vault
- PDA seeds **`[b"challenge_usdc", market]`** (program = `crate::ID`); SPL token
  account on `oracle.usdc_mint`, **token authority = the oracle PDA** (mirrors
  `oracle.stake_vault`, so C2 settle signs returns/fees with the oracle seeds).
  Created in `open_challenge` via `create_pda` + `InitializeAccount3` (rent paid
  by the challenger), then funded by a challenger-signed SPL `Transfer` of
  `required_usdc`. Under-funded source → the `Transfer` fails → whole ix reverts.
- `Market.challenger_usdc` is now the ON-CHAIN-computed amount (not a payload
  value); `Market.challenger_usdc_vault` records the escrow account.

### `open_challenge` account order (Ix=4) — appended 5 accounts; payload now nonce-only (8 bytes)
`0 oracle(w) · 1 ai_claim(w) · 2 proposer(w) · 3 market(w) · 4 challenger(signer,w) ·
5 question · 6 kass_vault(w) · 7 usdc_vault · 8 pass_amm · 9 fail_amm · 10 stake_vault(w) ·
11 kass_vault_underlying(w) · 12 pass_mint(w) · 13 fail_mint(w) · 14 oracle_pass_kass(w) ·
15 oracle_fail_kass(w) · 16 cv_program · 17 token_program · 18 system_program ·
19 cv_event_authority · 20 protocol · 21 kass_dao · 22 usdc_mint · 23 challenger_usdc_src(w) ·
24 challenger_usdc_vault(w, uninit, created here)`. The escrow is sized + created
AFTER all MetaDAO bindings are verified (no funds move before validation).
Payload dropped the legacy `challenger_usdc` field (compute-on-chain is cleaner).

### set_config payload growth + new bounds
- Payload `18 → 22` u64 fields (`144 → 176` bytes); 4 appended:
  `challenge_fail_usdc_fee_num/den`, `challenge_success_kass_fee_num/den`.
- New bounds (→ `InvalidConfig`): `challenge_fail_usdc_fee_den > 0`,
  `challenge_success_kass_fee_den > 0`, `challenge_fail_usdc_fee_num ≤ den`,
  `challenge_success_kass_fee_num ≤ den`.
- Harness `ConfigParams` grew the 4 fields + `to_payload` is now `[u8; 176]`.

### Tests
- `open_challenge.rs`: happy path now asserts escrow == `bond × kass_price` USDC
  in the vault + Market record + challenger debit + vault mint/authority; new
  `open_challenge_insufficient_usdc_fails` (under-funded source reverts, no
  Market). All existing open_challenge + settle_challenge tests updated for the
  new accounts (harness `bless_kass_price` blesses a deterministic futarchy Dao
  blob; `fund_usdc` funds the challenger).
- `set_config.rs`: default-fee snapshot, fee update + new-oracle snapshot, and
  den==0 / num>den rejection.
- C2 (settle-side fee routing / redeem) intentionally NOT implemented here.

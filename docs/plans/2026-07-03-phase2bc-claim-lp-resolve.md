# Phase 2b/2c — `claim_lp` + `resolve_market` Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans / subagent-driven-development to implement task-by-task, TDD.

**Goal:** Complete the market lifecycle. **2b `claim_lp`:** distribute the AMM LP tokens seeded at `activate` pro-rata to contributors. **2c `resolve_market`:** once the Kassandra oracle is terminal, bridge its result into a program-signed MetaDAO `resolve_question`, so users can `redeem` their winning conditional tokens 1:1 (or pro-rata on a void).

**Architecture:** Both are permissionless cranks that perform Market-PDA-signed CPIs. `claim_lp` moves LP from the Market-PDA-owned `lp_vault` to a contributor (standard `pinocchio_token::Transfer.invoke_signed`). `resolve_market` reads the Kassandra `Oracle` (`resolved_option`/`phase`) and CPIs `resolve_question` on the market's MetaDAO Question (whose oracle-authority == the Market PDA) with numerators derived from the outcome. User redemption itself is client-side via the conditional vault — the program only provides the SDK `redeem_tokens` builder. All wire format below is verified against the sibling `../kassandra` (`cpi/metadao.rs`, `settle_challenge.rs`, `claims.rs`).

**Tech Stack:** Pinocchio 0.8, `pinocchio-token`, hand-built MetaDAO CPI. Tests: LiteSVM with the vendored MetaDAO `.so` fixtures (already in `tests/fixtures/`).

**Depends on:** Phase 2a `activate` (Market has `question, vault, yes_mint, no_mint, amm, lp_mint, lp_vault, lp_total, settled`, and `status == Active` after activation). Phase 1 has `Contribution { market, contributor, amount, claimed, bump }` and `Market { oracle, total_contributed, bump, ... }`.

---

## Task 1 — `claim_lp` (Phase 2b)

Pro-rata LP distribution to a contributor from `lp_vault`. Permissionless (anyone cranks for any contributor; funds only ever go to the recorded contributor's chosen LP token account, which we validate).

**Design note on the `claimed` flag:** Phase 1 `Contribution.claimed` is currently set by `refund`. A contribution can be refunded (cancelled market) XOR claim-LP'd (activated market) — never both, because a market is either Cancelled or Active, never both, and each path checks status. So **reuse the single `claimed` flag**: `claim_lp` requires `market.status == Active` and `contribution.claimed == 0`, sets it to 1. `refund` requires `status == Cancelled`. The flag means "this contribution has been settled" in either direction. (Verify this reasoning holds; if a clearer two-flag model is warranted, note it — but YAGNI favors one flag.)

**Payload:** empty. **Accounts (exact order):** `[market(ro), lp_vault(w), contribution(w), contributor_lp_ata(w), token_program(ro)]`.

**Guards:**
1. `assert_key(token_program, &pinocchio_token::ID)`.
2. `load_market`; `market.status == Active` else a new `MarketError::NotActive`.
3. `assert_key(lp_vault, &market.lp_vault)`.
4. `load_contribution`; `contribution.market == *market_ai.key()` else `InvalidAccount`; `contribution.claimed == 0` else `AlreadyClaimed`.
5. Destination LP ATA's SPL owner (bytes 32..64) == `contribution.contributor` else `InvalidAccount`; AND assert `dest` is token-program-owned (`assert_owned_by_program(dest, &pinocchio_token::ID)`) before reading — mirror the `refund` hardening; AND its mint (bytes 0..32) == `market.lp_mint` else `InvalidAccount` (so LP goes to a real LP-mint account).

**Action:** compute `share = lp_total × contribution.amount / total_contributed` via a **floor** u128-intermediate checked helper (mirror sibling `fee_amount`: `(lp_total as u128 * amount) / total_contributed`, `u64::try_from`). Program-signed `Transfer { from: lp_vault, to: dest, authority: market_ai, amount: share }.invoke_signed(&[Signer::from(&[b"market", market.oracle, [market.bump]])])`. No-op guard if `share == 0` (still mark claimed to prevent retry loops — or reject `ZeroAmount`; prefer marking claimed so dust contributors don't wedge). Set `contribution.claimed = 1`, write.

**Dust note:** floor division leaves at most `(n_contributors)` base units of LP undistributed in `lp_vault` forever (acknowledged; a Phase-3 sweep to the DAO could recover it — out of scope).

**SDK builder** `sdk-rs/src/ix.rs::claim_lp(market, lp_vault, contribution, contributor_lp_ata)`; `IX_CLAIM_LP = 7` in lib.rs + `Ix::ClaimLp = 7` in instruction.rs + dispatch + parity assertion.

**Harness:** `claim_lp(&mut self, market, contributor, contributor_lp_ata) -> TransactionResult` (derives contribution PDA). Also need a helper to create a contributor LP ATA (token account with mint = lp_mint, owner = contributor) — reuse `create_token_account(lp_mint, owner, 0)`.

**Tests `tests/claim_lp.rs`** (LiteSVM + `load_metadao` + full activate flow):
1. **Happy (two contributors, pro-rata):** fund a market with creator seed A + contributor B (A+B >= min), activate, then `claim_lp` for each. Assert each receives `lp_total × their_amount / total ` (floor), sum of claims ≤ `lp_total`, `lp_vault` drains to ≤ dust, both `Contribution.claimed == 1`.
2. **AlreadyClaimed:** second claim for same contributor → `AlreadyClaimed`.
3. **NotActive:** claim on a still-Funding market → `NotActive`.
4. **Wrong dest owner / wrong mint:** dest LP ATA owned by a stranger, or a non-LP-mint account → `InvalidAccount`.
5. **Interaction with refund:** confirm a contribution that was `claim_lp`'d cannot then be `refund`'d and vice-versa (status guards make these mutually exclusive; assert the second fails).

Commit `feat: claim_lp — pro-rata LP distribution to contributors`.

---

## Task 2 — `resolve_market` (Phase 2c)

Bridge the terminal Kassandra oracle result into MetaDAO `resolve_question`, enabling redemption. Permissionless crank, idempotent.

**Payload:** empty. **Accounts (exact order):** `[market(w), oracle(ro), question(w), cv_event_authority(ro), cv_program(ro)]`. (Market PDA is the resolver/signer — it is NOT a separate account; it signs via seeds. But `market` must be writable to set `settled`. The Question's `oracle` field == Market PDA, so the Market PDA account itself is passed as the `readonly_signer` in the CPI metas — i.e. the `market` account doubles as the resolver signer. Confirm pinocchio allows the same AccountInfo as both a writable top-level account and a readonly_signer in a CPI meta; if not, the Market PDA is passed once and referenced in both roles. Mirror how the sibling passes `oracle_ai` — there it's a separate readonly_signer from the writable market. Here they coincide; validate this works or split into two AccountInfos of the same key.)

**Guards:**
1. `load_market`; `market.status == Active` else `NotActive` (only an activated market has a Question to resolve).
2. `market.settled == 0` else `AlreadySettled` (new error) — idempotency. (Belt-and-suspenders: also read `Question.payout_denominator @84`; if already != 0, the question is resolved — treat as already done. But the `settled` flag is the primary guard.)
3. `assert_key(question, &market.question)`; `assert_owned_by_program(question, &CONDITIONAL_VAULT_ID)`.
4. `assert_key(cv_program, &CONDITIONAL_VAULT_ID)`; verify `cv_event_authority` == `event_authority_pda(CONDITIONAL_VAULT_ID)` (re-derive `[b"__event_authority"]`).
5. Load the Kassandra oracle; require terminal: `phase == Phase::Resolved (7)` OR `phase == Phase::InvalidDeadend (8)` else `OracleNotTerminal`. `assert_key(oracle, &market.oracle)`.

**Numerator selection:**
- If `phase == Resolved`: read `resolved_option: u8` (@197 in the Oracle). It's an index into `0..options_count`; for our binary market it's 0 or 1. `numerators = if resolved_option == 0 { [1,0] } else if resolved_option == 1 { [0,1] } else { <unexpected — reject InvalidAccount, our markets are binary> }`. (YES = option 0, NO = option 1 — this is the semantic binding; document it. Must match how `create_market` interprets the oracle's options for a binary question. Confirm the intended YES/NO↔option-index mapping with the design; recorded here as YES=0, NO=1.)
- If `phase == InvalidDeadend`: `numerators = [1,1]` (void — every conditional token redeems for 0.5 KASS; losers-vs-winners nets at the pool level per the design).

**Action:** build `resolve_question_data_binary(numerators)` (disc `3420e0b3b40800f6` ++ `2u32 LE` ++ `n0:u32 LE` ++ `n1:u32 LE`, 20 bytes — add to `cpi/metadao.rs`). CPI metas: `[question(w), market_pda(readonly_signer), cv_event_authority(ro), cv_program(ro)]`; `invoke_conditional_vault_signed(&data, &metas, &infos, &[Signer::from(&[b"market", market.oracle, [market.bump]])])`. Set `market.settled = 1` and `market.status = Resolved` (or `Void` if InvalidDeadend), write once.

**MarketStatus:** on success set `Resolved = 2` (normal) or `Void = 3` (deadend). This distinguishes them for any UI/analytics; both mean "question resolved, redeem open."

**SDK builders:** `sdk-rs/src/ix.rs::resolve_market(market, oracle, question, cv_event_authority)` (`IX_RESOLVE_MARKET = 8`, `Ix::ResolveMarket = 8`, dispatch, parity). ALSO add the client-side **`redeem_tokens`** builder to `sdk-rs/src/metadao.rs` (disc `f662862998217845`, no args, InteractWithVault account order: `question(ro), vault(w), vault_underlying_ata(w), authority(signer), user_underlying_ata(w), token_program, event_authority, cv_program, yes_mint(w), no_mint(w), user_yes_acct(w), user_no_acct(w)`) so users/tests can redeem. (Optionally a `merge_tokens` builder too, disc `e259fb79e182b40e`, args amount u64 — for completeness / void handling; include if cheap.)

**Harness:** `resolve_market(&mut self, market, oracle, question)` (derives cv_event_authority). A `set_oracle_resolved(oracle, resolved_option)` helper extending `set_oracle_phase` to also stamp `resolved_option` at @197 (and options_count=2). A client `redeem(user, market_refs, outcome)` helper for the redemption assertion.

**Tests `tests/resolve_market.rs`** (LiteSVM + full activate flow):
1. **Happy YES wins:** activate a market, `set_oracle_resolved(oracle, 0)`, `resolve_market` → assert `market.settled == 1`, `status == Resolved`, and the Question is resolved (read `payout_denominator @84 != 0`, `num0@76 == 1`, `num1@80 == 0`). Then a user who holds `yes` conditional tokens (from a swap or split) redeems and receives KASS; a `no` holder redeems and gets 0. (Simplest: have the Market's own split leftover, or do a client split for a test user, then redeem.)
2. **Happy NO wins:** `resolved_option = 1` → `[0,1]`, status Resolved.
3. **Void:** `set_oracle_phase(oracle, 8)` (InvalidDeadend) → `resolve_market` → `[1,1]`, `status == Void`; a holder of either leg redeems ~half.
4. **OracleNotTerminal:** oracle in Proposal → `OracleNotTerminal`.
5. **AlreadySettled:** second `resolve_market` → `AlreadySettled` (idempotent).
6. **NotActive:** resolve a Funding (never-activated) market → `NotActive`.

Commit `feat: resolve_market — bridge oracle result into MetaDAO resolve_question`.

---

## Task 3 — End-to-end lifecycle + review

- **Full e2e `tests/lifecycle_active.rs`:** `init_config → create_market → contribute (to min) → activate → claim_lp (all contributors) → [trade: one MetaDAO swap so a user holds a net YES position] → oracle resolves → resolve_market → user redeems winnings → LP holder removes liquidity + redeems`. Assert conservation: total KASS out ≤ total KASS in (escrow), winners paid, no funds stranded beyond acknowledged dust.
- `just test` fully green; `cargo clippy -p kassandra-market-program --tests` clean.
- Two-stage review (spec + code-quality) on `resolve_market` and `claim_lp` — both are fund-custody + wire-format critical (program-signed LP transfer; program-signed resolution that determines who gets paid).
- Update `docs/plans/2026-07-03-kassandra-market-design.md` Status: Phase 2 complete (activate + claim_lp + resolve_market), binary markets fully live end-to-end in LiteSVM.
- Commit `test: full active-market lifecycle e2e + status update`.

---

## Wire-format reference (verified against `../kassandra`)

- **resolve_question:** disc `34 20 e0 b3 b4 08 00 f6`; payload `resolve_question_data_binary([n0,n1]) = disc ++ 2u32_LE ++ n0_u32_LE ++ n1_u32_LE` (20 bytes; the `2` is the Borsh Vec length). Accounts: `question(w), resolver(readonly_signer), cv_event_authority(ro), cv_program(ro)`. Resolver must equal `Question.oracle @40` and sign. `[1,0]`=outcome0 pays, `[0,1]`=outcome1 pays, `[1,1]`=void (denominator 2, each leg pays half).
- **Question offsets:** `oracle @40`, `num_outcomes_len @72`, `num0 @76`, `num1 @80`, `payout_denominator @84`. `is_resolved ⇔ denominator @84 != 0`.
- **redeem_tokens:** disc `f6 62 86 29 98 21 78 45`, no args. InteractWithVault accounts: `question(ro), vault(w), vault_underlying_ata(w), authority(signer), user_underlying_ata(w), token_program, event_authority, cv_program`, then remaining `conditional_token_mint[0..n](w)` then `user_conditional_token_account[0..n](w, owner==authority)`. Requires `question.is_resolved()`. Burns full balance of each leg, pays `Σ balance_i × num_i / denominator`.
- **merge_tokens:** disc `e2 59 fb 79 e1 82 b4 0e`, args `amount:u64 LE` (16 bytes), same InteractWithVault accounts. Inverse of split.
- **Program-signed SPL transfer out of a PDA vault:** `pinocchio_token::instructions::Transfer { from: pda_vault, to: dest, authority: pda_ai, amount }.invoke_signed(&[Signer::from(&seeds)])`. No-op if `amount == 0`.
- **Pro-rata (floor):** `u64::try_from((value as u128).checked_mul(num as u128)? / (den as u128))` — never over-distributes; dust stays in vault.
- **conditional_vault program:** `VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg`. **event_authority:** `[b"__event_authority"]` under that program.

## Deferred (still, after Phase 2)
- Uneven opening prior (50/50 only). Categorical N>2. Protocol fee. LP/token-account close for rent recovery + dust sweep. TS SDK + app. Surfpool mainnet-fork e2e (LiteSVM fixtures suffice for logic).

# Phase 2a — `activate` Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (or subagent-driven-development) to implement this plan task-by-task, TDD.

**Goal:** Implement `activate` — the instruction that turns a fully-funded `kassandra-market` `Market` into a live MetaDAO conditional-vault + AMM prediction market, by verifying a client-composed MetaDAO market, splitting the escrowed KASS into `cYES`/`cNO`, and seeding the AMM pool. This is the first MetaDAO-CPI task (Phase 2a); `claim_lp` (2b) and `resolve_market` (2c) follow in separate plans.

**Architecture:** Mirror the sibling Kassandra `open_challenge` precedent (`../kassandra/programs/kassandra/src/processor/open_challenge.rs`): the **client composes** the MetaDAO `Question` + `conditional_vault` + `Amm` in its own transactions; our program **verifies + records** those bindings and performs only the CPIs that require the Market-PDA authority (`split_tokens`, `add_liquidity`). All MetaDAO CPI wire format (discriminators, PDA seeds, account orders, account offsets) is documented in the map appended at the end of this plan — treat it as the source of truth and re-verify against `../kassandra/programs/kassandra/src/cpi/metadao.rs` while implementing.

**Tech Stack:** Pinocchio 0.8, `pinocchio-token`, hand-built Anchor CPI (8-byte sighash + Borsh args) into MetaDAO `conditional_vault` (`VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg`, v0.4.0) and `amm` (`AMMyu265tkBpRW21iGQxKGLaves3gKm2JcMUqfXNSpqD`, v0.4.2). Tests: LiteSVM with the MetaDAO `.so` fixtures (copied from the sibling repo) — no surfpool required for this path.

---

## ⚠️ Open design decision (flagged, default chosen)

Our design uses a **single `cYES`/`cNO` pool** (base = `cYES`, quote = `cNO`) and lets the creator set an uneven opening prior via `open_yes_bps`. But `split_tokens(N)` mints **equal** `cYES`/`cNO`, while an uneven spot price needs uneven reserves. **This plan seeds the pool 50/50** (balanced, zero stranded capital) and keeps `open_yes_bps` recorded-but-unused for now. Honoring the uneven prior (uneven reserves + merge-back of the balanced remainder, or a TWAP-observation seed) is a deferred refinement that changes only the `add_liquidity` amounts — the composition and verification below are identical. **Revisit with the user before shipping to mainnet.**

---

## Structure this plan builds on (Phase 1, already implemented)

- `Market` (PDA `[b"market", oracle]`) has `status: u8` (`MarketStatus`: Funding=0, Active=1, Resolved=2, Void=3, Cancelled=4), `escrow_vault` (KASS token account owned by the Market PDA), `total_contributed`, `open_yes_bps`, `min_liquidity`, `oracle`, `bump`, `escrow_bump`. **This task extends `Market`** with the MetaDAO bindings (Task 1).
- The Market PDA is the escrow authority and will be the Question's oracle-authority + the split/liquidity authority — it signs CPIs with seeds `[b"market", oracle, [market.bump]]` (same seeds `refund` already uses).

---

## Task 0: Vendor the MetaDAO `.so` fixtures + LiteSVM loader

**Files:**
- Copy: `../kassandra/programs/kassandra/tests/fixtures/metadao_conditional_vault.so` → `programs/kassandra-market/tests/fixtures/metadao_conditional_vault.so`
- Copy: `../kassandra/programs/kassandra/tests/fixtures/metadao_amm.so` → `programs/kassandra-market/tests/fixtures/metadao_amm.so`
- Modify: `programs/kassandra-market/tests/common/mod.rs`

**Steps:**
1. Copy the two `.so` fixtures (check the sibling's `.gitignore`/git-lfs handling; if they are git-tracked binaries, copy the actual bytes). Add a `tests/fixtures/` note to `.gitignore` only if they should NOT be committed — but they must be present for tests, so commit them (they are ~small deployed programs).
2. In the harness, add a `load_metadao(&mut self)` method: `self.svm.add_program(Pubkey::new_from_array(CONDITIONAL_VAULT_ID), include_bytes!("../fixtures/metadao_conditional_vault.so")); self.svm.add_program(Pubkey::new_from_array(AMM_ID), include_bytes!("../fixtures/metadao_amm.so"));` (mirror sibling `challenge_e2e.rs`).
3. **Verify:** a throwaway test that calls `TestCtx::new()` + `load_metadao()` and asserts both programs are present (`svm.get_account(program_id).executable`). Commit `test: vendor MetaDAO vault+amm fixtures + LiteSVM loader`.

## Task 1: Extend `Market` with MetaDAO bindings + `MarketStatus::Active`

**Files:** `src/state.rs`; `tests/state_layout.rs`.

Add fields to `Market` (append after the Phase-1 fields; re-pin the new LEN and offsets in `state_layout.rs`):
- `question: Pubkey` — the MetaDAO binary Question (its oracle-authority == this Market PDA).
- `vault: Pubkey` — the KASS conditional vault.
- `yes_mint: Pubkey`, `no_mint: Pubkey` — conditional-KASS mints idx 0/1.
- `amm: Pubkey` — the `cYES`/`cNO` pool.
- `lp_mint: Pubkey`, `lp_vault: Pubkey` — the pool's LP mint and the Market-PDA-owned LP token account holding the seeded liquidity.
- `lp_total: u64` — LP tokens minted at activation (basis for pro-rata `claim_lp` in Phase 2b).
- `settled: u8` — reserved for Phase 2c (`resolve_market`).

Follow the Phase-1 layout conventions exactly (repr(C), Pod, explicit `_pad`, 8-byte multiple, pinned offsets + LEN). TDD: write the new layout assertions first (they fail), then extend the struct. Commit `feat: extend Market with MetaDAO market bindings`.

## Task 2: `sdk-rs` MetaDAO CPI module (single source of truth)

**Files:** `sdk-rs/src/metadao.rs` (new); `sdk-rs/src/lib.rs`; `sdk-rs/src/pda.rs`.

Port the wire format from the appended map (and re-verify against `../kassandra/programs/kassandra/src/cpi/metadao.rs`):
- **Program IDs** `CONDITIONAL_VAULT_ID`, `AMM_ID`.
- **Discriminators** for `initialize_question`, `initialize_conditional_vault`, `split_tokens`, `create_amm`, `add_liquidity`, `resolve_question` (others as needed later).
- **PDA derivers:** `question(question_id, oracle_authority, num_outcomes)` = `[b"question", question_id, oracle_authority, [n]]`; `vault(question, underlying_mint)` = `[b"conditional_vault", question, underlying_mint]`; `conditional_token_mint(vault, index)` = `[b"conditional_token", vault, [index]]`; `event_authority(program)` = `[b"__event_authority"]`; `amm(base_mint, quote_mint)` = `[b"amm__", base, quote]`; `amm_lp_mint(amm)` = `[b"amm_lp_mint", amm]`.
- **Instruction builders** (client-composition, returning `solana_sdk::Instruction`): `initialize_question`, `initialize_conditional_vault`, `create_amm`, `add_liquidity` — with the exact account orders in the map §3/§4. These are what the test harness/keeper uses to compose the market before calling `activate`.
- For **`question_id`** use the Kassandra oracle address (unique per market, deterministic, verifiable on-chain).

Add a `parity.rs` assertion that these discriminators match the ones the program will use (share them via a common const module, or assert equality against the program crate's `cpi` module once Task 3 exists). TDD not strictly applicable to pure constants; instead add a small test that derives a known Question/vault/mint PDA and checks determinism. Commit `feat: sdk-rs MetaDAO v0.4 CPI builders + PDAs`.

## Task 3: Program-side MetaDAO CPI helpers

**Files:** `src/cpi/metadao.rs` (new); `src/cpi/mod.rs`; `src/lib.rs` (add `pub mod cpi;`).

Mirror `../kassandra/programs/kassandra/src/cpi/metadao.rs` but keep ONLY what `activate` needs: the two program IDs, the discriminators + arg encoders for `split_tokens_data(amount)` and (if the program builds them) nothing else — because `create_amm`/`add_liquidity` are composed client-side; **the only CPIs `activate` itself invokes are `split_tokens` and `add_liquidity`** (both Market-PDA-signed). So the program needs: `split_tokens_data`, `add_liquidity_data(quote_amount, max_base_amount, min_lp_tokens)`, the account-offset reader consts for `Question` (`oracle @40`, `num_outcomes_len @72`), `ConditionalVault` (`question @8`, `underlying_mint @40`, `underlying_account @72`, `conditional_token_mints @104`), and `Amm` (`base_mint @49`, `quote_mint @81`, `AMM_ACCOUNT_DISCRIMINATOR`), plus `read_pubkey/read_u32/read_u64` LE bounded readers and `event_authority` seed. Add `invoke_conditional_vault_signed` / `invoke_amm_signed` wrappers over `pinocchio::cpi::slice_invoke_signed`. TDD: unit-test the arg encoders (byte-exact) and readers. Commit `feat: program MetaDAO CPI helpers (split + add_liquidity + readers)`.

## Task 4: `activate` — verify, split, seed liquidity, record

**Files:** `src/processor/activate.rs`; `src/instruction.rs` (add `Activate = 6`); `src/processor/mod.rs`; `sdk-rs/src/{ix.rs,lib.rs}` (`IX_ACTIVATE = 6`, builder + full account list); `tests/parity.rs`; `tests/activate.rs`.

**Precondition (client, in the test harness / keeper, BEFORE `activate`):** compose the MetaDAO market in prior instructions — `initialize_question` (oracle-authority = Market PDA, num_outcomes = 2, question_id = kassandra oracle), `initialize_conditional_vault` (underlying = KASS), `create_amm` (base = `yes_mint`, quote = `no_mint`, balanced initial observation). Build a harness helper `compose_metadao_market(market, oracle, kass_mint) -> MetaDaoRefs` that does this and returns the derived addresses.

**`activate` behavior (program):**
1. Guard: `load_market`; `market.status == Funding` else `NotFunding`; `market.total_contributed >= market.min_liquidity` else a new `NotFunded` error (only fully-funded markets activate). Load the Kassandra oracle; require **non-terminal** (`phase < Resolved`) else `OracleResolved` (a terminal oracle must go the cancel/refund path, not activate — this closes the C1-adjacent race for the funded case).
2. **Verify the composed MetaDAO market** (mirror `open_challenge`): re-derive and `assert_key` the `question`, `vault`, `yes_mint`, `no_mint`, `amm`, `lp_mint` PDAs from the map's seeds; owner-check each against the correct MetaDAO program; read + assert bindings: `Question.oracle == market PDA`, `Question.num_outcomes == 2`, `Vault.question == question`, `Vault.underlying_mint == kass_mint`, `Vault.underlying_account == vault_underlying_ata`, conditional mint idx 0/1 == `yes_mint`/`no_mint`, and (stronger than `open_challenge`) `Amm.discriminator == AMM_ACCOUNT_DISCRIMINATOR`, `Amm.base_mint == yes_mint`, `Amm.quote_mint == no_mint`.
3. **Program-signed `split_tokens`** (authority = Market PDA, seeds `[b"market", oracle, [bump]]`): source = `market.escrow_vault`, amount = `total_contributed`, destinations = Market-PDA-owned `cYES`/`cNO` token accounts. (Create those two conditional-token ATAs for the Market PDA first, or require them pre-created + validated.)
4. **Program-signed `add_liquidity`** (authority = Market PDA): deposit the split `cYES`/`cNO` (balanced 50/50 for v1) into the pool; LP tokens → a Market-PDA-owned `lp_vault`.
5. **Record** into `Market`: `question, vault, yes_mint, no_mint, amm, lp_mint, lp_vault, lp_total = <LP minted>`, and set `status = Active`.

Account list will be large (~20+, like `open_challenge`). Specify the exact order in the SDK builder AND the processor slice, and confirm they match (parity of order verified by the passing happy-path test).

**Tests (`tests/activate.rs`, LiteSVM + MetaDAO fixtures):**
- **Happy path:** fund a market to `min_liquidity` (creator + contributors), `compose_metadao_market`, `activate` → assert `status == Active`, escrow KASS drained into the vault, the AMM pool holds the `cYES`/`cNO` reserves, `lp_vault` holds `lp_total > 0`, and all bindings recorded on `Market`.
- **Failures:** under-funded market → `NotFunded`; terminal oracle → `OracleResolved`; a mis-composed market (e.g. `Question.oracle != Market PDA`, or `Amm.base_mint` swapped) → the corresponding `InvalidAccount`; double-activate → `NotFunding`.

Commit `feat: activate — verify MetaDAO market, split escrow, seed AMM liquidity`.

## Task 5: End-to-end + review

- Full LiteSVM e2e: `init_config → create_market → contribute (to min) → compose → activate`, asserting the pool is live and tradeable (optionally do one `swap` via a MetaDAO builder to confirm the pool works).
- `just test` green; clippy clean.
- Dispatch spec + code-quality review (this is fund-custody + wire-format critical — full two-stage review). Update the design-doc status note (Phase 2a activation implemented).
- Commit `test: activate end-to-end + status update`.

## Deferred to later Phase-2 plans
- **2b `claim_lp`** — pro-rata LP distribution from `lp_vault` to contributors (basis: `Contribution.amount / total_contributed × lp_total`).
- **2c `resolve_market`** — read `resolved_option`, program-signed `resolve_question` (`[1,0]`/`[0,1]`; `[1,1]` void), enabling user `redeem`.
- **Uneven opening prior** (the flagged decision above).
- **TS SDK + app.**

---

## Appendix: MetaDAO v0.4 CPI wire-format map

*(Verbatim from the sibling `../kassandra` exploration — re-verify against `programs/kassandra/src/cpi/metadao.rs` while implementing.)*

**Program IDs:** conditional_vault `VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg` (v0.4.0), amm `AMMyu265tkBpRW21iGQxKGLaves3gKm2JcMUqfXNSpqD` (v0.4.2).

**Discriminators** (`sha256("global:<name>")[..8]`):
- `initialize_question` `f5 97 6a bc 58 2c 41 d4`
- `initialize_conditional_vault` `25 58 fa d4 36 da e3 af`
- `split_tokens` `4f c3 74 00 8c b0 49 b3`
- `merge_tokens` `e2 59 fb 79 e1 82 b4 0e`
- `redeem_tokens` `f6 62 86 29 98 21 78 45`
- `resolve_question` `34 20 e0 b3 b4 08 00 f6`
- `create_amm` `f2 5b 15 aa 05 44 7d 40`
- `add_liquidity` `b5 9d 59 43 8f b6 34 48`
- `swap` `f8 c6 9e 91 e1 75 87 c8`
- `crank_that_twap` `dc 64 19 f9 00 5c c3 c1`
- `Amm` account disc (`sha256("account:Amm")[..8]`) `8f f5 c8 11 4a d6 c4 87`

**PDA seeds:** Question `[b"question", question_id[32], oracle[32], [num_outcomes:u8]]`; ConditionalVault `[b"conditional_vault", question[32], underlying_mint[32]]`; conditional token mint `[b"conditional_token", vault[32], [index:u8]]`; event authority `[b"__event_authority"]` (per target program); AMM `[b"amm__", base_mint[32], quote_mint[32]]`; AMM LP mint `[b"amm_lp_mint", amm[32]]`.

**Arg encoders:**
- `initialize_question` = disc ++ `question_id[32]` ++ `oracle[32]` ++ `num_outcomes:u8` (73 bytes).
- `initialize_conditional_vault` = disc only (8 bytes).
- `split_tokens` / `merge_tokens` = disc ++ `amount:u64 LE` (16 bytes).
- `redeem_tokens` = disc only.
- `resolve_question` = disc ++ `2u32 LE` (Borsh Vec len) ++ `n0:u32 LE` ++ `n1:u32 LE` (20 bytes). `[1,0]` = outcome-0 (YES) wins; `[0,1]` = NO wins; `[1,1]` = void.
- `create_amm` (body) = disc ++ `twap_initial_observation:u128` ++ `twap_max_observation_change_per_update:u128` ++ `twap_start_delay_slots:u64`.
- `add_liquidity` (body) = disc ++ `quote_amount:u64` ++ `max_base_amount:u64` ++ `min_lp_tokens:u64`.
- `swap` (body) = disc ++ `swap_type:u8 (0=Buy,1=Sell)` ++ `input_amount:u64` ++ `output_amount_min:u64`.

**Account offsets** (after the 8-byte Anchor disc): `Question.oracle @40`, `Question.payout_numerators` len `@72` (resolved numerators at `@76/@80/@84` = `[num0,num1,denominator]`). `ConditionalVault.question @8`, `.underlying_token_mint @40`, `.underlying_token_account @72`, `.conditional_token_mints (Vec<Pubkey>) @104`. `Amm.base_mint @49`, `.quote_mint @81`, `.created_at_slot @9`, `.last_updated_slot @131`, `.aggregator @171`, `.start_delay_slots @219`, min len `227`.

**Account orders** (`#[event_cpi]` appends `event_authority` + target-program before any "remaining" accounts):
- `initialize_question`: `question(w,PDA) payer(signer,w) system_program event_authority cv_program`.
- `initialize_conditional_vault`: `vault(w,PDA) question underlying_mint vault_underlying_ata(w) payer(signer,w) token_program associated_token_program system_program event_authority cv_program` + `conditional_token_mint[0..n](w,PDA)`.
- `split_tokens`/`merge_tokens`/`redeem_tokens` (InteractWithVault): `question vault(w) vault_underlying_ata(w) authority(signer) user_underlying_ata(w) token_program event_authority cv_program` + `conditional_token_mint[0..n](w)` + `user_conditional_token_account[0..n](w, owner==authority)`.
- `create_amm`: `payer(signer,w) amm(w) lp_mint(w) base_mint quote_mint vault_ata_base(w) vault_ata_quote(w) associated_token_program token_program system_program amm_event_authority amm_program`.
- `add_liquidity`: `payer(signer,w) amm(w) lp_mint(w) user_lp(w) user_base(w) user_quote(w) vault_ata_base(w) vault_ata_quote(w) token_program amm_event_authority amm_program`.
- `resolve_question`: `question(w) oracle(readonly_signer=resolver) cv_event_authority cv_program`.

**Composition order (client):** initialize_question → initialize_conditional_vault (KASS) → create_amm(base=cYES, quote=cNO). Then `activate` does the Market-PDA-signed split + add_liquidity. **base = conditional-KASS (cYES), quote = conditional-KASS (cNO)** — note both sides are conditional-KASS from the single KASS vault (unlike Kassandra's cKASS/cUSDC futarchy pools).

> **Divergence from Kassandra to keep in mind:** Kassandra's challenge market uses TWO vaults (KASS + USDC) and TWO pools (pass, fail), each conditional-KASS/conditional-USDC — a futarchy metric comparison. Ours uses ONE KASS vault and ONE `cYES`/`cNO` pool — a plain probability market. The CPI mechanics are the same; the market topology is simpler.

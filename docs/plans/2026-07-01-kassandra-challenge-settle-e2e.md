# Challenge-market settle E2E (forked MetaDAO) — Design + Plan

> **For Claude:** REQUIRED SUB-SKILL: subagent-driven-development (per-task implement + review).

**Goal:** Close the T4 deferral: drive `settle_challenge` END-TO-END on forked-mainnet surfpool with a REAL swap-driven v0.4 AMM TWAP. Build the missing SDK builders for the MetaDAO v0.4 standalone AMM, then extend the surfpool challenge-market test to: open a challenge → create + seed the real pass/fail v0.4 AMM pools → swap to a verdict + crank the TWAP over ≥150-slot windows (via `surfnet_timeTravel`) → `settle_challenge` → assert BOTH arms (challenge SUCCEEDS → disqualify; challenge FAILS → survive). NO program change — `settle_challenge` is already fully implemented.

**Why now (the unlock):** T4 deferred this as "AMM-TWAP cranking non-deterministic on a fork," but the futarchy G3 milestone PROVED real AMM-TWAP cranking is feasible on a fork (real swaps + `surfnet_timeTravel` past the delayed-TWAP window). This applies that technique to the v0.4 AMM. The v0.4 AMM wire formats are FULLY KNOWN (no Meteora-style blocker).

## Source of truth (from investigation, file:line)
- **AMM:** MetaDAO v0.4 standalone AMM `AMMyu265tkBpRW21iGQxKGLaves3gKm2JcMUqfXNSpqD` (delayed-twap v0.4.2). Instruction discriminators + arg layouts pinned in `programs/kassandra/src/cpi/metadao.rs:78-94`; `Amm` account offsets + `get_twap` math at `:131-178`; pool/lp/vault PDA seeds (`[b"amm__", base, quote]`, `[b"amm_lp_mint", amm]`, AMM vault ATAs) + the full account orderings proven against the real `.so` in `programs/kassandra/tests/challenge_e2e.rs:631-769`. This is DIFFERENT from the v0.6 futarchy embedded AMM (G3) — separate program, explicit `create_amm`/`add_liquidity`/`swap`/`crank_that_twap`.
- **settle_challenge** (`programs/kassandra/src/processor/settle_challenge.rs`): reads each AMM's stored TWAP (`verify_and_read_twap`, no crank at settle: `aggregator/(last_updated_slot−(created_at+start_delay))`); decision (`:337-358`): disqualify iff `pass_twap != 0` AND `fail_twap·DEN > pass_twap·(DEN+NUM)` with `MARKET_THRESHOLD = 1/10` (10% margin); `pass_twap==0` always survives. Hard AMM binding: pass_amm.{base,quote}==conditional mint idx0 of (kass_vault,usdc_vault); fail_amm==idx1; pass_amm!=fail_amm. Token movements: both paths resolve_question + redeem the bond's conditional KASS into stake_vault; **disqualify** → kass_fee=bond·1/100→challenger, full challenger_usdc→challenger, bond−kass_fee→bond_pool, surviving_count−1; **survive** → usdc_fee=challenger_usdc·1/100→proposer, remainder→challenger, bond stays. Gated `now ≥ market.twap_end`. 21-account graph at `settle_ix` (`challenge_e2e.rs:847-891`).
- **Rust e2e reference (PORT THIS to RPC):** `challenge_e2e.rs` drives a REAL crank — `build_pool` (`create_amm` initial_obs=quote·1e12/base + `add_liquidity`, `:631-720`), `swap_buy` (SWAP type=0 after warp 5 slots, `:723-752`), `crank_pool` (warp 300 slots ≥150 then `crank_that_twap`, accounts `[amm(w),event_auth,amm_program]`, `:756-769`). Fraud path: neutral pass + 90-USDC BUY on fail + two cranks 300 slots apart → past the 10% margin → disqualify (`:1073-1095`). Honest path: both neutral → survive (`:952-968`).
- **T4 stopped at:** placeholder AMM-owned accounts (`fabricateAmmOwned`, `challenge-market-e2e.test.ts:488-497`); no real pools, settle never called.
- **SDK:** `settleChallenge` + `openChallenge` builders EXIST (`sdk/src/instructions/challenge.ts`); conditional_vault builders exist (`sdk/src/futarchy/instructions.ts`). MISSING: v0.4-AMM builders (`create_amm`/`add_liquidity`/`swap`/`crank_that_twap`) — only in Rust CPI today.

## Tasks

### CS1 — SDK builders for the v0.4 standalone AMM
- Add TS builders (web3.js v3) for the 4 v0.4 AMM instructions — `createAmm`, `addLiquidity`, `swap`, `crankThatTwap` — under a dedicated module (e.g. `sdk/src/amm-v04/` or extend an existing MetaDAO module; pick + document), exported from the barrel. Mirror EXACTLY: the discriminators + arg layouts in `cpi/metadao.rs:78-94`, and the account orderings proven in `challenge_e2e.rs:631-769` (create_amm `:676-689`, add_liquidity `:703-714`, swap `:723-752`, crank `:756-769`). Add PDA derivers: amm `[b"amm__", base, quote]`, lp_mint `[b"amm_lp_mint", amm]`, the AMM vault ATAs (ATAs of the AMM PDA). Reuse the SDK's existing builder/PDA conventions.
- Byte-layout unit tests: assert each builder's data == `[disc, ...args]` + the account metas/roles for representative cases, against the known discriminators/seeds. (Offline, default suite.)
- `cd sdk && pnpm typecheck && pnpm test` (default offline green incl. the new tests). Commit `feat(sdk): MetaDAO v0.4 standalone AMM builders (create/add-liquidity/swap/crank-twap)`.

### CS2 — surfpool settle_challenge E2E, both arms (the headline) + docs
- Extend `sdk/test/surfpool/challenge-market-e2e.test.ts` (or a sibling gated test): on forked mainnet, drive the existing open-challenge composition, then REPLACE the `fabricateAmmOwned` placeholders with REAL pools:
  - **Build the markets:** `createAmm` for pass (base=conditional-KASS idx0, quote=conditional-USDC idx0) + fail (idx1) on the conditional mint pairs; `addLiquidity` to seed both. (Port `build_pool` — initial obs = quote·1e12/base; the conditional KASS/USDC come from `split_tokens`.)
  - **DISQUALIFY arm (challenge succeeds):** leave the pass pool neutral, `swap` a BUY on the FAIL pool to push fail price up, then `crankThatTwap` twice spaced ≥150 slots via `surfnet_timeTravel` (mirror `crank_pool` warp 300) so `fail_twap·DEN > pass_twap·(DEN+NUM)` (past the 10% margin). Advance past `market.twap_end`. `settleChallenge` → assert: question resolved `[0,1]`, `kass_fee` (bond·1/100) → challenger KASS, full `challenger_usdc` escrow → challenger, `bond − kass_fee` → bond_pool (the proposer's slashed_amount), `surviving_count − 1`, the bond's conditional KASS redeemed into stake_vault. Decode the on-chain accounts over RPC to assert.
  - **SURVIVE arm (challenge fails):** both pools neutral (pass_twap==0 or within margin) → `settleChallenge` → assert: question `[1,0]`, `usdc_fee` (challenger_usdc·1/100) → proposer USDC, remainder → challenger, bond stays the proposer's (claimable). 
  - GENUINE real crank (no seeded/forced TWAP) — this is the point. If a specific step proves intractable on the fork AFTER a real attempt (e.g. the v0.4 AMM observation won't accumulate, or a pool/vault PDA derives differently on the deployed binary than the fixture), STOP and report the exact blocker (program error + what you tried) — do NOT fake the TWAP. Keep the default `pnpm test` offline + green; this is gated (`KASSANDRA_E2E=1`).
- Docs: update `sdk/test/surfpool/README.md` covered-vs-deferred — `settle_challenge` (the swap-driven v0.4 AMM TWAP) now COVERED end-to-end (both arms); only the Meteora DAMM v2 spot path remains deferred (undeterminable offsets). Append the final note to this plan.
- Commit `test(e2e): settle_challenge end-to-end on forked MetaDAO (real v0.4 AMM TWAP, both arms)`.

## Out of scope / deferred
- Any program change (settle_challenge is complete).
- Meteora DAMM v2 spot-path builders (undeterminable zero-copy offsets — stays deferred).
- The other deferred items (dust sweeping #3; SDK/runner integration #4) — separate milestones.

## Execution note
After each task: default `pnpm test` stays offline + green; the gated suite spawns surfpool (fork, network). CS1 (the v0.4 AMM builders) is the prerequisite; CS2 is the real-crank settle E2E (genuine attempt; stop-and-report a real blocker, the v0.4 wire formats are known so it should be tractable per G3's precedent). No program change. Append a CS1/CS2 delta log here.

## Delta log

### CS1 — v0.4 standalone AMM SDK builders (DONE)

New module `sdk/src/amm-v04/` (`constants.ts`, `pda.ts`, `instructions.ts`, `index.ts`), re-exported from the barrel as `ammV04.*` (`sdk/src/index.ts`). Byte-layout tests in `sdk/test/amm-v04.test.ts` (13 tests, offline). Default `pnpm test` green: 101 passed (after `cargo build-sbf` for the litesvm acceptance tests' `.so`). `pnpm typecheck` clean. Program/runner untouched.

All layouts derived from the binary-validated `cpi/metadao.rs:82-94` (discriminators + arg layouts) and the real-`metadao_amm.so`-proven `challenge_e2e.rs:641-769` (account orderings + PDA seeds) — nothing guessed. Verified layouts:

- **`createAmm`** — disc `f25b15aa05447d40` (`metadao.rs:82`). Args (Borsh, 40 B): `twap_initial_observation:u128 ++ twap_max_observation_change_per_update:u128 ++ twap_start_delay_slots:u64` (`metadao.rs:78-82`). Accounts `[payer(ws), amm(w), lp_mint(w), base_mint, quote_mint, vault_ata_base(w), vault_ata_quote(w), ata_program, token_program, system_program, event_authority, amm_program]` (`challenge_e2e.rs:676-689`).
- **`addLiquidity`** — disc `b59d59438fb63448` (`metadao.rs:85`). Args: `quote_amount:u64 ++ max_base_amount:u64 ++ min_lp_tokens:u64` (`metadao.rs:83-85`). Accounts `[payer(ws), amm(w), lp_mint(w), user_lp(w), user_base(w), user_quote(w), vault_ata_base(w), vault_ata_quote(w), token_program, event_authority, amm_program]` (`challenge_e2e.rs:703-714`). user_* are payer ATAs.
- **`swap`** — disc `f8c69e91e17587c8` (`metadao.rs:90`). Args: `swap_type:u8 (Buy=0/Sell=1) ++ input_amount:u64 ++ output_amount_min:u64` (`metadao.rs:88-90`). Accounts `[payer(ws), amm(w), user_base(w), user_quote(w), vault_ata_base(w), vault_ata_quote(w), token_program, event_authority, amm_program]` (`challenge_e2e.rs:736-749`).
- **`crankThatTwap`** — disc `dc6419f9005cc3c1` (`metadao.rs:94`), NO args. Accounts `[amm(w), event_authority, amm_program]` (`challenge_e2e.rs:760-769`).

PDA/ATA derivers (`amm-v04/pda.ts`): `amm = [b"amm__", base, quote]`, `lpMint = [b"amm_lp_mint", amm]`, `eventAuthority = [b"__event_authority"]` (all under `AMMyu265…`), and `ata(owner, mint)` = SPL ATA `[owner, TOKEN_PROGRAM, mint]`; the AMM vaults are `ata(amm, base/quote)` (`challenge_e2e.rs:641-650`). Nothing undeterminable — no stop-report.

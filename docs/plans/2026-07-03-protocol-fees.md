# Protocol Fees (KASS → futarchy) Implementation Plan

**Goal:** A protocol fee, denominated in KASS, paid to the KASS futarchy. Volume-proportional and **non-bypassable**, realized as a `fee_bps` cut of each market's **LP-fee earnings** — the market PDA holds the LP position that earns the MetaDAO AMM's swap fee on *every* trade (including trades made directly against the public AMM, which a naive trade-wrapper fee could not capture). Respects the "program is never in the trading hot path" architecture — the fee is taken at `resolve_market`, which the program already mediates.

**Why not a trade-wrapper fee:** the MetaDAO AMM is a shared public program; a user can call its `swap` directly, bypassing any `buy`/`sell` wrapper we add. Only operations our program solely gatekeeps (create/contribute/activate/resolve/claim) can enforce a non-bypassable fee. The LP position we control earns fees from all volume — so cutting *that* is both volume-proportional and unavoidable.

**Decision flagged for confirmation:** mechanism = LP-earnings cut at resolution (chosen while user away; confirm before Part 2). Sub-choice within it (Part 2) also flagged: cut of *accrued* LP value (fairer, tiny extra math) vs *total* LP value (simpler). Default: **accrued** (`fee = max(0, realized_kass − total_contributed) × fee_bps / 10000`).

---

## Part 1 — Governed fee config (FOUNDATION — safe, build now)

Prerequisite for any fee mechanism; low-risk; correct regardless of Part 2.

### Program (`programs/kassandra-market`)
- **`Config` gains two fields** (`src/state.rs`): `fee_bps: u16` and `fee_destination: Pubkey` (a KASS token account the futarchy governs; fees land here). Re-lay out packed to an 8-byte multiple and RE-PIN in `tests/state_layout.rs` (Config LEN grows from 88; keep the existing field offsets `authority@8/kass_mint@40/min_liquidity@72` unchanged, append the new fields after `bump`). Add a `MAX_FEE_BPS` const (e.g. `1000` = 10%) as a governance guardrail.
- **`init_config`** (`processor/init_config.rs`): payload gains `fee_bps: u16` + `fee_destination: Pubkey` (extend PAYLOAD_LEN). Validate `fee_bps <= MAX_FEE_BPS` else a new `MarketError::InvalidFee` (Ix error 17). Validate `fee_destination` is an SPL token account owned by the token program whose mint == the config kass_mint (read the token account's mint @0, assert == kass_mint) else `InvalidAccount`. Stamp both into Config.
- **`update_config`** (`processor/update_config.rs`, futarchy-gated already): allow updating `fee_bps` (+ `min_liquidity` as today) and `fee_destination`. Keep the `authority == config.authority` gate. Re-validate `fee_bps <= MAX_FEE_BPS` and the destination mint. (Extend the payload; keep it a single futarchy-gated setter.)
- **`create_market`**: snapshot `fee_bps` from Config into the `Market` at creation (config-as-state, mirroring `min_liquidity`) so an in-flight market's fee is immune to mid-life governance changes. Add `fee_bps: u16` to `Market` (re-pin its LEN/offsets; append after `settled`). `fee_destination` stays read from Config at resolve time (it's a routing target, fine to be current).
- **Tests** (`tests/`): `state_layout` (new Config/Market LEN + offsets); `init_config` (fee stored; `InvalidFee` when > MAX; `InvalidAccount` when fee_destination mint ≠ kass); `update_config` (futarchy updates fee_bps/destination; stranger rejected; > MAX rejected); `create_market` (snapshots fee_bps).

### sdk-rs + TS SDK + app
- `sdk-rs/src/ix.rs`: `init_config`/`update_config` builders gain `fee_bps` + `fee_destination` args. `sdk-rs` Config decode (if any) + a `MAX_FEE_BPS` const.
- TS SDK: `decodeConfig` reads `feeBps` + `feeDestination`; `decodeMarket` reads `feeBps`; `initConfig`/`updateConfig` ix builders gain the args; re-pin `ACCOUNT_SIZES.Config`/`.Market` + parity test (new LENs, `InvalidFee=17`). Keep the LiteSVM round-trip green.
- App: the CreateMarket / config surfaces don't need fee inputs from users (fee is governance-set), but the market detail can DISPLAY the market's `feeBps` (a "protocol fee: X%" line) and the config's fee destination. Add a small read-only display; no new write UI (fee is set by futarchy governance, out of the app's scope).

**Part 1 acceptance:** all existing tests green with the re-pinned layouts; new fee-config tests green; SDK parity + LiteSVM green; app typecheck/build green. Commit `feat: governed KASS protocol-fee config (fee_bps + fee_destination)`.

---

## Part 2 — LP-earnings cut via a `collect_fee` crank (CONFIRMED: accrued-only)

**Confirmed design:** a **separate permissionless `collect_fee` instruction** (Ix 9), not a bloated `resolve_market`. Keeps resolve lean, isolates the heavy CPIs + accrued math + big account list, and lets `claim_lp` gate on collection so ordering is forced: **resolve → collect_fee → claim_lp**. Accrued-only, computed **analytically** (read reserves + LP supply — no full-pool unwind, no claim redesign).

### State
- `Market` gains `fee_collected: u8` (a flag; fits the existing tail padding — re-pin). 
- `MarketError::FeeNotCollected = 18`.

### `resolve_market` (minimal change)
Unchanged except: after setting Resolved/Void + settled, if `fee_bps == 0 || lp_total == 0`, set `fee_collected = 1` (nothing to collect). Otherwise leave it 0 (the crank must run). No new accounts.

### Program CPI helpers (`src/cpi/metadao.rs`)
Add `remove_liquidity_data(lp_amount, min_base, min_quote)` + the AMM remove_liquidity account order, and `redeem_tokens_data()` (disc-only) + the vault redeem_tokens (InteractWithVault) order. Verify byte-exact vs the deployed programs / the SDK metadao. Add readers for the AMM LP mint supply (SPL mint supply @36) if not present.

### `collect_fee` (Ix 9) — permissionless crank, idempotent
Guards: `load_market`; `status ∈ {Resolved, Void}` else `NotActive`/a suitable error; `fee_collected == 0` else `AlreadySettled`-style (or `FeeNotCollected` inverse — use a clear reject); `fee_bps > 0 && lp_total > 0` (else resolve already set the flag). `assert_key(config)`; `fee_destination == config.fee_destination` and it is a KASS token account. Verify the amm/lp_mint/lp_vault/cyes/cno/vault/mints against the recorded `Market` bindings.

**Accrued math (u128, floor, conservative):**
1. Read resolved numerators `(num0, num1, denom)` from the `Question` account (@76/@80/@84).
2. Read pool reserves from `amm`: `base_amount`(cYES)@115, `quote_amount`(cNO)@123.
3. Read AMM LP mint supply from `lp_mint` @36.
4. `pool_value = (base·num0 + quote·num1) / denom`  (full pool KASS value at resolution).
5. `realized_full = lp_total · pool_value / amm_lp_supply`  (the market LP position's KASS value).
6. `accrued = realized_full.saturating_sub(total_contributed)`. If 0 → set `fee_collected=1` and return (no fee; impermanent loss case).
7. `accrued_lp = lp_total · accrued / realized_full`  (LP tokens representing accrued value).
8. `fee_lp = accrued_lp · fee_bps / 10000`. If 0 → set flag, return.

**Realize the fee slice:**
9. Program-signed **remove_liquidity**(`fee_lp`, min_base 0, min_quote 0) from `lp_vault` (authority = market PDA, seeds `[b"market", oracle, [bump]]`) → market-PDA-owned `cyes`/`cno` holders (the `[b"cyes"|b"cno", market]` PDAs created at activate — reuse; they're empty).
10. Program-signed **redeem_tokens** against the resolved `Question` → KASS into the market-PDA-owned KASS account (reuse `escrow_vault`, drained since activate).
11. Program-signed **Transfer** that KASS `escrow_vault → config.fee_destination`.
12. `market.lp_total -= fee_lp`; `market.fee_collected = 1`. Write once.

### `claim_lp` (gate)
Add: require `market.fee_collected == 1` else `FeeNotCollected`. (Forces collect_fee before any claim, so lp_total is final before pro-rata distribution.) Everything else unchanged.

### SDK + app
- sdk-rs + TS SDK: `collectFee` builder (+ `IX_COLLECT_FEE=9`, `Ix::CollectFee`), a `flows.collectFeeInstruction(marketRefs)` assembling the accounts from Market bindings, decodeMarket reads `feeCollected`, parity + LiteSVM.
- App: a permissionless "Collect protocol fee" crank control on market detail (shown Resolved/Void && !feeCollected && feeBps>0), and `ClaimLpControl` shows "waiting for fee collection" until feeCollected. `MarketActions` wires it.

### Tests
- LiteSVM (MetaDAO fixtures): fund→activate→**a real swap to grow the pool**→resolve→collect_fee → assert `fee_destination` KASS ≈ accrued×fee_bps, `lp_total` reduced, `fee_collected==1`; claim_lp works AFTER (and errors `FeeNotCollected` before); winner still redeems. `fee_bps==0` market → resolve sets fee_collected, collect_fee is a no-op/reject, claim works. Impermanent-loss case (accrued==0) → no fee, flag set. Void path.
- surfpool e2e: extend the lifecycle — init_config with fee_bps>0, a real swap on the fork to accrue fees, resolve → collect_fee → assert the futarchy KASS destination received the cut.

**Part 2 acceptance:** new collect_fee + gating tests green; full LiteSVM + surfpool lifecycles green with a non-zero fee; two-stage review (fund-custody + wire-format critical — program-signed remove/redeem/transfer + the accrued math). Commit `feat: protocol fee — collect_fee crank cuts accrued LP earnings to futarchy`.

---

## Deferred / not doing
- Trade-wrapper `buy`/`sell` fee (bypassable; rejected in favor of the LP cut). Could be added later as an *additional* best-effort app-routed fee if desired.
- Per-contribution / activation / creation fees (other non-bypassable points not chosen).

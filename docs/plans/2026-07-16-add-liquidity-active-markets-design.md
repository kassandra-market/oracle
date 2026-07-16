# Add liquidity to active markets — design

**Date:** 2026-07-16
**Status:** implemented (branch `feat/add-liquidity-active-markets`)

**Delivered:** on-chain `add_liquidity` (Ix 11) + gross-LP `claim_lp`/state changes
(program tests incl. an exact fairness proof + fee-path consistency), Rust & TS
SDK builders/flow, indexer DTO fields, an `AddLiquidityControl` in the Active
markets' Liquidity tab, AND bulk add across an oracle's sibling group
(`GroupLiquidityPanel` uniform-splits the total across every depositable outcome —
Funding → contribute, Active-with-reserves → add_liquidity).

**Goal:** let anyone deposit KASS into a market that is already `Active` (its
cYES/cNO AMM is live), receiving pooled LP that is claimable pro-rata alongside
the original funders.

## 1. Why this is not a UI change

`contribute` is hard-gated to `Funding` (`contribute.rs:44`). Once a market is
`Active`, the pool was seeded once at `activate` and there is **no instruction to
add more liquidity**. So this needs a new on-chain `add_liquidity` instruction
(`Ix::AddLiquidity = 11`, append-only), plus SDK, indexer, and UI wiring.

## 2. The three coupled invariants a live-pool add must respect

1. **Pool ratio.** At `activate` the pool is empty, so it seeds a clean 50/50.
   Once `Active`, trading has skewed the cYES/cNO ratio. A conditional-vault
   `split` always yields **1:1** cYES/cNO, which no longer matches the pool.
2. **Claim basis.** `claim_lp` currently pays
   `lp_total × contribution.amount / total_contributed` — **KASS-proportional**.
   That is only fair when every LP token was minted at one rate (the single
   `activate` seed). A late deposit mints LP at a *different* rate, so
   KASS-proportional claim mis-distributes. Worked example (no fees, pool moved
   from 50/50 to 80:125 by trading): a late 100-KASS LP would claim ~10% of the
   early LPs' capital. **Claim must become LP-proportional.**
3. **Fee math.** `collect_fee` computes `accrued = pool_value − total_contributed`
   and skims `fee_bps` of it off `lp_total`. It relies on `total_contributed`
   being the KASS principal behind `lp_total`, and on the market being the **sole
   LP holder** (`lp_total == LP-mint supply`).

## 3. Chosen approach: balanced add, return the remainder (no swap)

Decided with the user. We do **not** add a swap CPI (avoids slippage/MEV surface
and extra compute). The flow is **program-signed, mirroring `activate`** (the
Market PDA is the authority for both MetaDAO CPIs), which reuses the exact,
already-proven `activate` account wiring and sidesteps any question of whether
MetaDAO accepts a non-authority `user_lp`:

1. Depositor-signed SPL `Transfer` of `amount` KASS from the depositor's KASS ATA
   into `escrow_vault` (the depositor is a signer on the ix).
2. **Program-signed** `conditional_vault::split_tokens(amount)`: `escrow_vault →
   market_cyes` / `market_cno` — byte-identical to `activate`'s split (authority =
   Market PDA, source = escrow). This drains escrow back to empty, preserving
   `collect_fee`'s "escrow empty since activate" invariant.
3. **Program-signed** `amm::add_liquidity` at the live ratio — byte-identical
   account order to `activate` (`user_base = market_cyes`, `user_quote =
   market_cno`, `user_lp = lp_vault`, authority = Market PDA). Deposits the
   ratio-limited amounts; LP mints into `lp_vault`.
4. **Program-signed** SPL `Transfer` of the leftover heavy-side balance from
   `market_cyes`/`market_cno` back to the depositor's cYES/cNO ATA (idempotent
   create-ATA first). This returns `market_cyes`/`market_cno` to **0**, preserving
   `collect_fee`'s assumption that those holders are empty until it runs.

Client passes `quote_amount = min(amount, floor(amount · reserveQuote /
reserveBase))` (with a small headroom — the AMM rounds the derived base up) and
`max_base_amount = amount` so the CPI never needs more of either side than was
split; any shortfall just enlarges the returned remainder (never a revert, never
a loss). MetaDAO **requires a non-zero `min_lp_tokens`** for a non-empty pool, so
it is a 4th payload field (client-computed slippage floor); the 32-byte payload is
`amount ++ quote_amount ++ max_base_amount ++ min_lp_tokens` (4 × u64 LE).

**Trade-off (documented):** for a skewed pool the depositor gets back one-sided
conditional tokens they can only realize by trading/redeeming. This is the price
of skipping the swap. The UI must show the expected deployed vs. returned split.

## 4. Accounting model (the core of the design)

Source of truth becomes **gross LP tokens**, not KASS.

### State changes

`Market` — add three `u64` (into existing padding / by growing the struct;
pre-launch so no migration, see §7):
- `activation_lp` — LP minted at `activate` (frozen). Basis for funders' shares.
- `activation_contributed` — `total_contributed` at `activate` (frozen).
- `gross_lp_total` — activation LP + Σ late LP (frozen; **not** reduced by fee).
  The denominator for pro-rata claims.

Repurposed existing fields (semantics widen, activation-only markets unchanged):
- `lp_total` — total gross LP **currently claimable** (activation + late), reduced
  by `collect_fee`'s `fee_lp`. Still `== LP-mint supply` because all LP stays in
  `lp_vault`.
- `total_contributed` — total KASS principal (activation + late).

`Contribution` — add one `u64`:
- `late_lp` — this contributor's LP minted by post-activation `add_liquidity`
  (0 for pure funders).

### add_liquidity (Ix 11) effects, after the CPIs land

Let `lp_new = lp_vault balance delta` across the `add_liquidity` CPI.
- `lp_total += lp_new`; `gross_lp_total += lp_new`.
- `total_contributed += amount` (the full split; **conservative** — see below).
- `contribution.late_lp += lp_new`; create the Contribution if absent and, when
  created, `open_contributions += 1` (mirrors `contribute`'s create-vs-top-up).

**Why `total_contributed += amount` (not net principal):** the returned remainder
is one-sided conditional tokens with no clean KASS value until resolution. Adding
the full `amount` overstates principal by the remainder's value, which *understates*
`accrued` and therefore *under-collects* the protocol fee — strictly conservative
(matches `collect_fee`'s existing floor/undercharge philosophy), never a safety
issue, and it does **not** touch claim fairness (claims use gross LP, not
`total_contributed`). Documented revenue leak, acceptable for v1.

### claim_lp (changed)

Per-contribution gross LP:
```
gross_lp_i = activation_lp · contribution.amount / activation_contributed
           + contribution.late_lp
```
Distribute the post-fee vault pro-rata by gross LP:
```
share_i = lp_total(post-fee) · gross_lp_i / gross_lp_total     (non-last claimer)
```
The last claimer (`open_contributions == 1`) still sweeps the entire remaining
`lp_vault` balance, absorbing floor dust. For an activation-only market
(`late_lp == 0`, `gross_lp_total == activation_lp`, `activation_contributed ==
total_contributed`) this reduces **exactly** to today's formula.

### collect_fee (mostly unchanged)

Reads `supply` from the LP mint and `lp_total`/`total_contributed` from the
market; because all LP is in `lp_vault`, `lp_total == supply` still holds and
`realized_full == pool_value`. `accrued = pool_value − total_contributed` now
nets late principal too. It still reduces `lp_total` (not `gross_lp_total`) by
`fee_lp`, so the fee is borne by **all** LPs proportionally — no late-LP fee
escape. No structural change beyond the widened `total_contributed`.

## 5. Guards & security for add_liquidity

- Status **must be `Active`** (reject Funding/Resolved/Void/Cancelled).
- Oracle **must be non-terminal** (mirror `activate`): once the oracle can
  resolve, no new liquidity (avoids adding into a market about to settle).
- Re-verify every recorded MetaDAO binding against `market.*` (question, vault,
  mints, amm, lp_mint, lp_vault, amm vaults, event authorities) exactly as
  `collect_fee` does — the instruction trusts only the Market record.
- `amount > 0`; typed `ValidationError`.
- The two MetaDAO CPIs are **program-signed with the Market seeds and use the
  identical account order as `activate`** (§3), so no new MetaDAO-authority
  question arises — decision B is dissolved by construction.
- The depositor-signed KASS transfer into escrow (step 1) uses the depositor's
  ATA as source with the depositor as authority; the return transfer (step 4) is
  Market-PDA-signed. Create the depositor's cYES/cNO ATAs idempotently.

## 6. Off-chain changes

- **SDK (`sdks/markets/{rust,ts}`):** `add_liquidity` ix builder + discriminant;
  TS helper computes `quote_amount`/`max_base` from live AMM reserves.
- **App:** new `addLiquidity.ts` action (mirrors `contribute.ts`); surface it in
  `MarketActions.tsx` under the `Active` arm (today only `ClaimLpControl`) and in
  `GroupLiquidityPanel.tsx` (bulk deposit across Active siblings, reusing the
  uniform-split UX). Show deployed-vs-returned preview.
- **Indexer:** decode Ix 11; keep `lp_total`/`total_contributed`/`gross_lp_total`
  and per-contribution `late_lp` in the market/contribution projections and the
  JSON the app reads.

## 7. Migration

**Resolved: pre-launch.** No live Active-phase markets depend on the current
layout, so we grow `Market` and `Contribution` directly (append the new `u64`s,
adjust `_pad`, update `state_layout.rs`) — no versioning or migration crank.

## 8. Testing plan (TDD)

LiteSVM program tests, extending `tests/lifecycle_active.rs` / new
`tests/add_liquidity.rs`:
1. Add to a balanced (untraded) Active pool → LP minted, no remainder, claim
   parity with a pure-funding market.
2. Add to a **skewed** pool → remainder returned to depositor; deployed side
   matches ratio; gross-LP accounting exact.
3. **Fairness:** funder + late LP, pool moved by trades → each claims their true
   gross-LP share (the §2.2 example, asserted to the base unit).
4. Fee interaction: `collect_fee` after a late add nets late principal;
   `lp_total` reduced correctly; all LPs share the fee.
5. Guards: reject on Funding/terminal-oracle/zero-amount; re-verify bindings.
6. `state_layout.rs` + SDK parity for the new fields.
7. `just build` before `cargo test` (LiteSVM `include_bytes!` the `.so`).

## 9. Decisions

- **A. Field layout / migration (§7):** RESOLVED — pre-launch, grow structs
  directly.
- **B. `user_lp = lp_vault` viability:** DISSOLVED — the add is program-signed
  with the Market PDA as authority using `activate`'s exact wiring (§3), which is
  already proven to mint LP into `lp_vault`.
- **C. Conservative `total_contributed += amount` fee leak (§4):** ACCEPTED for
  v1 (strictly conservative, no safety impact).

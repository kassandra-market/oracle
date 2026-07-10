# Kassandra Market — protocol scenarios

A step-by-step walkthrough of what the on-chain program does in each usage
scenario, for verifying the protocol behaves as expected. Every step cites the
instruction and the guard that enforces it; the **must-fail** scenarios (§7) are
the verification checklist — they're where a bug would let value leak.

> Source of truth: `programs/kassandra-market/src/processor/*.rs`. If this doc and
> the code disagree, the code wins — file a fix.

---

## 1. Actors & objects

- **Creator** — opens a market and seeds the first stake. No special powers after
  creation (rent recipient at close).
- **Contributor** — stakes KASS into a `Funding` market. Recorded in a
  `Contribution` PDA. Later claims LP (if activated) or a refund (if cancelled).
- **Cranker** — anyone. All lifecycle transitions after creation are
  **permissionless** cranks; the program pins every output to recorded state so a
  cranker can never redirect funds to itself.
- **Futarchy authority** (`Config.authority`) — the KASS DAO; the only key that may
  `update_config`.
- **Upgrade authority** — the program's on-chain BPF upgrade authority; the only key
  that may `init_config` (genesis).
- **Kassandra oracle** — external; its terminal `phase` drives resolution.
- **MetaDAO** `conditional_vault` + `amm` v0.4 — the vault/AMM primitives the market
  program composes and program-signs CPIs into.

**State accounts** (first byte = `AccountType` tag; type-confusion is rejected):
`Config` (`["config"]`), `Market` (`["market", oracle, outcome_index]`),
`Contribution` (`["contribution", market, contributor]`).

---

## 2. Lifecycle state machine

```
                    contribute*                         activate                 resolve_market
   [create] ─► Funding ─────────► Funding ─(≥ floor)──► Active ─(oracle terminal)─► Resolved / Void
                 │  (below floor keeps accumulating)       │                              │
                 │                                         │                    collect_fee (cut protocol fee)
    oracle       │ cancel                                  │                              │
    terminal ───►│ (any funding level)                     │                       claim_lp* (LPs paid pro-rata)
                 ▼                                          │                              │
             Cancelled ─ refund* (each contributor) ───────┴──────────────────────► close_market (rent → creator)
```

`*` = repeated per contributor. Status enum: `Funding=0, Active=1, Resolved=2,
Void=3, Cancelled=4`.

**Forced ordering after activation:** `resolve_market → collect_fee → claim_lp →
close_market`. `claim_lp` requires `fee_collected == 1`, which is only set at/after
resolution — so LP shares are always computed off the **post-fee** `lp_total`, and no
LP can escape before the protocol fee is cut.

---

## 3. Governance setup (once)

### 3.1 `InitConfig` (Ix 0) — genesis

**Caller:** the program's upgrade authority (a signer). **Accounts:** config, payer,
kass_mint, fee_destination, system_program, **program_data**.

Steps:
1. Assert payload = `authority[32] ++ min_liquidity[8] ++ fee_bps[2] ++ fee_destination[32]`.
2. `assert_signer(payer)`.
3. **`assert_upgrade_authority(program_data, payer)`** — reads the BPF-Upgradeable-
   Loader `ProgramData` account (pinned to its canonical PDA + loader-owned) and
   requires `Some(upgrade_authority) == payer`. Rejects an immutable program.
4. `fee_destination` account key matches the payload; `fee_bps ≤ MAX_FEE_BPS` (1000
   = 10%); `kass_mint` is SPL-token-owned; `fee_destination` is an SPL account on
   `kass_mint`.
5. Config PDA `["config"]` re-derived; **re-init guard by tag** (already-`Config` →
   `AlreadyInitialized`).
6. Create-or-adopt the account (tolerates a pre-funded singleton — `Transfer` top-up
   if short, then PDA-signed `Allocate` + `Assign`), write `Config`.

**End state:** `Config` exists with the futarchy authority, KASS mint, liquidity
floor, and protocol fee. **The authority in `Config` is a payload arg** — the
deployer bootstraps, then hands governance to the DAO (which may differ from the
upgrade authority).

### 3.2 `UpdateConfig` (Ix 1) — futarchy tuning

**Caller:** `Config.authority`. Sets `min_liquidity`, `fee_bps`, `fee_destination`
(all at once). Guards: signer == `config.authority` else `Unauthorized`; `fee_bps ≤
MAX_FEE_BPS`; new `fee_destination` is an SPL account on `kass_mint`. Does **not**
touch existing markets (they snapshot their fee/floor at creation).

---

## 4. Happy path — a market that funds, activates, resolves, and pays out

### Scenario A: YES wins

**A1. `CreateMarket` (Ix 2).** Creator submits `seed_amount ++ outcome_index`.
- Guards: signer; `kass_mint == config.kass_mint`; `seed_amount > 0`;
  `outcome_index < oracle.options_count`; `oracle.phase < Resolved` (can't open on a
  settled oracle); market PDA is empty (one sub-market per `(oracle, outcome)`).
- Effects: create `Market` (status `Funding`, `fee_bps`/`min_liquidity` snapshot from
  `Config`); create the KASS `escrow` token account (authority = market PDA);
  transfer `seed_amount` into escrow; create the creator's `Contribution`.
  `total_contributed = seed_amount`, `open_contributions = 1`.

**A2. `Contribute` (Ix 3), repeated.** Each contributor adds `amount` KASS.
- Guards: signer; market is `Funding`; `escrow == market.escrow_vault`; `amount > 0`.
- Effects: KASS transferred into escrow; `Contribution` created (or **incremented**
  for a repeat contributor); `total_contributed += amount`; `open_contributions += 1`
  **only for a brand-new contributor** (a top-up doesn't double-count).

**A3. `Activate` (Ix 6)** once `total_contributed ≥ min_liquidity`. Permissionless.
- Guards: market `Funding`; `total_contributed ≥ min_liquidity` else `NotFunded`;
  oracle **non-terminal** (a resolved oracle → `OracleResolved`, take the cancel
  exit); the composed MetaDAO accounts are re-derived + owner-checked + field-bound
  (Question oracle-authority == market PDA; vault underlying == KASS; cYES/cNO mints
  derive from the vault; AMM derives from the mints; **pool must be empty**).
- Effects (all program-signed with the market seeds): create the transient
  Market-PDA-owned `cyes`/`cno`/`lp_vault` token accounts; `split_tokens` the full
  escrow KASS → equal cYES + cNO; `add_liquidity` seeds the pool **50/50** (base ==
  quote == `total_contributed`); measure minted LP → `lp_total`; record all bindings;
  status → `Active`. Escrow is now empty.

**A4. Trading (off the market program).** Users swap cYES/cNO against the MetaDAO
AMM directly. The market program isn't involved in trades; the SDK/app drive these.

**A5. Oracle resolves.** The Kassandra oracle reaches `phase == Resolved` with
`resolved_option`.

**A6. `ResolveMarket` (Ix 8).** Permissionless crank.
- Guards: market `Active`, `settled == 0` (idempotency first → `AlreadySettled`);
  Question bound + vault-owned; cv program + event authority pinned; oracle key ==
  `market.oracle` and terminal else `OracleNotTerminal`.
- Numerator: `resolved_option == outcome_index` → `[1,0]` (**YES pays**), else
  `[0,1]` (**NO pays**); out-of-range option → `InvalidAccount`.
- Effects: program-signed `resolve_question` (market PDA is the resolver); `settled =
  1`; status → `Resolved`. If `fee_bps == 0 || lp_total == 0`, also set
  `fee_collected = 1` (nothing to collect — skip the fee crank).

**A7. `CollectFee` (Ix 9).** Permissionless crank (skipped by A6 for fee-free/no-LP
markets).
- Guards: `fee_collected == 0` (idempotency → `AlreadySettled`); market terminal
  (`Resolved`/`Void`); config + `fee_destination` (on KASS) bound; every MetaDAO
  binding re-verified.
- Accrued math (u128, floor, conservative): `pool_value = (base·num0 + quote·num1)/
  denom`; `realized_full = lp_total·pool_value/supply`; `accrued =
  realized_full − total_contributed` (saturating; **0 on impermanent-loss/no-gain →
  just set the flag**); `accrued_lp = lp_total·accrued/realized_full`; `fee_lp =
  accrued_lp·fee_bps/10000`.
- Effects (program-signed): `remove_liquidity(fee_lp)` → cYES/cNO; `redeem_tokens`
  → KASS into escrow; **transfer the redeemed KASS → `fee_destination`** (the KASS
  futarchy); `lp_total -= fee_lp`; `fee_collected = 1`.

**A8. `ClaimLp` (Ix 7), repeated per contributor.** Permissionless.
- Guards: market activated; **`fee_collected == 1`** else `FeeNotCollected` (this is
  the gate that forces A7 before any LP leaves); `lp_vault` bound; `Contribution`
  belongs to this market; **destination is on `lp_mint` AND owned by
  `contribution.contributor`** (a cranker can't redirect); passed `contributor`
  account == `contribution.contributor` (rent recipient).
- Share: `floor(lp_total · contribution.amount / total_contributed)`; the **last**
  claimer (`open_contributions == 1`) sweeps the entire remaining `lp_vault` (absorbs
  rounding dust so it ends at 0).
- Effects: program-signed LP transfer to the contributor; **close the `Contribution`**
  (rent → contributor; absence == idempotency, so a second claim can't reload it);
  `open_contributions -= 1`.

**A9. `CloseMarket` (Ix 10).** Permissionless rent reclaim.
- Guards: terminal status; if activated, `fee_collected == 1`; **`open_contributions
  == 0`** (every contributor has exited — you can't close out from under someone with
  an unclaimed share) else `ContributionsOpen`; token accounts bound to recorded
  state; rent recipient == `market.creator`.
- Effects: SPL-`CloseAccount` the (0-balance) escrow + cyes/cno/lp_vault (rent →
  creator), then close the `Market` data PDA (rent → creator).

**End state:** every contributor holds their pro-rata LP; the protocol fee is with the
futarchy; all rent is reclaimed; the market is gone.

### Scenario B: NO wins

Identical to A, except A6 selects `[0,1]` because `resolved_option != outcome_index`.
The "losers vs winners nets out at the pool" — redemption value flows through the AMM
reserves, which `collect_fee`'s accrued math reads. LP claims are unchanged in
mechanism.

### Scenario C: Void (`InvalidDeadend`)

A6 sees `oracle.phase == InvalidDeadend` → numerators `[1,1]` (each conditional leg
redeems for half), status → `Void`. A7–A9 proceed as in A (`Void` is terminal and
collectable).

---

## 5. The cancel path — oracle resolves before activation

### Scenario D: never funded, or oracle settles early

**D1.** `CreateMarket` + optional `Contribute` (market stays `Funding`, possibly
below floor).

**D2. `Cancel` (Ix 4).** Permissionless.
- Guards: market `Funding`; oracle **terminal** else `OracleNotTerminal`.
- Effect: status → `Cancelled`. **Admitted at any funding level** — a terminal oracle
  makes `activate` impossible, so even a fully-funded market must be allowed to exit,
  or contributions would be stranded.

**D3. `Refund` (Ix 5), repeated per contributor.** Permissionless.
- Guards: market `Cancelled`; `escrow` bound; `Contribution` belongs to this market;
  **destination owned by `contribution.contributor`** (no redirect); passed
  `contributor` == `contribution.contributor`.
- Effect: program-signed transfer of **exactly `contribution.amount`** KASS from
  escrow → the contributor; **close the `Contribution`** (rent → contributor; absence
  == idempotency); `open_contributions -= 1`.

**D4. `CloseMarket` (Ix 10).** As A9, but the `Cancelled` path never activated
(`lp_vault == default`), so no fee gate and only `escrow` is closed. Requires
`open_contributions == 0` (everyone refunded).

**End state:** every contributor's stake fully returned; rent reclaimed. `refund` XOR
`claim_lp` — a `Contribution` is settled by exactly one, and the two paths' status
guards (`Cancelled` vs activated) are mutually exclusive.

---

## 6. Special value cases

- **Fee-free market (`fee_bps == 0`)** — `resolve_market` sets `fee_collected = 1`
  directly; `collect_fee` is a no-op; `claim_lp` opens immediately. LPs get the full
  `lp_total`.
- **No-profit / impermanent-loss market** — `collect_fee` computes `accrued == 0` (or
  `fee_lp` floors to 0) and just sets `fee_collected = 1` without moving value. No fee
  is taken; LPs get the full `lp_total`.
- **Dust contributor** — if `floor(share) == 0`, `claim_lp` skips the transfer but
  still closes the `Contribution` (so it can't wedge a retry loop and can't block
  `close_market`).
- **Donated dust** — KASS a griefer sends into `escrow`, or rounding dust, is swept to
  `fee_destination` by `collect_fee` (harmless: LP claims are off `lp_total`, which
  only decreases by exactly `fee_lp`).

---

## 7. Must-fail scenarios (the verification checklist)

Each should be **rejected** with the noted error. These are the security-relevant
behaviors — assert them explicitly.

| # | Attempt | Rejected because | Error |
|---|---------|------------------|-------|
| F1 | `init_config` signed by a non-upgrade-authority | front-run defense (`assert_upgrade_authority`) | `NotUpgradeAuthority` |
| F2 | Second `init_config` | account already tagged `Config` | `AlreadyInitialized` |
| F3 | `update_config` by a non-authority signer | `signer != config.authority` | `Unauthorized` |
| F4 | `init/update_config` with `fee_bps > 1000` | governance guardrail | `InvalidFee` |
| F5 | `create_market` with `outcome_index ≥ options_count` | invalid outcome | `InvalidOutcome` |
| F6 | `create_market`/`activate` on a resolved oracle | terminal oracle can't open/activate | `OracleResolved` |
| F7 | `create_market` with a non-KASS mint | must match `config.kass_mint` | `WrongMint` |
| F8 | `contribute` / `create_market` with `amount == 0` | zero stake | `ZeroAmount` |
| F9 | `activate` below the floor | `total_contributed < min_liquidity` | `NotFunded` |
| F10 | `activate` into a non-empty pool | front-run of the 50/50 seed | `PoolNotEmpty` |
| F11 | `claim_lp` while `fee_collected == 0` (incl. an `Active` market) | fee gate — LP can't leave pre-fee | `FeeNotCollected` |
| F12 | `claim_lp`/`refund` to a destination not owned by `contribution.contributor` | cranker redirect blocked | `InvalidAccount` |
| F13 | `claim_lp` with a destination not on `lp_mint` | wrong token | `InvalidAccount` |
| F14 | Second `claim_lp`/`refund` for the same `Contribution` | reaped on first (absence == idempotency) | `InvalidAccount` (load fails) |
| F15 | `refund` on a non-`Cancelled` market | wrong lifecycle | `NotCancelled` |
| F16 | `cancel` while the oracle is non-terminal | premature | `OracleNotTerminal` |
| F17 | Second `resolve_market` | idempotency | `AlreadySettled` |
| F18 | `resolve_market` on a non-`Active` market | must be live | `NotActive` |
| F19 | Second `collect_fee` | idempotency | `AlreadySettled` |
| F20 | `close_market` with `open_contributions > 0` | can't strand an unexited contributor | `ContributionsOpen` |
| F21 | `close_market` before resolution/fee (activated) | must be fully settled | `NotSettled` / `FeeNotCollected` |
| F22 | Any instruction with a substituted account (wrong owner / wrong PDA / wrong program id / wrong type tag) | hand-rolled validation | `InvalidAccount` |
| F23 | A program-signed CPI whose MetaDAO account/program is swapped | callee pinned to constant + accounts bound | `InvalidAccount` |

---

## 8. Invariants to assert (any time)

1. **Escrow conservation (Funding):** `escrow balance == total_contributed`.
2. **`open_contributions` == number of live `Contribution` PDAs** for the market; it
   only reaches 0 when every contributor has claimed/refunded.
3. **Fee ordering:** no LP leaves `lp_vault` until `fee_collected == 1`; after
   `collect_fee`, `lp_total` has decreased by exactly `fee_lp`.
4. **LP conservation:** `Σ claimed shares == lp_total` (the last claimer sweeps the
   dust; `lp_vault` ends at exactly 0).
5. **Refund conservation:** `Σ refunds == total_contributed` (each contributor gets
   exactly their recorded `amount`).
6. **Settle exclusivity:** a `Contribution` is settled by `claim_lp` XOR `refund`,
   never both.
7. **No caller-controlled value:** every crank's destination + amount derive from
   recorded state or measured balances, never from caller input.
8. **Rounding direction:** fees round **up** against LPs / payouts round **down** —
   always against the less-trusted party; the protocol never over-distributes.
9. **Close-ability:** a fully-settled market with all contributors exited always has
   0-balance token accounts, so `close_market` never faces an un-closeable balance.

---

## 9. How to exercise these

- **Rust (LiteSVM):** `just test` — `programs/kassandra-market/tests/` covers every
  instruction, the full lifecycle (`lifecycle*.rs`), and the must-fail cases
  (per-instruction `*_rejects_*` tests).
- **TS (LiteSVM + parity):** `pnpm --filter @kassandra-market/markets test`.
- **End-to-end (real local validator + MetaDAO fixtures):**
  `KASSANDRA_MARKET_E2E=1 SURFPOOL_OFFLINE=1 pnpm --filter @kassandra-market/markets exec vitest run test/surfpool/`
  drives compose → activate → trade → resolve → collect_fee → claim_lp against a real
  surfpool node.

Map each scenario above to its test; a gap in §7 is a gap in coverage.

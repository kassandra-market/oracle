# kassandra-market — Design

*2026-07-03*

## Summary

**kassandra-market** is a KASS-denominated, outcome-token AMM prediction market on
Solana (Polymarket-style), built on the [Kassandra](../../../kassandra) optimistic
oracle. Users trade on the outcome of Kassandra oracle questions; markets settle
**trustlessly** against the oracle's on-chain resolution via a thin on-chain resolver
program.

Scope for v1 is **binary markets**. Categorical (N > 2) is deferred.

## Why a program (not just an SDK)

MetaDAO's `conditional_vault` resolves a market through a permissioned
`resolve_question` call from the question's designated **oracle authority**. A trusted
off-chain keeper holding that key would defeat Kassandra's entire premise (an
economically-secured, trustless oracle). So kassandra-market is a small **Pinocchio**
program that owns each market's `Question` as its oracle authority and permissionlessly
bridges the Kassandra `Oracle` result into `resolve_question`. Settlement is trustless
end to end; anyone can crank it.

The program is only in the path at **create / activate / resolve**. All trading happens
directly in the MetaDAO AMM — the program is never in the hot path (cheap, no per-trade
custody).

## Denomination

All markets are denominated in **KASS**: the `conditional_vault` underlying mint is KASS,
so `cYES`/`cNO` are conditional-KASS and winners redeem 1 KASS per winning token. Traders
enter and exit in any token — the **UI routes through Jupiter** (any token → KASS → enter;
redeem KASS → any token). On-chain state is pure KASS. This also concentrates KASS utility.

## Market mechanics (binary)

**Pool shape: a single `cYES`/`cNO` pool** (base = `cYES`, quote = `cNO`). Because a full
set (`cYES` + `cNO`) always redeems for exactly 1 KASS, the pool's spot ratio *is* the
implied probability: `price = cYES / (cYES + cNO)`. This halves the AMM accounts vs.
Kassandra's dual pass/fail-pool futarchy setup.

**Buying YES (split-and-swap):**

1. `split_tokens`: deposit *N* KASS → *N* `cYES` + *N* `cNO`.
2. Swap `cNO` → `cYES` on the pool.
3. Hold `cYES`; if YES wins, each redeems 1 KASS.

Selling is the mirror (swap `cYES` → `cNO`, then `merge` a full set back to KASS). The SDK
wraps this into a single `buy(outcome, amount)` / `sell(...)` helper so users never reason
about conditional tokens.

## Governance config

A governed **`Config` singleton PDA** holds `min_liquidity` (in KASS). Its authority is the
**KASS futarchy DAO** (MetaDAO futarchy v0.6, already integrated by Kassandra). Only
`update_config` carrying the DAO executor authority can change the minimum — so "the
minimum is set by KASS futarchy" is enforced on-chain.

## Market lifecycle

**Funding → Active → Resolved / Void.**

1. **`create_market`** — permissionless, lightweight. Records a `Market` (oracle, opening
   split, creator) and opens a KASS escrow. The creator picks an **opening split** (uneven
   allowed → sets the opening prior) and seeds any amount of KASS, even below
   `min_liquidity`. No MetaDAO accounts are created yet.
2. **`contribute`** — anyone stakes KASS toward `min_liquidity`. Each contributor gets a
   `Contribution` record. No trading yet, so everyone funds at the same pre-pool basis.
3. **`activate`** — permissionless crank once total escrow ≥ `min_liquidity`. The client
   composes the MetaDAO `Question` (oracle authority = Market PDA), `conditional_vault`
   (underlying = KASS), and the `cYES`/`cNO` AMM pool; the program **verifies + records**
   those bindings (Question.oracle == Market PDA, mints/vault correct, oracle
   `prompt_hash` / `options_count` match), then `split`s the escrow per the creator's ratio
   and `add_liquidity`'s it. LP tokens are escrowed to the Market PDA.
4. **`claim_lp`** — each contributor claims LP tokens **pro-rata** to their stake.
5. **`resolve_market`** — permissionless crank after the oracle resolves.

Deferring the heavy MetaDAO accounts to `activate` means a never-funded market touches
MetaDAO not at all — clean refund, nothing to unwind.

## Resolution & void

Read the `Oracle` account (see the integration surface below):

- **Resolved** (`phase == 7`): CPI `resolve_question` with `[1,0]` / `[0,1]` derived from
  `resolved_option`. Winners `redeem` 1 KASS/token; LPs withdraw then redeem.
- **InvalidDeadend** (`phase == 8`, `resolved_option == 0xFF`): resolve the Question with
  **equal numerators `[1,1]`** — a neutral 50/50 par. This is the standard void: it returns
  the pooled KASS to current holders/LPs using only the fixed-numerator mechanic the vault
  supports. A directional bettor gets 0.5 KASS/token, not their entry price;
  losers-vs-winners nets out **at the pool level** (conventional behavior).
- **Resolves during Funding** (never activated): `cancel` → refund **all** KASS, including
  the creator's seed. There is no forfeiture; unreclaimable account rent is the only spam
  cost.

## Oracle integration surface

Settlement is a two-read contract against the `Oracle` account
(`../kassandra/programs/kassandra/src/state.rs`, TS decoder `sdk/src/accounts/oracle.ts`):

- `phase: u8` @161 — `Resolved = 7` (terminal success), `InvalidDeadend = 8` (void).
- `resolved_option: u8` @197 — winning option index; valid **only** when `phase == 7`;
  `0xFF` on void.
- `options_count: u8` @160 (≥ 2; binary = 2). No on-chain labels — bind a market to an
  oracle via `prompt_hash: [u8;32]` @200.

Reuse Kassandra's proven MetaDAO CPI: `programs/kassandra/src/cpi/metadao.rs` (wire format
for `initialize_question` / `initialize_conditional_vault` / `split` / `merge` / `redeem` /
`resolve_question` + `create_amm` / `add_liquidity` / `swap`) and the `sdk/src/amm-v04/`
builders. Program IDs: conditional_vault `VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg`,
amm v0.4 `AMMyu265tkBpRW21iGQxKGLaves3gKm2JcMUqfXNSpqD`.

## Components (mirrors Kassandra conventions)

- **`programs/kassandra-market/`** — Pinocchio program.
  Instructions: `init_config`, `update_config` (futarchy-gated), `create_market`,
  `contribute`, `cancel`, `activate`, `resolve_market`, `claim_lp`.
  Accounts: `Config` (singleton), `Market`, `Contribution`.
  `#[repr(C)]` Pod + bytemuck accounts, first byte = `AccountType` tag, one processor per
  file, 1-byte instruction discriminant. Reuse `cpi/metadao.rs`.
- **`sdk/`** — hand-written TS client (no IDL): `pda.ts`, `accounts/`, `instructions/`, and
  a `flows/` layer wrapping split-and-swap into `buy` / `sell` plus Jupiter routing. A
  `parity.test.ts` pins layouts to the program. Depends on `@kassandra/sdk` to read the
  `Oracle`.
- **`app/`** — Vite + React: market list (labels resolved off-chain via `prompt_hash`),
  funding-progress UI, trade UI with Jupiter any-token entry.

## Testing

Two layers, as in Kassandra:

- **LiteSVM** (Rust) — unit / invariant / CPI-integration tests, including a
  `state_layout.rs` that pins every account byte offset (the SDK decoder's source of truth).
- **surfpool** (mainnet-fork) — full e2e against the real deployed MetaDAO vault / AMM /
  futarchy programs.

Golden-path e2e: create → contribute → activate → trade → oracle resolves →
`resolve_market` → redeem. Plus the **void** and **cancel** paths.

## Status

As of 2026-07-03, **Phases 1 and 2 are implemented and covered by LiteSVM
integration tests against the real MetaDAO v0.4 vault + AMM fixtures.** Phase 1
is the crowdfunding lifecycle (`init_config`, `update_config`, `create_market`,
`contribute`, `cancel`, `refund`); Phase 2 adds `activate` (splits the escrowed
KASS into cYES/cNO, seeds the 50/50 AMM, mints LP), `claim_lp` (pro-rata LP
distribution to contributors), and `resolve_market` (bridges the terminal
Kassandra oracle result into a program-signed MetaDAO `resolve_question`).
**Binary markets are therefore fully live end-to-end** — fund → activate → trade
→ resolve → redeem — with a comprehensive conservation test asserting no KASS is
minted from nothing (redeemed ≤ deposited; the traded split→redeem round trip is
exact, and LP distribution leaves only bounded floor-division dust, observed 0 on
the balanced seed).

The **TS SDK (`@kassandra-market/sdk`) and the web app (`app/`) are now
implemented.** The app is a full-MVP React 19 + Vite + Tailwind v4 dApp built
entirely on the SDK, covering the whole lifecycle as wallet-signed, phase-gated
actions — fund → activate → trade YES/NO → resolve → redeem, plus create-market
and the permissionless activate/resolve cranks — and read-only browse (markets
list + market detail with live AMM-implied probability). It mirrors the sibling
Kassandra oracle app's "Delphi" design system verbatim (parchment/chestnut
tokens, the same primitives, providers, cluster switcher, and `useWriteAction`
tx idiom). Verification bar met: `tsc` typecheck, `oxlint`, `vite build`, and the
Vitest unit suite (42 tests over the view/format helpers + the ix-builder wiring)
all green, and the production preview serves + renders the shell/landing. A
**not-initialized** empty state surfaces a missing on-chain `Config` per cluster.

Still deferred: uneven opening prior (50/50 only), categorical N>2, protocol fee,
LP / conditional-token account close for rent + dust recovery, a surfpool
mainnet-fork e2e (the LiteSVM fixtures suffice for logic), the app's **live-cluster
e2e click-through** (needs a deployed program + a seeded market — a manual next
step), and the app's **live Jupiter** any-token entry (the SDK exposes the entry
helpers; the HTTP quote/swap wiring is stubbed behind a disabled toggle).

## Deferred (YAGNI)

- **Categorical (N > 2)** — v1 is binary-only; later, *N* independent outcome pools.
- **Protocol fee switch** — inherit the AMM's LP fee for now.
- **Off-chain label registry** — start from Kassandra's existing metadata path; add a
  dedicated registry only if needed.

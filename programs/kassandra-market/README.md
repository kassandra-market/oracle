# `kassandra-market-program`

The on-chain program for [Kassandra Market](../../README.md): a KASS-denominated
AMM prediction market resolved by the Kassandra oracle.

Written in **[Pinocchio](https://github.com/anza-xyz/pinocchio) (no Anchor)** —
zero-copy `#[repr(C)]` + `bytemuck` account layouts, a 1-byte instruction
discriminant, and hand-built CPIs. Because there is no framework, **every account
is validated by hand** (owner + PDA + type-tag + signer checks); that discipline is
the security model (see [Security](#security)).

Program id: `FEGNHWAB7kc7VC9CCwbvVPsv4Jykz2r2WQ758V4xCT9S`

## Design: a thin wrapper over MetaDAO + the Kassandra oracle

The program does **not** re-implement conditional-token or AMM math. It:

1. **verifies** externally-composed MetaDAO accounts (re-derives every PDA,
   owner-checks, binds recorded fields), then
2. **program-signs** the value-moving CPIs into MetaDAO's v0.4 `conditional_vault`
   (`VLTX…`) + `amm` (`AMMyu…`) — split, add/remove liquidity, redeem, resolve —
   with the market PDA as authority, and
3. **defers resolution** to the Kassandra oracle: the market PDA is the MetaDAO
   `Question`'s oracle-authority, so `resolve_market` bridges a terminal oracle
   result into `resolve_question`.

Each market is one **binary sub-market** per oracle outcome (PDA
`["market", oracle, outcome_index]`); a categorical oracle → N independent binary
markets.

## State accounts (zero-copy, first byte = `AccountType` tag)

| Account | PDA seeds | Holds |
|---------|-----------|-------|
| `Config` | `["config"]` | governance authority, KASS mint, min-liquidity floor, protocol `fee_bps` + destination |
| `Market` | `["market", oracle, outcome_index]` | lifecycle status, escrow, totals, the recorded MetaDAO bindings (question/vault/mints/amm/lp), `fee_bps` snapshot |
| `Contribution` | `["contribution", market, contributor]` | one contributor's staked KASS (reaped on claim/refund) |

`Config`/`Market` snapshot the fee + liquidity floor at creation, so in-flight
markets are immune to later governance changes.

## Instructions

| # | Instruction | Who | Effect |
|---|-------------|-----|--------|
| 0 | `InitConfig` | upgrade authority | create the `Config` singleton |
| 1 | `UpdateConfig` | `config.authority` (futarchy) | set fee / destination / min-liquidity |
| 2 | `CreateMarket` | anyone | open a `Funding` market + escrow, seed the creator's stake |
| 3 | `Contribute` | anyone | add KASS to a `Funding` market's escrow |
| 4 | `Cancel` | anyone (crank) | mark a `Funding` market `Cancelled` once its oracle is terminal |
| 5 | `Refund` | anyone (crank) | return a contributor's stake from a `Cancelled` market |
| 6 | `Activate` | anyone (crank) | compose→verify MetaDAO, split escrow → cYES/cNO, seed the AMM 50/50, go `Active` |
| 7 | `ClaimLp` | anyone (crank) | pro-rata LP payout to a recorded contributor (after fee collection) |
| 8 | `ResolveMarket` | anyone (crank) | bridge the terminal oracle result into `resolve_question` |
| 9 | `CollectFee` | anyone (crank) | cut the protocol's `fee_bps` of accrued LP earnings → KASS futarchy |
| 10 | `CloseMarket` | anyone (crank) | reclaim rent once fully settled + every contributor has exited |

Ordering after resolution is forced by gates: **resolve → collect_fee → claim_lp →
close** (`claim_lp` only opens once `fee_collected == 1`, so LP shares are computed
off the post-fee total).

## Build & test

```bash
just build     # cargo build-sbf → ../../target/deploy/kassandra_market_program.so
just test      # build + cargo test -p kassandra-market-program
```

Tests are LiteSVM integration tests (`tests/`) that `include_bytes!` the built
`.so` and load the vendored MetaDAO `.so` fixtures (`tests/fixtures/`), covering
every instruction + the full lifecycle, plus `state_layout` offset-pin and `parity`
(vs the Rust SDK) tests. The Kassandra-oracle layout the program reads is vendored
in [`src/kass_oracle.rs`](src/kass_oracle.rs), so no sibling repo is needed.

## Security

Reviewed against the [Solana Security Standard](https://github.com/Copenhagen0x/solana-security-standard).
Key properties:

- **Hand-rolled validation is complete** — every processor checks owner, re-derives
  and compares PDAs, verifies the `AccountType` tag, and asserts signers; the
  central helpers live in [`src/processor/guards.rs`](src/processor/guards.rs).
- **CPIs are safe** — callee program ids are pinned to constants, accounts are bound
  to recorded state, and the market PDA is the only signer.
- **Permissionless cranks can't redirect funds** — `claim_lp`/`refund`/`collect_fee`
  pin their destination + amount to immutable recorded fields (e.g. the LP goes to
  `contribution.contributor`, the amount is the recorded stake).
- **Idempotent by construction** — settle paths reap the `Contribution` (absence ==
  idempotency); re-init is rejected by the account-type tag.
- **Checked arithmetic** throughout (`checked_*`, `u128` intermediates, conservative
  rounding *against* the less-trusted party).
- **`init_config` is gated to the program's on-chain upgrade authority** (read from
  the BPF-Upgradeable-Loader `ProgramData` account), so an attacker can't front-run
  genesis to seize `Config.authority`.

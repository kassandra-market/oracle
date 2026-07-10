# `kassandra-oracles-program`

The core on-chain program for [Kassandra](../../README.md) — a decentralized, AI-assisted
optimistic oracle on Solana. It owns oracle requests, proposal windows, fact
proposal/voting, the AI-claim registry, plurality computation, slash/recompute, KASS
staking & emissions, and the dynamic creation fee.

- **Program ID:** `KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY` (`src/lib.rs`)
- **Framework:** [Pinocchio](https://github.com/anza-xyz/pinocchio) — **not Anchor**. No
  IDL, manual account deserialization/validation, manual instruction dispatch.
- **Crate type:** `cdylib` + `lib`. Build with `cargo build-sbf`.

MetaDAO's deployed **conditional-vault + AMM** programs are reused for the pass/fail
decision markets via hand-built CPI (`src/cpi/`); the vault/AMM are not reimplemented here.

## Build & test

From the repository root:

```bash
just build     # cargo build-sbf --manifest-path programs/oracles/Cargo.toml
just test      # rebuilds the .so first, then runs the LiteSVM suite
```

`just test` depends on `just build` so the tests never run against a stale `.so`. The tests
under [`tests/`](./tests) are **LiteSVM** unit tests (per-instruction account validation,
phase gating, arithmetic), invariant/fuzz tests (`invariants.rs`), and MetaDAO CPI
integration tests (which load MetaDAO's program binaries — see
[`../../scripts`](../../scripts)).

## Module map (`src/`)

| Module | Responsibility |
| --- | --- |
| `lib.rs` | Entrypoint, program ID, dispatch into `processor::process`. |
| `instruction.rs` | The `Ix` discriminants (0..=22) and payload parsing. |
| `processor/` | One file per instruction handler + shared `guards.rs`. |
| `state.rs` | On-chain account structs, the `Phase` enum, `AccountType`, sentinels. |
| `config.rs` | Protocol constants and governable-field defaults. |
| `error.rs` | `KassandraError` codes (0..=35). |
| `reward.rs` | Emission / reward-bucket math (proposer + fact rewards). |
| `fee.rs` | Dynamic creation-fee EMA (half-life decay + interpolation). |
| `price.rs` | KASS price read from the futarchy DAO TWAP. |
| `plurality.rs` | Strict-plurality / mode computation with tie handling. |
| `clock.rs` | Slot/timestamp helpers. |
| `cpi/` | Hand-built CPI into MetaDAO (`metadao.rs` = v0.4 vault+AMM, `metadao_v06.rs` = v0.6 futarchy/Meteora). |

## Instruction set (`Ix`, `src/instruction.rs`)

23 instructions, discriminant = the first payload byte:

| Ix | Name | Purpose |
| --- | --- | --- |
| 0 | `SubmitFact` | Post a candidate fact (evidence) with a KASS stake. |
| 1 | `VoteFact` | Approve or mark-duplicate a fact by stake. |
| 2 | `FinalizeFacts` | Settle the fact-voting phase incrementally (batched). |
| 3 | `SubmitAiClaim` | Resubmit a value + AI-claim metadata over the agreed facts. |
| 4 | `OpenChallenge` | Open a MetaDAO decision market against an AI claim. |
| 5 | `SettleChallenge` | Read the market TWAP, apply the verdict, resolve/redeem. |
| 6 | `FinalizeOracle` | Compute the final plurality and reach a terminal state. |
| 7 | `AdvancePhase` | Advance the phase when a window's deadline has passed. |
| 8 | `FinalizeAiClaims` | Close the AI-claim window. |
| 9 | `InitProtocol` | Initialize the singleton `Protocol` account. |
| 10 | `CreateOracle` | Create an oracle (immutable config, burn the creation fee). |
| 11 | `Propose` | Submit a categorical value with a KASS bond. |
| 12 | `FinalizeProposals` | Close the proposal window (resolve or open a dispute). |
| 13 | `SetGovernance` | Set the governance authority. |
| 14 | `SetConfig` | Update governable protocol config. |
| 15 | `ResolveDeadend` | Governance resolution of an Invalid dead-end oracle. |
| 16 | `KassPrice` | Refresh the cached KASS price from the DAO TWAP. |
| 17 | `ClaimProposer` | Claim a proposer's reward / bond return. |
| 18 | `ClaimFact` | Claim a fact submitter's reward / stake. |
| 19 | `ClaimFactVote` | Claim a fact voter's reward / stake. |
| 20 | `CloseAiClaim` | Close an `AiClaim` account (reclaim rent). |
| 21 | `CloseMarket` | Close a settled `Market` account (reclaim rent). |
| 22 | `SweepOracle` | Sweep dust / final cleanup on a resolved oracle. |

## Accounts (`src/state.rs`)

Fixed-layout, `bytemuck`-castable structs, tagged by `AccountType` (0..=7). Byte sizes are
pinned by [`tests/state_layout.rs`](./tests/state_layout.rs).

| Account | Size | Role |
| --- | --- | --- |
| `Protocol` | 368 B | Singleton — global config, governance authority, emission state. |
| `Oracle` | 392 B | One request — prompt/interpretation config, phase, tallies, pools. |
| `Proposer` | 96 B | One proposer's value + bond within an oracle. |
| `Fact` | 336 B | One candidate fact — content hash/URI, submitter, stake, votes. |
| `FactVote` | 88 B | One staker's approve/duplicate vote on a fact. |
| `AiClaim` | 208 B | One AI claim — model id, params hash, io hash, option. |
| `Market` | 416 B | One challenge's MetaDAO market state (mints, TWAP, verdict). |

## Phase state machine (`Phase`, `src/state.rs`)

```
Proposal (1) → FactProposal (2) → FactVoting (3) → AiClaim (4) → Challenge (5)
             → Resolved (7)  |  InvalidDeadend (8)
```

`Created (0)` and `FinalRecompute (6)` are reserved discriminants (kept for ABI stability;
no live oracle occupies them — `create_oracle` initializes directly into `Proposal`).

## Reference

The authoritative, cross-linked reference — every instruction, account field, error code,
constant, and PDA — is in the docs site under **Protocol reference**
([`docs-site/protocol/`](../../docs-site/protocol)). The SDK ([`sdks/oracles/ts/`](../../sdk)) mirrors
these layouts in TypeScript.

# `kassandra-market-sdk`

The **Rust SDK** for the [Kassandra Market program](../programs/kassandra-market):
PDA derivation and instruction builders, decoupled from the on-chain crate's
Pinocchio/SBF toolchain so native Rust (the indexer, tests, off-chain tooling) can
build market transactions.

It exposes:

- **`pda`** — every program PDA (`config`, `market`, `escrow`, `contribution`,
  `program_data`) plus the shared program-id constants;
- **`ix`** — builders for all 11 instructions (`init_config` … `close_market`),
  returning `solana_sdk::Instruction`s with the exact account order + wire payload
  the program expects;
- **`metadao`** — the MetaDAO v0.4 `conditional_vault` + `amm` wire constants
  (discriminators, offsets, PDA seeds), re-verified byte-for-byte against the
  program's own copy.

The account order and payload encoding are the source of truth shared with the
program via its `tests/parity.rs`, and mirrored by the TypeScript SDK's
`test/parity.test.ts` — so all three stay in lockstep.

## Use

```toml
[dependencies]
kassandra-market-sdk = { path = "../sdk-rs" }
```

```rust
use kassandra_market_sdk::{ix, pda};

let (config, _) = pda::config();
let ix = ix::init_config(&payer, &kass_mint, &authority, min_liquidity, fee_bps, &fee_destination);
```

## Consumers

- **[`indexer`](../indexer)** — derives PDAs and relays/builds transactions.
- **Program tests** (`programs/kassandra-market/tests`) — build every instruction
  against LiteSVM.

## Test

```bash
cargo test -p kassandra-market-sdk
```

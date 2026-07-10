---
name: kassandra-rust-client
description: "Use when integrating with the Kassandra optimistic-oracle Solana program from Rust - a test harness, an off-chain keeper or bot, or another service that builds a Kassandra instruction, derives a Kassandra PDA, or decodes an on-chain account (oracle, proposer, fact, ai_claim, market). Reach for the kassandra-oracles-sdk crate before hand-rolling account metas, discriminant bytes, or PDA seeds."
---

# Integrating Kassandra from Rust

The `kassandra-oracles-sdk` crate is the Rust client for the Kassandra dispute-oracle program. Its
single source of truth is the on-chain program (it re-exports the discriminants, account
layouts, and constants), so it never drifts. Depend on it (path or git), not on hand-rolled
encoding.

```toml
kassandra-oracles-sdk = { git = "https://github.com/Dodecahedr0x/kassandra", package = "kassandra-oracles-sdk" }
```

## Surface

- **`kassandra_oracles_sdk::PROGRAM_ID`** (a `solana_pubkey::Pubkey`), plus `TOKEN_PROGRAM_ID`,
  `SYSTEM_PROGRAM_ID`, `ATA_PROGRAM_ID`, and the `Ix` discriminant enum.
- **`kassandra_oracles_sdk::ix::*`** — one builder per instruction, returning
  `solana_instruction::Instruction`. Unlike the TS client, these take the account pubkeys
  **explicitly** (derive PDAs yourself via `pda::*`). Examples: `ix::propose`, `ix::create_oracle`,
  `ix::submit_fact`, `ix::vote_fact`, `ix::submit_ai_claim` (+ `submit_ai_claim_raw` for a
  pre-built 97-byte payload), `ix::open_challenge` / `ix::settle_challenge` (take an
  `OpenChallengeAccounts` / `SettleChallengeAccounts` struct), `ix::finalize_*`, `ix::claim_*`,
  `ix::close_*`, `ix::sweep_oracle`.
- **`kassandra_oracles_sdk::pda::*`** — return `(Pubkey, u8)`: `pda::oracle(&PROGRAM_ID, nonce)`,
  `pda::proposer(&PROGRAM_ID, &oracle, &authority)`, `pda::stake_vault`, `pda::fact`, `pda::vote`,
  `pda::ai_claim`, `pda::protocol`, `pda::mint_authority`, `pda::challenge_usdc_vault`, `pda::kass_ata`.
- **`kassandra_oracles_sdk::accounts`** — the layout structs (`Oracle`, `Proposer`, `Fact`, `FactVote`,
  `AiClaim`, `Market`, `Protocol`) + `decode::<T>` (zero-copy, aligned) and `read::<T>` (owned
  copy, unaligned-safe — use this for RPC buffers), plus sentinels `CLAIM_OPTION_NONE`,
  `VOTE_APPROVE`, `VOTE_DUPLICATE`.

## Example

```rust
use kassandra_oracles_sdk::{accounts::{self, Oracle}, ix, pda, PROGRAM_ID};
use solana_instruction::Instruction;
use solana_pubkey::Pubkey;

fn build_propose(oracle: Pubkey, authority: Pubkey, authority_kass: Pubkey, option: u8, bond: u64) -> Instruction {
    let (proposer, _) = pda::proposer(&PROGRAM_ID, &oracle, &authority);
    let (stake_vault, _) = pda::stake_vault(&PROGRAM_ID, &oracle);
    ix::propose(&PROGRAM_ID, oracle, proposer, authority, authority_kass, stake_vault, option, bond)
}

// Decode an Oracle from account bytes fetched over RPC (unaligned-safe).
fn read_oracle(data: &[u8]) -> Result<Oracle, bytemuck::PodCastError> {
    accounts::read::<Oracle>(data)
}
```

## Notes

- Instructions that sign as the oracle PDA (finalize_*, open/settle challenge, claim_*,
  close_market, sweep, and create_oracle's payload) need the oracle **nonce** — the `Oracle`
  struct does not store it, so carry it alongside the oracle pubkey.
- `kassandra-oracles-program` is pulled in transitively (with `no-entrypoint`); you don't depend on it
  directly.
- The TS client (`@kassandra-market/oracles`) mirrors this — see the `kassandra-ts-client` skill.

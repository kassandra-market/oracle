# `scripts/`

Helper scripts for the [Kassandra](../README.md) monorepo. Most are invoked via the
top-level [`Makefile`](../Makefile) (`make help`) rather than directly.

## Dev stack + E2E

| Script | Make target | What it does |
| --- | --- | --- |
| `dev-up.sh` | `make chain` / `make app-local` / `make dev` | Boot a **seeded** local surfpool chain (via `app/e2e/dev-seed.ts`) and/or the app dev server against it. |
| `e2e-playwright.sh` | `make test-e2e` | Browser E2E: surfpool + a script-funded wallet + the app. |
| `e2e-playwright-fork.sh` | `make test-e2e-fork` | Browser E2E over a **mainnet-forked** challenge-market cluster. |
| `e2e-playwright-indexer.sh` | `make test-e2e-indexer` | Browser E2E: surfpool + Postgres + the real indexer + the app ActivityFeed. |

## MetaDAO fixtures

The `fetch-*` scripts dump MetaDAO's on-chain program binaries into the test fixtures
directory so the program's **LiteSVM CPI tests are fully hermetic** — no network access at
test time.

Kassandra reuses MetaDAO's deployed programs (conditional vault + AMM for the v0.4 dispute
core, and the futarchy/Meteora stack for v0.6) via CPI. To exercise those CPI paths in
tests, the real program binaries are fetched once and committed/loaded as fixtures.

## Scripts

| Script | What it fetches |
| --- | --- |
| `fetch-metadao.sh` | The **v0.4** dispute-core stack: `conditional_vault` (`VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg`, v0.4.0) and `amm` (`AMMyu265tkBpRW21iGQxKGLaves3gKm2JcMUqfXNSpqD`, v0.4). |
| `fetch-metadao-v06.sh` | The **v0.6** governance stack: `futarchy` (`FUTARELBfJfQ8RDGhg1wdhddq1odMAJUePHFuBYfUxKq`), `conditional_vault` (unchanged), Meteora DAMM v2 cp-amm (`cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG`), and Squads v4 (`SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf`). Additive — it does not touch the v0.4 fixtures. |

Each script documents its **authoritatively sourced** program IDs in a header comment (with
the `declare_id!` / `Anchor.toml` source of truth). Do not edit the IDs from memory.

## Usage

```bash
# from the repository root
./scripts/fetch-metadao.sh        # v0.4 dispute-core fixtures
./scripts/fetch-metadao-v06.sh    # v0.6 futarchy + Meteora fixtures
```

Each script uses `solana program dump` against mainnet-beta and writes the `.so` binaries
into [`programs/oracles/tests/fixtures/`](../programs/oracles/tests/fixtures). Runs are
idempotent — re-running overwrites the fixtures with a fresh dump. Run them once (or
whenever the pinned MetaDAO versions change) before the CPI integration tests, so
`just test` can load the real programs into LiteSVM.

## Related

- [`programs/oracles/src/cpi/`](../programs/oracles/src/cpi) — the hand-built CPI into
  these programs (`metadao.rs` = v0.4, `metadao_v06.rs` = v0.6).
- [Challenge markets](../docs-site/challenge) in the docs site — how the conditional vaults
  and AMMs are composed into a decision market.

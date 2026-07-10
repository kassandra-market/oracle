# Kassandra

**A decentralized, AI-assisted optimistic oracle on Solana.**

Kassandra answers **binary and categorical** questions. The common case is cheap: an
uncontested proposal settles with no AI and no markets. The dispute machinery — fact
agreement, AI claims, and decision markets — only fires when proposers disagree.

The core idea: **interpretation is fixed at oracle creation**, so disputes reduce to *which
evidence is real and relevant* (objective) rather than *what the evidence means*
(subjective). An AI applies that fixed interpretation to an agreed fact set, and a
MetaDAO-style decision market is the ultimate arbiter that can override a faulty AI claim.

No zkTLS, no TEEs. Honesty is enforced **economically** (KASS staking and slashing) and by
**markets** (the final arbiter of truth).

> **Full documentation** lives in [`docs-site/`](./docs-site) — an extensive Mintlify site
> (concepts, architecture, protocol reference, challenge markets, SDK, and dApp guide). See
> [`docs/plans/2026-06-29-kassandra-design.md`](./docs/plans/2026-06-29-kassandra-design.md)
> for the original design document.

## How an oracle resolves

1. **Create** — a creator posts a prompt, immutable interpretation rules, categorical
   options, and a deadline, and pays a dynamic KASS creation fee (burned).
2. **Propose** — after the deadline, proposers submit a categorical value plus a KASS bond,
   no proofs. If everyone agrees, the oracle **resolves** immediately — no AI, no markets.
3. **Dispute** (only on conflict) — two or more distinct values lock the proposers in and
   open a **fact proposal** window, then a disjoint **fact voting** window that freezes the
   agreed evidence set.
4. **AI claims** — each locked-in proposer reruns the open-source runner over the agreed
   facts and resubmits a value plus AI-claim metadata (model, params, hashes).
5. **Challenge** — every AI claim is challengeable in parallel; a challenge opens a MetaDAO
   decision market, and a fail-vs-pass TWAP decides whether the claim is disqualified.
6. **Resolve or dead-end** — after the last market settles, the final plurality over
   surviving proposers is computed. If nothing survives (or a tie), the oracle reaches an
   **Invalid dead-end**, resolvable only by KASS governance.

## Two products, one repo

This monorepo hosts **two** on-chain programs and the shared surface around them:

- **Kassandra** — the AI-assisted optimistic oracle described above.
- **[Kassandra Market](./programs/kassandra-market)** — a KASS-denominated **AMM
  prediction market** that wraps MetaDAO v0.4 `conditional_vault` + `amm` and defers
  resolution to the oracle. Program ID `FEGNHWAB7kc7VC9CCwbvVPsv4Jykz2r2WQ758V4xCT9S`.

There is a **single app** (both `/oracles` and `/markets`) and a **single indexer**
(one Postgres, two pipelines) serving both. See the docs
[Prediction markets](./docs-site/market/overview.mdx) section.

## Monorepo layout

| Path | What it is |
| --- | --- |
| [`programs/kassandra/`](./programs/kassandra) | The oracle Solana program, written in **Pinocchio** (not Anchor). Owns oracle state, phases, facts, AI claims, plurality, staking, emissions, and the dynamic fee. Program ID `KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY`. |
| [`programs/kassandra-market/`](./programs/kassandra-market) | The **prediction-market** program — a Pinocchio wrapper over MetaDAO v0.4 vault + amm, resolved by the oracle. |
| [`runner/`](./runner) | The open-source AI runner (`kassandra-runner`). Applies the fixed interpretation to the agreed facts and produces a categorical answer plus verifiable metadata. |
| [`sdks/oracles/ts/`](./sdks/oracles/ts) · [`sdks/markets/ts/`](./sdks/markets/ts) | Hand-written TypeScript clients (`@kassandra-market/oracles`, `@kassandra-market/markets`) — instruction builders, account decoders, PDA helpers. No IDL; layouts mirror the programs. |
| [`sdks/oracles/rust/`](./sdks/oracles/rust) · [`sdks/markets/rust/`](./sdks/markets/rust) | The Rust SDKs (`kassandra-oracles-sdk`, `kassandra-markets-sdk`). |
| [`indexer/`](./indexer) | The single Carbon indexer — two pipelines (oracle transactions → `events`; market accounts → `market_accounts`) into one Postgres, serving both read + gateway APIs. |
| [`app/`](./app) | The single frontend (Vite + React) — both the oracle (`/oracles`) and market (`/markets`) sections. |
| [`docs-site/`](./docs-site) | The Mintlify documentation site (published via GitHub Actions → GitHub Pages) — covers both products. |
| [`docs/`](./docs) | Design documents + the dated implementation plans (`docs/plans/`) for both programs. |
| [`scripts/`](./scripts) | Helper scripts — dumping MetaDAO program binaries into the test fixtures. |

MetaDAO's deployed **conditional-vault + AMM** programs are reused via CPI — by the
oracle for the pass/fail decision markets, and by Kassandra Market for the cYES/cNO
AMM; neither reimplements the vault or AMM.

## Getting started

### Prerequisites

- **Rust** (stable — see [`rust-toolchain.toml`](./rust-toolchain.toml)) with the
  Solana toolchain (`cargo build-sbf`, from the Solana CLI / Agave).
- **Node.js** and **pnpm** (the `sdk`, `sdks/markets/ts`, `app`, and `docs-site` form a pnpm workspace).
- [`just`](https://github.com/casey/just) for the program build/test recipes.
- For the e2e / dev-stack targets: [`surfpool`](https://surfpool.run) and Postgres
  (`initdb`/`pg_ctl`).

### One entrypoint: `make`

Every useful task is a `make` target — `make help` lists them all. It delegates to
`cargo`, `just`, `pnpm`, and the `scripts/*.sh`, so there's a single surface:

```bash
make setup       # install JS deps + build both programs (.so) and both SDKs (first run)
make build       # build everything (both programs, both sdks, app, runner, indexer)
make test        # all unit tests (rust workspace + both sdks + app + indexer)
make lint        # oxlint (app) + clippy (rust)
make typecheck   # both sdks + app tsc
make dev         # boot a seeded local surfpool chain (both programs) AND the app dev server

make test-e2e            # browser E2E (surfpool + funded wallet + app)
make test-e2e-fork       # mainnet-forked challenge-market E2E
make test-e2e-indexer    # surfpool + Postgres + indexer + ActivityFeed E2E
make ci                  # exactly what CI runs
```

`make dev` boots surfpool, deploys **both** programs (+ the MetaDAO v0.4 fixtures the
market CPIs), seeds a spread of oracles across phases **and** demo prediction markets,
starts the single Postgres-backed indexer over both, and serves the app — then holds
the chain alive so you can browse `/oracles` and `/markets`. Ctrl-C tears it down.

### Build & test the program

```bash
just build            # cargo build-sbf for BOTH programs (oracle + market → target/deploy/*.so)
just test             # rebuilds the .so files first, then runs both LiteSVM test suites
```

The tests are **LiteSVM** unit + invariant + CPI-integration tests. `just test` depends on
`just build` so you never test a stale `.so`.

### Build the SDK and run the dApp

```bash
pnpm install
pnpm --filter @kassandra-market/oracles build         # the app imports the built oracle SDK
pnpm --filter ./sdks/markets/ts build  # …and the market SDK (note: `--filter sdk` won't match `@kassandra-market/oracles`)
pnpm --filter ./app dev           # serve the frontend locally
```

See each package's README for details:
[program](./programs/kassandra/README.md) ·
[runner](./runner/README.md) ·
[sdk](./sdk/README.md) ·
[app](./app/README.md) ·
[docs-site](./docs-site/README.md) ·
[scripts](./scripts/README.md).

## Architecture notes

- **Pinocchio, not Anchor.** Manual account deserialization/validation and manual
  instruction dispatch (no macros/IDL). CPI into MetaDAO's Anchor programs is constructed
  by hand (8-byte sighash discriminators + account metas + Borsh args). The trade-off: more
  manual serialization in exchange for a smaller, cheaper, dependency-light program.
- **On-chain:** request config, all stakes/bonds (KASS) and market collateral (USDC), the
  fact set & approvals, AI-claim metadata, plurality result, market triggers, emissions,
  and dynamic-fee state.
- **Off-chain:** model inference, private to each runner. No raw AI output on-chain — only
  the categorical claim and verifiable metadata.
- **Trust model:** economic + market-based. KASS slashing for bad facts/claims; MetaDAO
  decision markets as the ultimate arbiter over a faulty AI claim.

## Tokens

- **KASS** — the SPL token for staking, slashing, and decision-market collateral. No
  presale; fair-launch via participation emissions. Required to propose, to stake on facts,
  and (as a proposer) it is your conditional-market collateral.
- **USDC** — a challenger's stake when opening a decision market.

## Status

Kassandra is under active development. The program, SDK, runner, and dApp are implemented
and covered by LiteSVM and end-to-end (surfpool) tests; economic parameters (emission
curve, fee-EMA constants, reward splits) are still being tuned. See `docs/plans/` for the
implementation history and open items.

# Backend Carbon Indexer + RPC-hiding gateway

**Goal:** A Rust backend indexer built with **Carbon** (`carbon-core` 1.0.0) that indexes the kassandra-market program's accounts and is the app's **sole** data + transaction gateway. The app fetches ALL data from the indexer and submits ALL transactions through it, so the Solana **RPC URL is never in the browser bundle**. Deployed on Render as a **private service** fronted by a proxy web service. Tested, and wired into the e2e.

**Reference:** a working Carbon 1.0.0 indexer exists at `/Users/dode/Documents/solana/kassandra/indexer` (instruction-based, Postgres). Mirror its `Cargo.toml` (carbon-core 1.0.0, axum 0.8.9, solana-* v3, tokio full), `main.rs` (pipeline build + concurrent axum via `tokio::select!`), `processor.rs` (`Processor` impl + `Error::Custom` mapping + shared `Arc` state), `api.rs` (axum router sharing state via `State`). We index ACCOUNTS (not instructions) and use an **in-memory** store (no Postgres).

## Carbon API (verified, v1.0.0)
- `AccountDecoder<'a> { type AccountType; fn decode_account(&self, &'a solana_account::Account) -> Option<DecodedAccount<Self::AccountType>> }`.
- `Processor<AccountProcessorInputType<'a, T>> { async fn process(&mut self, &AccountProcessorInputType<'a,T>) -> CarbonResult<()> }`; input gives `.metadata.pubkey`, `.metadata.slot`, `.decoded_account.data`.
- `Pipeline::builder().datasource(ds).account(decoder, processor).build()?.run().await`. Runs as a blocking daemon; combine with axum via `tokio::spawn` + `tokio::select!` (Carbon does NOT own the runtime).
- Datasources: `carbon-rpc-gpa-datasource` (`GpaDatasource::new(http_url, program)`) for the initial snapshot + periodic reconcile; `carbon-rpc-program-subscribe-datasource` (`RpcProgramSubscribe`, ws) for the live tail. Register BOTH `.datasource(...)` (snapshot won't drift). Confirm the two constructors via `cargo doc` after adding the crates.

## Render topology (verified)
A browser static app cannot reach a Render Private Service. So: **indexer = `type: pserv`** (no public URL; holds `SOLANA_RPC_URL` secret) + a **web service** that serves the app's static build and same-origin-proxies `/api/*` to the indexer over the internal network (`fromService … hostport`). The browser only ever calls same-origin `/api/*` — neither the RPC nor the indexer URL is in the bundle. Locally + in e2e: Vite dev-server proxies `/api → http://127.0.0.1:$INDEXER_PORT` → indexer → surfpool (same-origin, mirrors prod).

---

## Phase 1 — Indexer core (`indexer/` Rust crate)

**Cargo.toml** (add `indexer` to the root workspace members): `carbon-core = "1.0.0"`, `carbon-rpc-gpa-datasource = "1.0.0"`, `carbon-rpc-program-subscribe-datasource = "1.0.0"`, `axum = "0.8"`, `tower-http` (cors), `tokio = { features=["full"] }`, `solana-client`/`solana-rpc-client` + `solana-sdk`/`solana-account`/`solana-pubkey` v3, `serde`/`serde_json`, `base64`, `bytemuck`, `anyhow`, `bs58`, and — REUSE — `kassandra-market-program = { path=..., default-features=false, features=["no-entrypoint"] }` (for the `state::{Config,Market,Contribution,AccountType,MarketStatus,Phase-from-oracle}` Pod structs) + `kassandra-market-sdk = { path=... }` (PDAs + program id + MetaDAO consts + the Amm offset consts).

Modules:
- `store.rs`: `Store { config: Option<(Pubkey, Config, u64 /*slot*/)>, markets: HashMap<Pubkey,(Market,u64)>, contributions: HashMap<Pubkey,(Contribution,u64)> }` behind `Arc<RwLock<Store>>`. Upsert methods are **slot-gated** (last-writer-wins: only apply if `slot >= stored slot`). Also a `contributions_for(market)` filter (Contribution.market field).
- `decoder.rs`: hand-written `KassandraAccountDecoder { program_id }` → tag-byte dispatch (0/1/2 wait — our AccountType is Config=1/Market=2/Contribution=3; tag@0) + size guard → `bytemuck::from_bytes` (guarded; use `try_from_bytes` for safety). Note the accounts have `account_type@0` then real fields; decode the WHOLE struct (`from_bytes::<Market>(&data[..Market::LEN])`) not `split_first` (the tag is part of the Pod struct). Returns `KassandraAccount::{Config,Market,Contribution}`.
- `processor.rs`: `KassandraAccountProcessor { store: Arc<RwLock<Store>> }` impl `Processor<AccountProcessorInputType<KassandraAccount>>` → slot-gated upsert into the store by `metadata.pubkey`/`slot`. (A closed/reaped account still lingers in the store; acceptable — or add a lamports==0/removed handling later.)
- `rpc.rs`: an `RpcClient` (nonblocking, `solana-rpc-client`) holding the http URL for the on-demand + tx endpoints: `get_account(pubkey) -> Option<{data,owner,lamports}>`, `latest_blockhash()`, `send_raw_transaction(bytes) -> Signature`, `signature_status(sig)`. These power the tx gateway + foreign-account reads (oracle, amm reserves, ATA-existence).
- `json.rs`: serde DTOs mapping the Pod structs → JSON (pubkeys as base58, u64 as string to avoid JS precision loss). `MarketDto` (all Market fields + status label), `ContributionDto`, `ConfigDto`, `MarketDetailDto { market, contributions[], oracle: {optionsCount,phase,resolvedOption}|null, reserves: {base,quote}|null }`, `AccountDto { data(base64), owner, lamports }`.
- `api.rs`: axum `Router` with `Arc<AppState { store, rpc, program_id }>` as `State`, a permissive-but-origin-scoped CORS layer (`tower_http::cors`, `ALLOWED_ORIGIN` env, default `*` for local). Routes:
  - `GET /health` → 200.
  - `GET /api/config` → ConfigDto | 404.
  - `GET /api/markets` → [MarketDto] (from the store).
  - `GET /api/markets/:pubkey` → MarketDetailDto: market from store, contributions from store (filter by market), then **on-demand RPC**: read the oracle (`market.oracle`, decode optionsCount@160/phase@161/resolvedOption@197 — reuse the program's Oracle layout via `kassandra_market_program`? no — the oracle is the KASSANDRA program's; decode the 3 bytes directly) and the amm reserves (read `market.amm` account, verify the AMM disc, base@115/quote@123 — reuse `kassandra_market_sdk::metadao` offsets).
  - `GET /api/account/:pubkey` → AccountDto | 404 (generic on-demand RPC read — powers ATA-existence, step-landed, balances).
  - `GET /api/blockhash` → { blockhash }.
  - `POST /api/transaction` { tx: base64 } → decode, `send_raw_transaction`, → { signature } (or a 4xx with the RPC error/logs).
  - `GET /api/transaction/:sig` → { status: "processed"|"confirmed"|"finalized"|"failed"|"pending", err? }.
- `main.rs` (mirror the sibling): read env (`PORT` default 10000, `SOLANA_RPC_URL` http, `SOLANA_WS_URL` ws, `ALLOWED_ORIGIN`, `MARKET_PROGRAM_ID` default the known id); build `Arc<RwLock<Store>>` + rpc client; `tokio::spawn` axum on `0.0.0.0:$PORT`; build the pipeline with `GpaDatasource` (snapshot) + `RpcProgramSubscribe` (live) both feeding `KassandraAccountDecoder`+`KassandraAccountProcessor`; `tokio::select!` { pipeline.run(), ctrl_c() }.

**Tests** (`indexer/tests/` or `#[cfg(test)]`): decoder round-trip (fabricate a Config/Market/Contribution byte buffer via the Pod structs → decode → assert fields); store slot-gating (older slot doesn't overwrite); axum handler tests with a hand-seeded `Store` (no RPC) for `GET /api/config`, `/api/markets`, and the JSON shape (using `axum::body` + `tower::ServiceExt::oneshot`). The RPC-backed routes (blockhash/transaction/account/detail-enrichment) are covered in Phase 4's e2e against surfpool (they need a live RPC). `cargo build -p kassandra-market-indexer` + `cargo test -p kassandra-market-indexer` green. Commit `feat(indexer): Carbon account indexer + axum data/tx gateway`.

## Phase 2 — App: route ALL data + tx through the indexer (remove RPC)

- `app/src/lib/indexer.ts`: `IndexerClient` (base `/api`, override via `VITE_API_BASE`) with `getConfig()`, `getMarkets()`, `getMarket(pubkey)`, `getAccount(pubkey) -> {data:Uint8Array,owner,lamports}|null` (base64→bytes), `getBlockhash()`, `sendTransaction(txBase64) -> signature`, `getSignatureStatus(sig)`. All `fetch` same-origin.
- Replace `ClusterProvider`/`useConnection`/`cluster.ts` with `IndexerProvider`/`useIndexer` (provides an `IndexerClient`). Delete `VITE_RPC_URL` + the cluster switcher + the web3.js `Connection` from the app entirely (keep `@solana/web3.js` for `Address`/`Transaction`/`Keypair` types + tx building only — NO `Connection`).
- `data/markets.ts`: rewrite `fetchMarkets`/`fetchMarketDetail`/`fetchConfig` to call the `IndexerClient` and map the DTOs to the existing app types (the indexer already decoded, so drop the client-side getProgramAccounts + Amm/oracle byte-reads). The `AmmReserves`/oracle now come from the detail DTO.
- `data/send.ts` + `hooks/useWriteAction.ts` + `lib/e2eWallet.tsx`: the send path becomes build-ix (SDK, pure) → `indexer.getBlockhash()` → set feePayer+blockhash → `wallet.signTransaction(tx)` (NOT `sendTransaction(tx, connection)`) → `indexer.sendTransaction(base64(serialize))` → poll `indexer.getSignatureStatus`. The `E2eWalletProvider` keypair-signs then posts to the indexer (drop its `connection.sendRawTransaction`). The real-wallet path uses `wallet.signTransaction` + indexer relay.
- The action builders (`data/actions/*`) that took a `Connection` for existence checks (`ata.ts`, `activate.ts stepAlreadyLanded`, `create` oracle read) → take the `IndexerClient` and use `getAccount`.
- `useKassBalance`: `indexer.getAccount(kassAta)` → decode SPL amount@64.
- Keep app unit tests green (mock the `IndexerClient` instead of a `Connection`). typecheck/lint/build/test green. Commit `feat(app): route all reads + tx through the indexer (no RPC in the bundle)`.

## Phase 3 — Deploy config

- `indexer/Dockerfile`: multi-stage Rust build FROM the repo root context (it depends on `programs/kassandra-market` + `sdk-rs` path crates — the Docker context must be the repo root; copy the workspace + build `-p kassandra-market-indexer`). Runtime stage: debian-slim + ca-certificates, bind `0.0.0.0:$PORT`.
- `app/server.mjs` (+ `app/server-package` deps or a tiny express in the app): a minimal Node web service that serves `app/dist` and reverse-proxies `/api/*` → `INDEXER_URL` (the pserv internal hostport). Used ONLY on Render (local uses Vite's proxy).
- `render.yaml` at repo root: `kassandra-indexer` (`type: pserv`, `runtime: docker`, `dockerfilePath: ./indexer/Dockerfile`, `dockerContext: .`, env `SOLANA_RPC_URL`/`SOLANA_WS_URL` `sync:false`, `PORT`); `kassandra-web` (`type: web`, `runtime: node`, serves the app + proxies, env `INDEXER_URL` via `fromService {name: kassandra-indexer, type: pserv, property: hostport}`, same region). Document the deploy (Blueprint, since the Render MCP can't create a pserv). Commit `feat(deploy): render.yaml (private indexer + proxy) + Dockerfile`.

## Phase 4 — Test the indexer + use it in the e2e

- **Indexer integration test** (against surfpool): a Rust `#[tokio::test]` (gated behind an env flag like the surfpool ones) OR a small TS test — boot surfpool + deploy program + seed a market via cheatcodes, run the indexer pointed at surfpool (http+ws), then hit its HTTP endpoints (`/api/markets`, `/api/config`, `/api/markets/:pubkey`, `/api/blockhash`, `POST /api/transaction` with a signed tx) and assert the indexed data matches on-chain + a relayed tx confirms. This directly "tests the indexer."
- **App e2e through the indexer:** extend `app/e2e/global-setup.ts` to ALSO boot the indexer binary (`cargo run -p kassandra-market-indexer` or a prebuilt binary) pointed at the surfpool RPC/ws, and configure the Playwright `webServer` (Vite) with a `/api` proxy → the indexer. The specs now drive the app which fetches from + submits through the indexer (no RPC in the app). Assert: the markets list renders from the indexer; a write (contribute/create) round-trips app→indexer→surfpool and the on-chain effect appears (poll the indexer's `/api/account` or the existing `onchain.ts`). Keep the existing specs green (they now go through the indexer). Add an `indexer.spec.ts` or fold into the existing ones. Commit `test(e2e): app↔indexer↔surfpool end-to-end + indexer integration test`.

## Verification (goal-complete when ALL hold)
- Indexer builds + unit tests green; the integration test indexes real accounts + relays a tx against surfpool.
- App builds/typechecks/lints; app unit tests green; **grep confirms no RPC URL / `new Connection` / `getProgramAccounts` remain in `app/src`** (only the indexer client + tx building).
- Playwright e2e green with the app talking ONLY to the indexer (which talks to surfpool).
- `render.yaml` + Dockerfile present (private pserv indexer + proxy web service); documented.

## Reuse (aligns with the standing reuse goal)
The indexer reuses the program's Pod structs (`kassandra-market-program`) for decoding and `sdk-rs` for PDAs/program-id/MetaDAO offsets — one source of truth for layouts across the on-chain program, the Rust SDK, and now the indexer.

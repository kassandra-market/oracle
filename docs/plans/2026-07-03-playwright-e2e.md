# Playwright Browser E2E Test Suite (app)

> **For Claude:** one cohesive deliverable, mirrors the proven sibling e2e harness + reuses our surfpool harness. Build the e2e wallet + seeding + config + specs, then RUN green in headless chromium.

**Goal:** Browser end-to-end tests that drive the **real app in a headless browser** against a **surfpool** instance with the program deployed + state seeded, using an **auto-signing in-page wallet** (no extension, no popup). Proves the full loop: browser UI → wallet-adapter → app `Connection` → RPC → program, with on-chain assertions.

**Confirmed environment:** Playwright + headless chromium install & launch OK here; surfpool 1.0.0 forks mainnet in ~2s and lazy-fetches the MetaDAO programs; our `sdk/test/surfpool/harness.ts` (`MarketSurfpoolHarness`) already has every cheatcode + `seedOracle`/`createMint`/`fundTokenAccount`/`sendIx` needed for seeding.

**The linchpin (mirror the sibling `app/src/lib/e2eWallet.tsx`):** an `E2eWalletProvider` gated by `VITE_E2E=1`/`?e2e` that reads a 64-byte secret from `window.__E2E_WALLET_SECRET__` (injected by Playwright via `page.addInitScript` before app JS runs), builds a `Keypair`, and implements the wallet-adapter `sendTransaction(tx, connection)` by setting feePayer + blockhash, `tx.sign(keypair)`, `connection.sendRawTransaction(serialize, {skipPreflight:false})`. web3.js `3.0.0-rc.2` (async `Keypair.fromSecretKey`/`tx.sign`). The app's RPC is already overridable via `VITE_RPC_URL` (cluster.ts, localnet default) — point it at surfpool.

---

## Deliverables

### 1. App: E2E wallet mode (port from the sibling verbatim, adjust imports)
- `app/src/lib/e2eWallet.tsx` — `E2eWalletProvider` (copy the sibling verbatim; it's a complete real-signing adapter — publicKey/connected from the injected keypair, `sendTransaction`/`signTransaction`/`signAllTransactions` keypair-sign over the passed Connection).
- `isE2eMode()` helper (`import.meta.env.VITE_E2E === '1' || (window && new URLSearchParams(location.search).has('e2e'))`) — put it in `app/src/lib/e2e.ts` (we have no `mockOracles.ts`).
- `app/src/providers/AppProviders.tsx` — add the branch: `isE2eMode() ? <E2eWalletProvider> : <WalletProvider autoConnect>`, inside `ClusterProvider`, wrapping `WalletModalProvider`. (No mock-render mode needed — skip `mockWallet`.)
- Confirm `app/src/lib/cluster.ts` already honors `VITE_RPC_URL` for localnet (it does, from app Task 1) — the e2e webServer sets it to the surfpool URL.

### 2. `app/e2e/` harness
- `app/e2e/global-setup.ts` (Playwright globalSetup): boot `MarketSurfpoolHarness.start({ port, fork: "mainnet", readyTimeoutMs: 60000 })` (import via relative path `../../sdk/test/surfpool/harness.ts`); keep the harness handle in a module global so `global-teardown` can kill it. Then seed:
  - `createMint(9)` → KASS mint.
  - generate a fresh e2e wallet `Keypair`; airdrop it ~50 SOL; fabricate its KASS ATA funded (e.g. 1e15 base units) via the harness `fundTokenAccount`.
  - `initConfig` (authority = the e2e wallet or the payer; kassMint; a min_liquidity reachable by a couple contributions) — send via the harness `sendIx` with the SDK `initConfig` builder.
  - `seedOracle({ phase: 1 /*Proposal*/ })` → a Kassandra oracle for create-market.
  - Create a market ALREADY in Funding (so read-only + contribute specs have a target independent of create): send `createMarket` (oracle above, openYesBps 5000, a partial seed below min) via `sendIx` signed by the e2e wallet (fund the wallet's KASS ATA first). Record its market pubkey.
  - Write `app/e2e/.wallet.json`: `{ secretKey: number[], rpcUrl, kassMint, authority, oracle, market }` for the specs.
  - `.gitignore` `app/e2e/.wallet.json` and `.surfpool/`.
- `app/e2e/global-teardown.ts`: kill the surfpool child (harness `teardown`).
- `app/e2e/onchain.ts`: read+decode accounts straight from surfpool RPC (raw `getAccountInfo` or a web3.js `Connection`) using the SDK decoders (`decodeMarket/decodeContribution/decodeConfig`); a `poll(fn, pred, timeoutMs)` helper.
- `app/playwright.config.ts`: `testDir: ./e2e`, `workers: 1`, `fullyParallel: false`, `globalSetup`, `globalTeardown`, `webServer` = `pnpm exec vite --port 5273 --strictPort` with env `{ VITE_E2E: '1', VITE_RPC_URL: '<surfpool url>' }`, `baseURL: http://localhost:5273`, `use: { headless: true }`, generous timeouts (fork RPC is slow). Pin the surfpool port + read the rpcUrl consistently between global-setup and the webServer env (a fixed port, e.g. 8899, or write it into an env file global-setup exports).
- `app/package.json`: `"test:e2e": "playwright test"` script.

### 3. Specs (`app/e2e/*.spec.ts`) — each injects the wallet in `beforeEach`
```ts
const wallet = JSON.parse(readFileSync('app/e2e/.wallet.json','utf8'))
test.beforeEach(async ({ page }) => {
  await page.addInitScript((s) => { (window as any).__E2E_WALLET_SECRET__ = s }, wallet.secretKey)
})
```
- **`markets.spec.ts` (read-only):** `page.goto('/markets')` → the seeded market card renders (status chip "Funding", funding %/TVL). Click it → `/markets/:pubkey` detail renders (status panel, funding panel, bindings panel, contributions). Proves data/decoding in a real browser against real chain state. (Wallet injected but not required for read-only.)
- **`create.spec.ts` (write):** `page.goto('/markets/new')` → assert the wallet shows Connected (injected keypair) → fill oracle address (the seeded oracle) + opening YES % + seed KASS → submit → `waitForURL(/\/markets\/<base58>/)` → assert the new `Market` exists on-chain via `onchain.ts` (`poll(() => fetchMarket(pda), m => m !== null)`), status Funding, creator == the e2e wallet. (Seed a SECOND fresh oracle in global-setup for this so it doesn't collide with the pre-created market.)
- **`contribute.spec.ts` (write):** `page.goto('/markets/'+seededMarket)` → contribute an amount via the ContributeForm → assert the on-chain `Market.totalContributed` increased and a `Contribution` for the wallet exists (poll `onchain.ts`). Note the UI success line is transient (wiped by refetch) — assert the PERSISTENT on-chain effect, not the DOM success text (mirror the sibling).

### 4. `app/e2e/README.md`
Prereqs (surfpool on PATH / SURFPOOL_BIN; `just build` for the `.so`; network for the mainnet fork; `pnpm --filter @kassandra-market/app exec playwright install chromium`), how to run (`pnpm --filter @kassandra-market/app test:e2e`), the injected-wallet mechanism, and that on-chain state is surfnet-fabricated while writes flow through the real program.

---

## Scope + honesty
**Core deliverable (must run green):** read-only market rendering + create-market + contribute — this proves the browser→wallet→RPC→program loop end-to-end with on-chain assertions.
**Stretch (attempt only if core is green and it comes together):** an `activate.spec.ts` driving the activate crank through the UI (the multi-tx sequence in-browser) against the forked MetaDAO programs, asserting Market→Active. If it's fiddly (in-browser multi-tx sequence, conditional-token setup), DEFER it with a clear note — do NOT block the core or fake it.

**RUN IT.** Build the e2e wallet + harness + config + the 3 core specs, then `pnpm --filter @kassandra-market/app exec playwright test` and iterate to GREEN in headless chromium. On failure, use Playwright traces + surfpool logs. Do NOT stub the wallet, skip specs, or weaken on-chain assertions to force green. If a genuine blocker appears (headless chromium can't reach surfpool, a web3.js-in-browser issue, a real app bug), STOP and report it precisely with the trace — a real app bug surfaced here is the point.

## Acceptance
- `pnpm --filter @kassandra-market/app test` (unit, 52) still green — playwright is separate.
- `pnpm --filter @kassandra-market/app test:e2e` → the 3 core specs PASS in headless chromium against surfpool (report the count + wall-clock).
- typecheck/lint clean; default builds unaffected.

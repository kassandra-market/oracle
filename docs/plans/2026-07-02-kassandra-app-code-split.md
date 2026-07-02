# Kassandra dApp — Route Code-Splitting + Vendor Chunking — Design + Plan

> **For Claude:** REQUIRED SUB-SKILL: subagent-driven-development (per-task implement + review).

**Goal:** Cut the dApp's single **752 kB (222 kB gzip) entry chunk** below Vite's 500 kB warning and speed the initial load, via **route-level `React.lazy` + Suspense** (defer the SDK/data/action-heavy pages) + **vendor `manualChunks`** (split wallet-adapter / `@solana/web3.js` / `@kassandra/sdk` into separate cacheable chunks). Pure build/loading optimization — NO behavior change, NO core (programs/runner/sdk) change, NO new deps.

## Baseline (from `pnpm --filter app build`)
- `dist/assets/index-*.js` = **752.00 kB / gzip 222.68 kB** (over the 500 kB warning) — the ONLY app JS chunk.
- Everything is eager: `app/src/main.tsx` statically imports all 5 pages (Landing/Oracles/CreateOracle/OracleDetail/StyleGuide); `vite.config.ts` has no `build.rollupOptions.output.manualChunks`. So React + wallet-adapter + web3.js + the SDK + all pages + the data/action layer are one chunk.

## Approach (both, complementary)
1. **Route-level lazy loading** (`app/src/main.tsx`): `const Landing = lazy(() => import('./pages/Landing'))` etc. for all 5 routed pages; wrap the `<Routes>`/`<Outlet>` content in a `<Suspense fallback={…}>` with an on-brand Delphi fallback (a quiet parchment "loading" — NOT a spinner-heavy thing; reuse the existing loading affordance style, e.g. the "Reading the chain…" tone). This splits each page (+ its imported data/action/component code) into its own lazily-loaded chunk, so `/` (Landing) no longer ships the Oracles/detail/create/SDK-data code up front.
2. **Vendor `manualChunks`** (`app/src/vite.config.ts` `build.rollupOptions.output.manualChunks`): split the heavy libs into separate chunks so they cache independently + leave the entry small — e.g. group `@solana/wallet-adapter-*` + `@solana/web3.js` into a `solana` chunk, `@kassandra/sdk` into a `sdk` chunk (or fold into solana), `react`/`react-dom`/`react-router-dom` into a `react` chunk. Use a function `manualChunks(id)` keying on `node_modules/<pkg>` (careful: `@kassandra/sdk` resolves via the workspace to `sdk/dist`, not node_modules — handle that path too). Tune the groupings so no single chunk exceeds ~500 kB and the ENTRY chunk is well under it.

## Constraints / correctness
- **No behavior change:** every route still renders identically; the wallet/provider wiring (AppProviders) still works; read-only browse + all write flows + the mock affordance all unaffected. The lazy boundary is purely a loading detail.
- **Suspense fallback must not flash/regress a11y:** the fallback is brief, on-brand, and doesn't break the single-`<h1>`-per-page or the landmarks (the fallback is a transient placeholder, replaced by the page). Keep the NavBar/Layout **outside** the Suspense boundary if possible (so the shell/nav stays instant and only the page content lazy-loads) — or a sensible boundary that keeps the nav responsive.
- **The `verify-css` guard must stay green** (Tailwind still compiles; `manualChunks` is JS-only).
- **AppProviders / wallet-adapter note:** the wallet providers wrap the whole app, so wallet-adapter loads eagerly regardless of route — that's expected. The win is (a) route-splitting the page/data/action code and (b) vendor-chunking so the entry chunk shrinks + the big libs cache separately. If the implementer sees a clean, low-risk way to defer the wallet-modal/provider weight further WITHOUT changing behavior, that's a bonus — but do NOT over-engineer or risk the connect flow; the primary deliverable is route-split + vendor chunks under the warning.

## Task CS1 — Route-split + vendor chunks
- **Lazy routes** in `main.tsx`: `lazy()` the 5 pages + a Delphi `<Suspense>` fallback; keep the Layout/NavBar responsive (nav outside the lazy boundary, page content inside).
- **`manualChunks`** in `vite.config.ts`: split solana (wallet-adapter + web3.js) / sdk / react-vendor; tune so the entry chunk is well under 500 kB and no chunk is alarmingly large (some big vendor chunks are fine — they cache; the goal is a small entry + no >500 kB warning, or a documented benign one).
- **Verify the split worked:** `pnpm --filter app build` — capture the NEW chunk list; assert the ENTRY chunk (`index-*.js`) is materially smaller (target: well under 500 kB) and the 500 kB warning is gone (or only on an intentional vendor chunk, documented). `verify-css OK` still prints. Compare before (752 kB) → after in the report.
- **No regression:** `pnpm --filter app typecheck` + `pnpm --filter app test` (72 offline still green — lazy loading is runtime, shouldn't affect the vitest unit/data tests, but confirm) + `pnpm --filter app lint`. Headless-render (if the browser cache is present, else document) `/`, `/oracles?mock`, `/oracles/:pubkey?mock`, `/oracles/new?mock`, `/styleguide` → confirm each route still renders (the lazy chunk loads), the Suspense fallback appears then resolves, 0 console errors, one h1/page. If no headless browser, `vite preview` + curl each route → 200 + the built chunks load, and confirm the lazy chunks exist in dist.
- Update `app/README.md` (a note on the code-split build) + append a CS1 delta to this plan. Commit `perf(app): route-level code-splitting + vendor chunking (752kB entry -> <500kB)`.

## Out of scope / deferred
- Any behavior/feature change, core (programs/runner/sdk) change, or new dep.
- Aggressive provider-level lazy-loading of wallet-adapter (only if trivially safe; not required).
- Font subsetting / other asset optimization (the fonts are already split per-subset by @fontsource).

## Execution note
Pure build optimization. Build the SDK first. The deliverable is a smaller entry chunk (well under 500 kB) via route `lazy()` + vendor `manualChunks`, with EVERY route still rendering identically (verify) + the offline suite + `verify-css` green + NO behavior/core change. Report the before→after chunk sizes. Append a CS1 delta.

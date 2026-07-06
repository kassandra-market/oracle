# Batch "Create all N outcomes" flow

**Goal:** A convenience flow to create all N per-outcome sub-markets of a categorical oracle in one action, instead of creating each `outcome_index` one at a time. Pure SDK + app — **no program change** (each sub-market is still its own `create_market`; the batch composes N of them and sends them as a resumable multi-tx sequence).

## Design
- Each `create_market` creates its own market/escrow/contribution accounts + CPIs → too heavy to pack many per tx reliably, so send **one create per tx** in a sequence (mirror the activate multi-tx pattern via `useActionSequence`, which already does skip-if-exists resume). For N=3–6 that's a handful of txs with clear progress.
- **Skip-if-exists resume:** a `create_market` reverts if its market PDA already exists (the duplicate guard), so a re-run of a partially-completed batch skips already-created outcomes — reuse `useActionSequence`'s `checkAccount` (the step's market PDA) exactly like activate.
- **Seeding:** one shared `seedAmount` applied to each outcome (creator funds N × seed total; show the total). One shared `openYesBps` (default 5000) for all — the per-outcome opening prior isn't expressible cheaply and 50/50 is the v1 seed anyway.

## SDK (`sdk/`)
- `flows/createAll.ts`: `createAllOutcomeMarkets({ oracle, optionsCount, creator, kassMint, creatorKassAta, openYesBps, seedAmount })` → `{ steps: Array<{ outcomeIndex, market, instruction }> }` where each `instruction` is `createMarket({ ..., outcomeIndex })` for outcomeIndex 0..optionsCount-1, and `market = pda.market(oracle, outcomeIndex)`. Pure (no RPC). Export from `flows/index.ts`.
- Test: `test/createAll` (or extend flows.test): N=4 → 4 steps, distinct market PDAs, each ix's payload carries the right outcome_index @10 and openYesBps/seed.

## App (`app/`)
- `data/actions/createAll.ts`: `buildCreateAllSteps({ connection, oracle, optionsCount, creator, kassMint, openYesBps, seedAmount })` → an `ActionStep[]` for `useActionSequence`: each step = (create-ATA prepend if the creator KASS ATA is missing — only needed once, put it on step 0) + the `createMarket` ix for that outcome, with `label` "Outcome i" and `checkAccount` = the market PDA (skip-if-exists). Reuse `flows.createAllOutcomeMarkets` + the existing `ensureKassAta`/compute-budget helpers.
- `CreateMarketForm.tsx`: when `optionsCount > 2`, offer a **"Create all N outcomes"** mode (a toggle or a second submit button beside the single-outcome create). In that mode: hide the outcome selector, show shared `openYesBps` + per-outcome `seedAmount` + a "Total: N × seed = …" line, and run the batch via `useActionSequence` with progress ("Creating outcome i of N…", skip-if-exists). On completion navigate to the grouped categorical view (`/markets`, or scroll to the oracle group). Keep the single-outcome create as the default/other mode.
- Unit test (`app/test`): `buildCreateAllSteps` yields N steps with the right markets + checkAccounts; the ATA prepend only on step 0.

## e2e (optional, if it comes together cheaply)
- A `batch-create.spec.ts`: on a fresh 3-option oracle (seed one in global-setup or reuse the categorical oracle if not yet fully populated), use the batch mode, assert all 3 sub-markets exist on-chain (poll `onchain.ts`) and the grouped card shows 3 outcomes. If flaky/heavy, note it — the SDK + app unit coverage + the existing categorical spec suffice.

## Verify
`pnpm --filter @kassandra-market/sdk test` + `--filter @kassandra-market/app typecheck/lint/build/test` green. Commit `feat: batch create-all-outcomes flow for categorical markets`.

## Not doing
- Packing multiple creates per tx (one-per-tx sequence is simpler + resumable; revisit only if tx count becomes a UX problem for large N).
- Per-outcome distinct seeds/priors (shared seed + 50/50 for v1).

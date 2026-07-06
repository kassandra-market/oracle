# Surfpool (mainnet-fork) E2E Test Suite

> **For Claude:** one cohesive deliverable, mirrors the proven sibling harness. Build harness → smoke → full lifecycle → gate → README, then RUN it green against the fork.

**Goal:** A live-validator end-to-end suite that drives the whole binary-market lifecycle through the **TS SDK** against a **surfpool mainnet fork** — real RPC, real tx signing/blockhash/compute-budgets/confirmation, and the **real deployed MetaDAO** conditional-vault + AMM programs (lazy-fetched from the fork, not fixtures). This is the final validation layer above the LiteSVM tests: it proves the SDK's tx-building + the program + the real MetaDAO programs all agree over a wire, not just against dumped `.so` fixtures.

**Why simpler than the sibling's challenge-market e2e:** kassandra-market only *reads* a Kassandra `Oracle` account (owner = Kassandra program id). We **fabricate that account directly** via the `surfnet_setAccount` cheatcode in whatever phase we need (Proposal for create/activate; rewrite to Resolved+resolved_option for resolve) — exactly like the LiteSVM `seed_kass_oracle`/`set_oracle_resolved` helpers. No Kassandra program, no dispute-core, no time-travel.

**Confirmed environment:** surfpool 1.0.0 present; forks mainnet in ~2s; lazily fetches the AMM v0.4 (`AMMyu265…`) + conditional-vault (`VLTX…`) programs on demand (verified). Network access works.

**Stack:** the sibling `sdk/test/surfpool/harness.ts` is the template — copy it, adjusting `SO_PATH → target/deploy/kassandra_market_program.so`, program id → `MARKET_PROGRAM_ID`. web3.js `3.0.0-rc.2` legacy `Connection` + `Transaction`, Keypair-signed, `sendRawTransaction` + `getSignatureStatuses` poll.

---

## Deliverables (all under `sdk/test/surfpool/`)

### 1. `harness.ts` — `MarketSurfpoolHarness`
Mirror the sibling `harness.ts` verbatim where possible:
- `SO_PATH = resolve(here, "../../../target/deploy/kassandra_market_program.so")`; `surfpoolBinary()`/`surfpoolReady()`/`augmentedPath()` (SURFPOOL_BIN env + `~/.local/bin` etc.).
- `start({ port, fork:"mainnet", blockProductionMode, slotTimeMs, readyTimeoutMs })`: spawn `surfpool start --no-tui --block-production-mode <mode> [--slot-time <ms>] --no-deploy --network mainnet --port <port>`; `waitForHealth` (poll `getHealth`); `deployProgram()` = `surfnet_setAccount(MARKET_PROGRAM_ID, { lamports, owner: BPFLoader2, executable:true, data: hex(elf) })`.
- Cheatcodes: `rpc(method, params)`, `setAccount`, `airdrop` (requestAirdrop + poll getBalance), `confirmSignature` (getSignatureStatuses poll), `teardown`.
- SPL packing: `mintBytes(authority, supply, decimals)` (82-byte), `tokenAccountBytes(mint, owner, amount)` (165-byte), `tokenAccountAmount(data)`, `toHex`.
- **Market-specific helpers:**
  - `createMint(decimals) → Address` (fabricate an SPL mint via setAccount+mintBytes, owner = TOKEN_PROGRAM_ID).
  - `fundTokenAccount(mint, owner, amount) → Address` (fabricate a funded ATA at `pda.associatedTokenAccount(owner, mint)` via setAccount+tokenAccountBytes) and a raw-address variant.
  - `seedOracle({ optionsCount=2, phase, resolvedOption=0xff }) → Address`: fabricate a Kassandra-`Oracle`-owned account — owner = `KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY`, a 392-byte buffer with `account_type=1(Oracle)@0`, `options_count@160`, `phase@161`, `resolved_option@197`. Returns the fabricated oracle pubkey (use a fresh `Keypair`/random address as the oracle key). Confirm the 392 len + offsets against `sdk/src/accounts/oracle.ts` / the Rust `../kassandra` state_layout.
  - `setOracleResolved(oracle, resolvedOption)` / `setOraclePhase(oracle, phase)`: rewrite that oracle account to a terminal phase (Resolved=7 + resolvedOption, or InvalidDeadend=8) mid-test.
  - `sendIx(payer, ixs[], signers[], computeUnits?)`: build legacy `Transaction`, feePayer=payer, blockhash from `getLatestBlockhash`, prepend `ComputeBudgetProgram.setComputeUnitLimit({units})` when given, `tx.sign(payer, ...signers)`, `sendRawTransaction(await tx.serialize(), {skipPreflight:false})`, `confirmSignature`. Return the sig.
  - `getAccountData(address) → Uint8Array | null`; `waitForAccount(address, timeoutMs)`.

### 2. `smoke.test.ts` — the first passing template
`describe.skipIf(!ENABLED)` (`ENABLED = process.env.KASSANDRA_MARKET_E2E === "1" && surfpoolReady()`). Boot the harness (default port e.g. 18899, fork mainnet), airdrop a payer, `createMint(9)` for KASS, build `initConfig` via the SDK, send it, poll for the `Config` PDA, `decodeConfig`, assert authority/kassMint/minLiquidity. Proves: deploy + SDK-built tx + real RPC round-trip + decoder.

### 3. `lifecycle-e2e.test.ts` — the full lifecycle against the fork
Mirror the Rust `programs/kassandra-market/tests/lifecycle_active.rs` recipe, but over RPC through the SDK + real MetaDAO programs. Fixed port (e.g. 18901). Steps (each an SDK-built tx or tx-group, asserting on-chain state):
1. `initConfig` (KASS mint, min_liquidity reachable by creator + 1 contributor).
2. `seedOracle({ phase: 1 /*Proposal*/ })`.
3. Creator: `fundTokenAccount(kass, creator, X)`; `createMarket({ oracle, openYesBps:5000, seed })` → decode `Market` (Funding, totalContributed, escrowVault); assert escrow balance == seed.
4. Contributor: fund + `contribute` to reach ≥ min_liquidity → decode Market totalContributed; decode `Contribution`.
5. **compose + activate** (the real MetaDAO CPI path): `flows.composeMarketInstructions({market, oracle, kassMint, payer})` → send the compose ixs (initialize_question, initialize_conditional_vault, create_amm) as separate txs (each with a compute budget); then `flows.activateInstruction(refs...)` with ~1.4M CU. Decode Market → status Active, `question/vault/yesMint/noMint/amm/lpMint/lpVault` recorded, escrow drained to 0, `lpTotal > 0`; read the AMM reserves (both > 0) via the byte-offset reader.
6. `claimLp` for both contributors → each LP ATA gets pro-rata; sum ≤ lpTotal; contributions `claimed`.
7. **Winner/loser setup + redeem** (prove directional payout): a test user `splitTokens(N)` KASS → N cYES + N cNO (create their conditional ATAs first); drain the cNO leg to a throwaway account so the user holds ONLY cYES (mirror `lifecycle_active.rs`'s cYES-only holder); a second user holds ONLY cNO.
8. `setOracleResolved(oracle, 0)` (YES wins) → `resolveMarket` → decode Market status Resolved, settled; read the MetaDAO `Question` account and assert `payout_denominator @84 != 0`, `num0@76==1`, `num1@80==0`.
9. `redeemInstructions`: the cYES holder redeems → receives ~N KASS; the cNO holder redeems → 0. Assert the winner is paid and the loser gets nothing.
Assert conservation where cheap (total KASS out ≤ total in, escrow 0).

Use generous per-tx compute budgets (compose 400k, activate 1.4M, trade/redeem 400k/300k) and generous confirm timeouts (fork RPC is slower). If a step needs the AMM's slot-based TWAP (only if you do a real `swap` — the split-based winner path avoids it), boot in `clock` mode with `slotTimeMs` and add a slot-advance helper; the split path should NOT need it.

### 4. vitest gate + script
- `sdk/vitest.config.ts`: gate `test/surfpool/**` behind `process.env.KASSANDRA_MARKET_E2E === "1"` (exclude otherwise) + `fileParallelism:false` when enabled (mirror the sibling config).
- `sdk/package.json`: `"test:e2e": "KASSANDRA_MARKET_E2E=1 vitest run"` (or `cross-env`-free inline since we're on a unix shell).

### 5. `sdk/test/surfpool/README.md`
Prereqs (surfpool on PATH / `SURFPOOL_BIN`; `just build` for the `.so`; network access for the mainnet fork), how to run (`KASSANDRA_MARKET_E2E=1 pnpm --filter @kassandra-market/sdk test:e2e`), the fixed ports, and the caveat that outcomes flow through the real forked MetaDAO programs while input state (mints/ATAs/oracle) is `surfnet_setAccount`-fabricated.

---

## Acceptance
- `pnpm --filter @kassandra-market/sdk test` (default, no e2e) still green (50 tests) — the gate excludes surfpool.
- `KASSANDRA_MARKET_E2E=1 pnpm --filter @kassandra-market/sdk exec vitest run test/surfpool/smoke.test.ts` → PASS (deploy + init_config over RPC).
- `KASSANDRA_MARKET_E2E=1 pnpm --filter @kassandra-market/sdk exec vitest run test/surfpool/lifecycle-e2e.test.ts` → PASS (full lifecycle against the fork). **This is the deliverable — it must actually run green here (surfpool works in this env).** If a real MetaDAO CPI reveals a wire-format bug the fixtures missed, that's the point of the e2e — report it precisely.
- typecheck clean.

## Notes / likely friction (debug against the live validator, don't fake)
- The compose+activate account wiring is the riskiest against real programs — if `activate`'s add_liquidity or a compose CPI reverts, inspect the tx logs (surfpool returns program logs on failed confirm) and reconcile the account list vs the SDK flow.
- The fabricated oracle must satisfy `load_kassandra_oracle` (owner + `account_type==Oracle(1)` + len ≥ 392). Get the 392 + offsets right.
- Conditional-token ATAs for split/redeem must exist first (use `flows.ensureConditionalAtasInstructions` or fabricate them).
- Fork RPC latency: raise confirm timeouts (e.g. 30–60s) and the health timeout (60s).

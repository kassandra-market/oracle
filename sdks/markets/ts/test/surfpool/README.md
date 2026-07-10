# surfpool mainnet-fork E2E suite

The top validation layer for `@kassandra-market/markets`: it drives the whole
binary-market lifecycle through the **TS SDK** against a **surfpool mainnet
fork** — real RPC, real tx signing / blockhash / compute-budgets / confirmation,
and the **real deployed MetaDAO** conditional-vault (`VLTX…`) + AMM v0.4
(`AMMyu…`) programs, lazily fetched from the fork (no local `.so` fixtures).

This is the layer above the LiteSVM tests (`test/lifecycle-active.e2e.test.ts`):
it proves the SDK's tx-building + the program + the real MetaDAO programs all
agree over a wire, not just against dumped fixtures.

## Files

- `harness.ts` — `MarketSurfpoolHarness`: spawns surfpool, deploys the local
  `target/deploy/kassandra_market_program.so` at the fixed `MARKET_PROGRAM_ID`
  via the `surfnet_setAccount` cheatcode, and exposes market fabrication helpers
  (`createMint`, `fundTokenAccount`, `createTokenAccount`, `seedOracle` /
  `setOracleResolved`, `sendIx`, `tokenBalance`, `waitForAccount`).
- `smoke.test.ts` — deploy + `init_config` over RPC → decode the `Config` PDA
  (port **18899**).
- `lifecycle-e2e.test.ts` — the full lifecycle: `initConfig → seedOracle →
  createMarket → contribute → compose → activate → claimLp → split → resolve →
  redeem`, with conservation assertions, against the real forked MetaDAO
  programs (port **18901**).

## Input state is fabricated; OUTCOMES flow through the real programs

The KASS mint, user token accounts, and the Kassandra `Oracle` account are
`surfnet_setAccount`-fabricated (the market only *reads* the oracle — owner
`KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY`, 392 bytes, `account_type=1`@0,
`options_count`@160, `phase`@161, `resolved_option`@197). Everything the
lifecycle *asserts* — the conditional vault/mints, the AMM pool + LP, the
question resolution numerators, the redeem payouts — is produced by the real
forked MetaDAO programs.

## Prerequisites

- `surfpool` on `PATH` (or set `SURFPOOL_BIN`). The suite SKIPS (does not fail)
  when it is absent.
- The program artifact built: `just build` (produces
  `target/deploy/kassandra_market_program.so`).
- Network access — surfpool forks mainnet and lazily fetches the MetaDAO
  programs on first touch.

## Running

Gated behind `KASSANDRA_MARKET_E2E=1` (the default `pnpm test` never spawns
surfpool):

```sh
# whole e2e suite
KASSANDRA_MARKET_E2E=1 pnpm --filter @kassandra-market/markets test:e2e

# individual files
KASSANDRA_MARKET_E2E=1 pnpm --filter @kassandra-market/markets exec vitest run test/surfpool/smoke.test.ts
KASSANDRA_MARKET_E2E=1 pnpm --filter @kassandra-market/markets exec vitest run test/surfpool/lifecycle-e2e.test.ts
```

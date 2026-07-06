# `@kassandra-market/sdk`

A hand-written, IDL-free TypeScript SDK for the **kassandra-market** Solana
program — a USDC/KASS AMM prediction market built on the Kassandra oracle and the
MetaDAO v0.4 `conditional_vault` + `amm` primitives.

It exposes:

- **constants + PDAs** (`Ix`, `MarketStatus`, `MarketError`, `pda.*`) — pinned to
  the Rust program and guarded by `test/parity.test.ts`;
- **account decoders** (`decodeConfig`/`decodeMarket`/`decodeContribution`, plus a
  minimal Kassandra-oracle reader);
- the **9 market instruction builders** (`initConfig` … `resolveMarket`);
- the **MetaDAO sub-SDK** (`metadao.*` — conditional-vault + amm v0.4 builders);
- **high-level flows** (`flows.*`) — compose/activate, buy/sell, redeem, and a thin
  Jupiter any-token entry helper.

Every builder returns a `@solana/web3.js@3.0.0-rc.2` (classic API)
`TransactionInstruction`; PDA derivation is async (`Address.findProgramAddress`).

## Install / build

```bash
pnpm install                                  # from the repo root (workspace)
pnpm --filter @kassandra-market/sdk build     # tsc → dist/
pnpm --filter @kassandra-market/sdk typecheck
pnpm --filter @kassandra-market/sdk test      # offline: parity + builders + litesvm
```

The default test suite is fully offline (pure decoders + LiteSVM). It requires the
program artifact `target/deploy/kassandra_market_program.so` (run `just build`
first) and the vendored MetaDAO fixtures under
`programs/kassandra-market/tests/fixtures/`. The opt-in surfpool suite runs behind
`pnpm test:e2e` (`KASSANDRA_MARKET_E2E=1`).

## Usage

Import the namespaced surface:

```ts
import { pda, metadao, flows, decodeMarket, MarketStatus } from "@kassandra-market/sdk";
```

### (a) Keeper — bring a funded market live: `compose → activate`

Before a market can be activated, a client stands up its MetaDAO scaffolding
(a `Question` whose resolver is the Market PDA, a KASS `ConditionalVault` minting
cYES/cNO, and the cYES/cNO AMM pool). `composeMarketInstructions` returns that
ordered instruction list **and every derived address** (`refs`); `activateInstruction`
wires those refs into the on-chain `activate`.

```ts
const market = (await pda.market(oracle)).address;

const { instructions, refs } = await flows.composeMarketInstructions({
  market, oracle, kassMint, payer,
});
// Send the 3 composition ixs (each may be its own tx); they must land in order
// and need a raised compute budget (SetComputeUnitLimit). Then activate:
const activateIx = await flows.activateInstruction({ refs, payer });
```

`activate` splits the escrowed KASS into balanced cYES/cNO, seeds the pool 50/50,
mints LP into the market's `lp_vault`, and flips the market to `Active`. The pool
must be **empty** at activate — the twap seed constants
(`TWAP_INITIAL_OBSERVATION` = `1e12`, `TWAP_MAX_OBSERVATION_CHANGE_PER_UPDATE` =
`(2^64−1)·1e12`, `TWAP_START_DELAY_SLOTS` = `0`) match the Rust harness so the
book is a valid balanced empty seed.

Contributors then permissionlessly claim their pro-rata LP via the `claimLp`
builder.

### (b) Trader — take / close a position (+ the Jupiter any-token boundary)

`buyInstructions` splits KASS 1:1 into a cYES+cNO pair, then swaps the unwanted leg
on the AMM to net a directional YES/NO position; `sellInstructions` is the mirror
(swap the held leg back toward balance, then merge to KASS). App code never derives
conditional tokens — the flows do (defaulting to the user's ATAs), and the SAME
resolved cYES/cNO accounts are threaded into both the split and the swap so they
never disagree.

```ts
const { instructions, userYesAta } = await flows.buyInstructions({
  refs, user, outcome: "yes", kassAmount: 1_000_000n, userKassAta,
  outputAmountMin, // slippage guard from a pool quote
});
```

> **PRECONDITION — the user's token accounts must already exist.** The MetaDAO
> split / swap / merge / redeem instructions carry no ATA/System program, so they
> **cannot** create the user's cYES/cNO (or the redeem KASS) accounts. For a fresh
> wallet, prepend the idempotent creators first:
>
> ```ts
> const { instructions: mk } = await flows.ensureConditionalAtasInstructions({
>   refs, user, includeKass: true, // includeKass for the redeem destination
> });
> const all = [...mk, ...instructions]; // then send (raise the compute budget)
> ```

> **COMPUTE** — a `split` + AMM `swap` CPI (buy) or `swap` + `merge` (sell) can
> exceed the 200k default; prepend a `SetComputeUnitLimit`, as with `activate`.

**Any-token entry via Jupiter.** A trader holding USDC/SOL/etc. swaps into KASS via
Jupiter first, then feeds the KASS into a `buy`. The SDK is offline and does **not**
call the network — it only *shapes* the request and *combines* instructions:

```ts
const req = flows.buildJupiterEntryRequest({
  inputMint: usdc, outputMint: kassMint, amount, slippageBps: 50, userPublicKey,
});
// ── APP does the HTTP work ──────────────────────────────────────────────
//   GET  {req.baseUrl}/quote  with req.quote
//   req.swap.quoteResponse = <that quote>
//   POST {req.baseUrl}/swap  with req.swap
//   deserialize the returned swap tx → jupiterSwapIx
// ─────────────────────────────────────────────────────────────────────────
const { instructions } = await flows.buyInstructions({ /* … */ });
const all = flows.composeWithEntry(jupiterSwapIx, instructions); // [swap, ...market]
```

**Boundary, restated:** the SDK shapes the Jupiter request and stitches the
returned swap instruction in front of the market instructions; the **app** performs
the actual `fetch` to `quote-api.jup.ag`. No SDK test touches the network.

### (c) Redeem — after resolution

Once the Kassandra oracle is terminal, crank `resolveMarket` (bridges the result
into the MetaDAO question), then holders redeem: `redeemInstructions` burns the
holder's full cYES+cNO and pays the resolved KASS per the winning numerators
(a YES winner's cYES pays 1:1, the losing leg pays 0).

```ts
const resolveIx = await resolveMarket({ market, oracle, question: refs.question, cvEventAuthority: refs.cvEventAuthority });
// … send it, then (ensuring the cYES/cNO + KASS accounts exist — see the buy note):
const { instructions } = await flows.redeemInstructions({ refs, user, userKassAta });
```

## Wire-format parity

The wire-format-critical values (instruction discriminants, account sizes, error
codes, program IDs, and the MetaDAO discriminators) are mirrored from the Rust
`sdk-rs` / program and **pinned by `test/parity.test.ts`** — a drift there fails
CI. The end-to-end correctness of the builders + decoders + flows against the real
compiled program is proven by the LiteSVM suites (`test/lifecycle.e2e.test.ts` for
the Phase-1 path and `test/lifecycle-active.e2e.test.ts` for the full
compose → activate → claimLp → resolve → redeem lifecycle).

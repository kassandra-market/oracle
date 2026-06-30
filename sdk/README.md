# `@kassandra/sdk`

Hand-written TypeScript SDK for the **Kassandra** dispute-core Solana program.
No IDL: every instruction, PDA, and account decoder is mirrored byte-for-byte
from the Rust program (the source of truth) and guarded by a parity test, then
verified end-to-end against the real `.so` via [`litesvm`](https://github.com/LiteSVM/litesvm).

- **Program ID:** `KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY`
- **Client library:** `@solana/web3.js@3.0.0-rc.2` (the classic v1-style API,
  reimplemented on `@solana/kit`). PDA derivation and signing are **async** here.

## Install

```sh
pnpm add @kassandra/sdk        # or: npm install @kassandra/sdk
```

Peer/runtime deps (`@solana/web3.js@3.0.0-rc.2`, `@solana/kit`) are pulled in
automatically. For local/integration testing with the bundled litesvm bridge you
also need `litesvm` as a dev dependency.

## Build

```sh
pnpm build        # tsc -> dist/ (ESM .js + .d.ts + sourcemaps)
pnpm typecheck    # tsc --noEmit over src + test + examples
pnpm test         # vitest (unit + litesvm end-to-end)
```

### Prerequisite for litesvm-based usage/tests: the program `.so`

The litesvm bridge, the `test/*.test.ts` integration tests, and `examples/quickstart.ts`
load the compiled program from `target/deploy/kassandra_program.so`. Build it first
**from the repo root**:

```sh
just build        # produces target/deploy/kassandra_program.so
```

Unit tests (codec round-trips, PDA derivation, parity) need no `.so`.

## Quickstart

```ts
import {
  pda,
  initProtocol,
  createOracle,
  propose,
  finalizeProposals,
  decodeOracle,
  decodeProtocol,
  Phase,
  toLiteSvmTransaction,
} from "@kassandra/sdk";
import { Keypair, Transaction } from "@solana/web3.js";

// 1. Derive a PDA. All derivations are async and return `{ address, bump }`.
//    The oracle PDA's nonce is encoded as a u64 little-endian seed for you.
const oraclePda = await pda.oracle(1n);          // pda.protocol(), pda.proposer(...), ...

// 2. Build an instruction. Builders are async (they derive the PDAs they own),
//    take an ergonomic named-arg object, and return a web3.js `TransactionInstruction`.
const ix = await createOracle({
  nonce: 1n,
  promptHash: new Uint8Array(32),
  optionsCount: 3,
  deadline: 1_900_000_000n,   // unix seconds
  twapWindow: 600n,
  creator: creator.publicKey,
  creatorKassToken,
  kassMint,
  usdcMint,
});

// 3. Sign + send with web3.js v3 — NOTE: sign() and serialize() are async (WebCrypto).
const tx = new Transaction();
tx.feePayer = payer.publicKey;
tx.recentBlockhash = recentBlockhash;            // from your RPC / litesvm
tx.add(ix);
await tx.sign(payer, creator);

//    Against a real cluster: send `await tx.serialize()` via your RPC.
//    For LOCAL testing with litesvm, bridge the signed tx and submit it:
const result = svm.sendTransaction(await toLiteSvmTransaction(tx));

// 4. Decode a fetched account (raw bytes -> a fully typed object;
//    u64/i64 as bigint, pubkeys as web3.js Address, enums decoded).
const oracle = decodeOracle(svm.getAccount(oraclePda.address)!.data);
console.log(oracle.phase === Phase.Resolved, oracle.resolvedOption);
```

A complete, runnable end-to-end script (init → create → propose → finalize →
decode) lives in [`examples/quickstart.ts`](./examples/quickstart.ts). With the
`.so` built, run it via `pnpm dlx tsx examples/quickstart.ts`.

## What's in the box

### PDA derivation (`pda.*`)

All 10 seed-pinned derivations, async, returning `{ address, bump }`:
`protocol`, `mintAuthority`, `oracle(nonce)`, `stakeVault(oracle)`,
`proposer(oracle, authority)`, `fact(oracle, contentHash)`,
`factVote(fact, voter)`, `aiClaim(oracle, proposer)`, `market(aiClaim)`,
`challengeUsdcVault(market)`. Each fn is also exported flat (e.g. `oracle(1n)`).

### Instruction builders (22)

Each is `async`, accepts a named-arg object (pubkeys as `Address | string`,
integers as `bigint | number`), takes an optional `programId` override, and
returns a web3.js `TransactionInstruction` with the exact account-meta order +
payload bytes the program expects.

| disc | builder | disc | builder |
| --- | --- | --- | --- |
| 0 | `submitFact` | 11 | `propose` |
| 1 | `voteFact` | 12 | `finalizeProposals` |
| 2 | `finalizeFacts` | 13 | `setGovernance` |
| 3 | `submitAiClaim` | 14 | `setConfig` |
| 4 | `openChallenge` | 15 | `resolveDeadend` |
| 5 | `settleChallenge` | 16 | `kassPrice` |
| 6 | `finalizeOracle` | 17 | `claimProposer` |
| 7 | `advancePhase` | 18 | `claimFact` |
| 8 | `finalizeAiClaims` | 19 | `claimFactVote` |
| 9 | `initProtocol` | 20 | `closeAiClaim` |
| 10 | `createOracle` | 21 | `closeMarket` |

`setConfig` also has `encodeSetConfigParams(params)` for the 22-field payload.

### Account decoders (7)

`decodeProtocol`, `decodeOracle`, `decodeProposer`, `decodeFact`,
`decodeFactVote`, `decodeAiClaim`, `decodeMarket` — each takes the raw account
`Uint8Array`, validates the `account_type` tag + exact size, and returns a typed
object. The matching types (`Protocol`, `Oracle`, …) are exported.

### Errors

`decodeError(custom: number)` maps a `ProgramError::Custom(u32)` (0..=30) to
`{ name, message }`; the `KassandraError` enum is exported too.

### Constants

`KASSANDRA_PROGRAM_ID`, `SYSTEM_PROGRAM_ID`, `TOKEN_PROGRAM_ID`, `Ix`,
`AccountType`, `Phase`, `ACCOUNT_SIZES`, `EXTERNAL_PROGRAM_IDS` (MetaDAO /
Meteora / Squads), `CONFIG` (default governable params), and the sentinels
`CLAIM_OPTION_NONE`, `VOTE_APPROVE`, `VOTE_DUPLICATE`.

## Known limitations / future work

- **Legacy transactions only — large `finalizeProposals` / `finalizeOracle` can
  overflow the packet.** The SDK builds legacy (pre-v0) transactions. These
  instructions append the **full proposer set** as an account tail; a near-cap
  proposer set (`MAX_PROPOSERS == 60`) blows past a legacy transaction's
  1232-byte packet limit. Such calls need a **versioned (v0) transaction + an
  Address Lookup Table** to fit the accounts — not yet provided here. Small/
  moderate proposer sets work fine as legacy txs.
- **MetaDAO market composition is caller-supplied.** `openChallenge` (25
  accounts) and `settleChallenge` (21 accounts) take the externally-composed
  MetaDAO conditional-vault / AMM / mint accounts as inputs — the SDK derives
  only the Kassandra-owned PDAs (oracle, ai_claim, market, stake_vault,
  protocol, escrow). Composing the MetaDAO market is left to the caller.
- **Emissions are governance-enabled and default-disabled.** At genesis
  `emission_num == 0`, so `createOracle` mints no KASS and the creation fee is
  0. KASS reward emission only activates once governance enables it via
  `setConfig`.
- **Not published to npm.** Consumed in-repo from `dist/` for now.

## Layout

```
src/
  constants.ts          program id, Ix, AccountType, Phase, errors, config consts
  pda.ts                the 10 PDA derivations
  accounts/             the 7 Pod account decoders (+ common readers)
  instructions/         the 22 builders (lifecycle / dispute / challenge / settlement)
  litesvm-interop.ts    web3.js v3 -> litesvm transaction bridge
  index.ts              the public barrel
examples/quickstart.ts  runnable end-to-end example
test/                   unit + parity + litesvm end-to-end
```

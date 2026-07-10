# API recon — `@solana/web3.js@3.0.0-rc.2`, `litesvm`, and their interop

> Findings read from the INSTALLED packages' `.d.ts` (not from memory). Date: 2026-06-30.

## Resolved dependency versions

| package | version | notes |
| --- | --- | --- |
| `@solana/web3.js` | `3.0.0-rc.2` | the **legacy/classic v1-style API**, reimplemented on top of `@solana/kit` |
| `@solana/kit` | `6.10.0` | transitive dep of BOTH web3.js v3 and litesvm; added as a direct dep here as the interop bridge |
| `litesvm` | `1.2.0` | latest; speaks `@solana/kit` types natively |
| `vitest` | `2.1.9` | |
| `typescript` | `5.9.3` | |
| `@types/node` | `22.20.0` | |

**Key surprise:** `@solana/web3.js@3.0.0-rc.2` is **NOT** the kit-style functional API
(no `createTransactionMessage`, no `AccountRole`, no `address()` fn, no
`getProgramDerivedAddress`). It is the **classic v1 API** — `PublicKey`, `Keypair`,
`Transaction`, `TransactionInstruction`, `Connection`, `SystemProgram`, etc. —
rebuilt on kit internals. `web3.js` depends on `@solana/kit@^6.8.0`; litesvm depends
on `@solana/kit@^6.10.0`; pnpm resolves both to a **single** `@solana/kit@6.10.0`
instance, so kit types are nominally identical across all three packages. That single
instance is what makes the interop clean.

## web3.js v3 (the classic API)

Import from `@solana/web3.js`.

- **Address type:** legacy `Address` class (also `PublicKey` is exported).
  `new Address(value)` where `AddressInitData = number | bigint | string | Uint8Array
  | ReadonlyUint8Array | Array<number> | Address | (kit Address)`. So
  `new Address("Kass...base58...")` builds one from base58, or from a 32-byte array.
- **Keypair:** `class Keypair implements KeyPairSigner`.
  - `static generate(): Promise<Keypair>` (ASYNC — uses WebCrypto)
  - `static fromSecretKey(bytes): Promise<Keypair>`, `static fromSeed(seed): Promise<Keypair>`
  - `.publicKey` → legacy `Address`; `.address` → **kit branded `Address` string**
    (provided for kit/signer compatibility); `.secretKey` → `Uint8Array`.
- **Instruction:** `new TransactionInstruction({ keys, programId, data? })`
  - `keys: AccountMeta[]` where `AccountMeta = { pubkey: Address; isSigner: boolean;
    isWritable: boolean }` (v1 boolean roles, NOT kit's `AccountRole` enum).
  - `programId: Address`
  - `data?: Uint8Array`
- **Transaction (mutable builder class):**
  - `new Transaction()`; set `.feePayer: Address`, `.recentBlockhash: Blockhash`
    (Blockhash is the kit branded base58 string, re-exported by web3.js).
  - `.add(...ix)` appends instructions.
  - `sign(...signers: Signer[]): Promise<void>` (ASYNC). `Signer =
    MessagePartialSigner | TransactionPartialSigner`; `Keypair` satisfies it.
  - `partialSign`, `addSignature(pubkey, sig)` also available.
  - `serialize(config?): Promise<Uint8Array>` (ASYNC) → **wire-format bytes**
    (signatures + compiled message).
- **PDA:** v1-style `findProgramAddress` / `createProgramAddress` (NOT kit's
  `getProgramDerivedAddress`). (Relevant for D1 — decide PDA API there; kit's
  `getProgramDerivedAddress` is also available via `@solana/kit` if preferred.)

## litesvm (npm) — `litesvm@1.2.0`

`import { LiteSVM, FailedTransactionMetadata, TransactionMetadata } from "litesvm"`.
litesvm's public API is typed entirely in **`@solana/kit`** types
(`Address`, `Transaction`, `EncodedAccount`, `Lamports`, `Blockhash`, `Signature`).

- **Construct:** `new LiteSVM()` (standard config) or `LiteSVM.default()` (minimal).
  Chainable `withComputeBudget/withSigverify/withBlockhashCheck/...`.
- **Load a program from a file:** `addProgramFromFile(programId: Address, path: string): void`.
  Also `addProgram(programId: Address, bytes: Uint8Array)` and
  `addProgramWithLoader(programId, bytes, loaderId)`. `programId` is a **kit `Address`**
  (branded base58 string), built with kit's `address("...")`.
- **Fund / create accounts:**
  - `airdrop(address: Address, lamports: Lamports): TransactionMetadata | FailedTransactionMetadata | null`.
    `Lamports` is `Brand<bigint,'Lamports'>`, built with kit's `lamports(1_000_000_000n)`.
  - `setAccount(account: EncodedAccount): void` — write/overwrite an account directly
    (bypasses runtime checks; useful for fabricating state in later tasks).
  - `getBalance(address): Lamports | null`,
    `minimumBalanceForRentExemption(dataLen: bigint): bigint`.
- **Blockhash:** `latestBlockhash(): Blockhash` (kit branded string). `expireBlockhash()`.
- **Send a transaction:** `sendTransaction(tx: Transaction): TransactionMetadata |
  FailedTransactionMetadata`. **`Transaction` is the kit type**
  `{ messageBytes, signatures }`, NOT the web3.js v3 legacy `Transaction` class.
  `simulateTransaction(tx)` similarly.
- **Read account bytes:** `getAccount(address: Address): MaybeEncodedAccount`.
  The kit `EncodedAccount` carries `.address`, `.data: Uint8Array` (raw account bytes),
  `.executable`, `.lamports`, `.programAddress`, `.space`. This is where the D2 Pod
  decoders will read from.
- **Results:**
  - `TransactionMetadata`: `logs()`, `computeUnitsConsumed()`, `returnData()`, `signature()`, `toString()`.
  - `FailedTransactionMetadata`: `err()` (a structured `TransactionError*`), `meta()`,
    `toString()`. The `InstructionErrorFieldless` enum includes
    `InvalidInstructionData = 2`.

## INTEROP — the make-or-break

litesvm does **NOT** accept the web3.js v3 legacy `Transaction` object. It wants a kit
`Transaction` (`{ messageBytes, signatures }`). The two are different shapes.

**Adapter (works, verified):** build + sign with web3.js v3, `serialize()` to wire
bytes, then decode those bytes into a kit `Transaction` with kit's
`getTransactionDecoder()`:

```ts
import { getTransactionDecoder } from "@solana/kit";
// tx: a signed legacy web3.js v3 Transaction
const litesvmTx = getTransactionDecoder().decode(await tx.serialize());
const result = svm.sendTransaction(litesvmTx);
```

This is implemented in `src/litesvm-interop.ts` (`toLiteSvmTransaction`). It works
because web3.js v3 and litesvm share the **same** `@solana/kit@6.10.0` instance, so the
decoded `Transaction` is the exact type `sendTransaction` expects.

Other interop points (no adapter needed — just use kit helpers for litesvm-facing args):

- Program ID / account addresses passed to litesvm: build with kit `address("...")`.
  (web3.js `Keypair.address` already returns a kit `Address` for the payer.)
- Airdrop amount: kit `lamports(n)`.
- `recentBlockhash`: `svm.latestBlockhash()` returns a kit `Blockhash`, which is exactly
  what the legacy `Transaction.recentBlockhash` field expects (web3.js re-exports
  `Blockhash` from kit).

**Verdict: COMPATIBLE.** A web3.js@3.0.0-rc.2-built-and-signed transaction round-trips
through litesvm via the one-line serialize→decode bridge. No fallback needed.

### Note for D1–D5

Because web3.js v3 is the *classic* API, the kit-style terminology in the plan
(`programAddress`, `AccountRole`, `getProgramDerivedAddress`, branded `Address`
strings) maps to **`@solana/kit`**, not to web3.js v3. Two viable styles going forward:

1. **Classic (web3.js v3):** `TransactionInstruction` + `AccountMeta{isSigner,isWritable}`
   + `findProgramAddress`, bridged to litesvm via `toLiteSvmTransaction`. Used by the D0
   smoke test (genuinely exercises web3.js v3).
2. **Kit-native:** build instructions/messages directly with `@solana/kit`
   (`AccountRole`, `getProgramDerivedAddress`, `compileTransaction`) — zero bridge, fed
   straight to litesvm.

Both are available from the installed deps. Recommendation: pick one consistently in D1
(kit-native is the lower-friction path for litesvm, but web3.js v3 is what the plan
pins as the client lib — the bridge makes it a non-issue either way).

## v0 transactions + Address Lookup Tables (I2) — `src/v0.ts`

`@solana/web3.js@3.0.0-rc.2` DOES export the classic ALT/v0 API (confirmed in
the installed `lib/index.d.ts`, NOT guessed). The near-cap finalizes
(`finalizeProposals` / `finalizeOracle`) inline the FULL proposer set as account
metas; past ~28 proposers a legacy compiled message exceeds the 1232-byte packet
(`PACKET_DATA_SIZE`, also exported), so a near-cap set (`MAX_PROPOSERS = 60`)
overflows. The v0/ALT path packs the read-only proposer PDAs into a lookup table
and references them by a 1-byte index instead of a 32-byte inline key.

**Exact symbols/signatures used (`src/v0.ts`):**

- `AddressLookupTableProgram.createLookupTable(params: { authority: Address;
  payer: Address; recentSlot: bigint | number }): Promise<[TransactionInstruction,
  Address]>` — **ASYNC**; returns `[createIx, lookupTableAddress]`. `recentSlot`
  is read from `connection.getSlot()` (must be < the slot the create ix executes
  in — true since slots advance before the confirmed tx).
- `AddressLookupTableProgram.extendLookupTable(params: { lookupTable: Address;
  authority: Address; payer?: Address; addresses: Address[] }):
  TransactionInstruction` — **SYNC**; the extend ix itself inlines each 32-byte
  key, so the address list is CHUNKED (`DEFAULT_EXTEND_CHUNK = 30`, one extend
  tx each).
- `connection.getAddressLookupTable(key: Address):
  Promise<RpcResponseAndContext<AddressLookupTableAccount | null>>` — fetch the
  resolved table after creation. `AddressLookupTableAccount` has `.key`,
  `.state.addresses`, `.state.lastExtendedSlot`, and `.isActive()`.
- `new TransactionMessage({ payerKey, instructions, recentBlockhash })
    .compileToV0Message(lookupTableAccounts?: AddressLookupTableAccount[]):
    MessageV0` — read-only, non-signer, non-programId keys present in a supplied
  ALT are replaced by `addressTableLookups` (readonlyIndexes). `MessageV0.version
  === 0`; `compileToLegacyMessage()` is the legacy comparison used in the unit test.
- `new VersionedTransaction(message: VersionedMessage)`; `.sign(signers:
  MessagePartialSigner[])` — **ASYNC** (a `Keypair[]`); `.serialize(): Uint8Array`
  — **SYNC** (note: differs from the legacy `Transaction.serialize()` which is
  async). Sent via `connection.sendRawTransaction(bytes, ...)`.

**ALT activation is 2+ txs + a slot wait.** A lookup table is only usable in a v0
tx once it is on-chain AND at least one slot has passed since its last extend
(the added addresses become active the FOLLOWING slot). `createProposerAlt`
polls `getAddressLookupTable` until all addresses are present AND
`getSlot() > state.lastExtendedSlot`. This makes the path **live-cluster /
surfpool only — NOT litesvm** (no ALT resolution / slot progression). On
surfpool, boot with `blockProductionMode: "clock"` + a fast `slotTimeMs` so the
slot advances over wall-clock (the slot-vs-timestamp finding: `surfnet_timeTravel`
moves `getSlot`/`unix_timestamp` but the execution slot only advances in clock
mode). The dispute-core time gates still cross via `advanceToUnix`/timeTravel.

**Public API** (`src/v0.ts`, re-exported from the barrel): `compileV0Message`
(pure, offline-testable), `createProposerAlt`, `sendV0`, and the one-shot
`sendFinalizeViaAlt({ connection, payer, instruction, lookupAddresses,
prependInstructions?, signers?, confirm?, extendChunkSize? })`. Proven in
`test/surfpool/v0-alt-e2e.test.ts` (a 40-proposer legacy finalize overflows;
the v0+ALT finalize resolves the oracle) + offline in `test/v0.test.ts`.

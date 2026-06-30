# Kassandra TypeScript SDK — Design + Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** A hand-written TypeScript SDK (`sdk/`) that lets clients build every Kassandra instruction, derive every PDA, and decode every on-chain account — with NO IDL, mirroring the Rust program's pinned wire formats exactly. Verified against the real program via `litesvm`.

**Architecture:** `@solana/web3.js@3.0.0-rc.2` as the client library. Hand-rolled codecs mirroring the program's fixed-offset `bytemuck::Pod` account layouts (little-endian) + manual instruction-payload encoders. The Rust program is the SOURCE OF TRUTH for every discriminant, byte offset, PDA seed, and payload layout — the SDK mirrors them and a **parity check** guards against drift.

**Tech Stack:** TypeScript, `@solana/web3.js@3.0.0-rc.2`, `litesvm` (npm, the LiteSVM JS binding — direct, not the bankrun wrapper), `vitest` (or node:test) for unit + integration tests. Package manager: pnpm (available).

**Source of truth (the program — all on `master`, do NOT modify):**
- Program ID `KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY`; built `.so` at `target/deploy/kassandra_program.so` (run `just build` to produce it).
- `programs/kassandra/src/instruction.rs` — `Ix` discriminants 0–21 (1-byte leading selector).
- `programs/kassandra/src/state.rs` — the 7 Pod accounts + their exact field offsets; `programs/kassandra/tests/state_layout.rs` — the PINNED absolute sizes + offsets (the SDK parity check mirrors these).
- `programs/kassandra/src/processor/*.rs` — each instruction's account order + payload layout (documented in module headers).
- `programs/kassandra/src/config.rs` — `MINT_AUTHORITY_SEED` etc.; the dispute-core/happy-path/futarchy/challenge/settlement plan deltas — the authoritative running record of seeds/discriminants/layouts/errors.
- MetaDAO program IDs + the futarchy `kass_dao` / Squads vault seeds are in `src/cpi/{metadao,metadao_v06}.rs` (the SDK may need them for the challenge/governance flows).

---

## Live-state the SDK must mirror (verify each against the program before coding)

- **Instructions (Ix, 1-byte disc):** SubmitFact=0, VoteFact=1, FinalizeFacts=2, SubmitAiClaim=3, OpenChallenge=4, SettleChallenge=5, FinalizeOracle=6, AdvancePhase=7, FinalizeAiClaims=8, InitProtocol=9, CreateOracle=10, Propose=11, FinalizeProposals=12, SetGovernance=13, SetConfig=14, ResolveDeadend=15, KassPrice=16, ClaimProposer=17, ClaimFact=18, ClaimFactVote=19, CloseAiClaim=20, CloseMarket=21.
- **Accounts (Pod, fixed-size, LE; sizes from state_layout.rs):** Protocol 368, Oracle 392, Proposer 96, Fact 336, FactVote 88, AiClaim 208, Market 416. Each starts with `account_type: u8` + `_pad_hdr[7]` (`AccountType{Uninitialized=0,Oracle=1,Proposer=2,Fact=3,FactVote=4,AiClaim=5,Market=6,Protocol=7}`). EXACT field offsets are pinned in `tests/state_layout.rs` — read them; do not guess.
- **PDA seeds:** Protocol `[b"protocol"]`; Oracle `[b"oracle", nonce_le8]`; Proposer `[b"proposer", oracle, authority]`; Fact `[b"fact", oracle, content_hash]`; FactVote `[b"vote", fact, voter]`; AiClaim `[b"claim", oracle, proposer]`; stake vault `[b"vault", oracle]`; challenge USDC escrow `[b"challenge_usdc", market]`; Market `[b"market", ai_claim]`; mint authority `[b"mint_authority"]`.
- **Payloads (after the 1-byte disc; verify each in the processor):** e.g. CreateOracle = `nonce u64 ++ prompt_hash[32] ++ options_count u8 ++ deadline i64 ++ twap_window i64`; Propose = `option u8 ++ bond u64`; SubmitFact = `content_hash[32] ++ stake u64 ++ uri_len u16 ++ uri[uri_len]`; VoteFact = `kind u8 ++ stake u64`; SubmitAiClaim = `model_id[32] ++ params_hash[32] ++ io_hash[32] ++ option u8`; SetConfig = 22×u64 (176B); SetGovernance = `dao_authority[32] ++ kass_dao[32]`; ResolveDeadend = `option u8`; the nonce-signed instructions (OpenChallenge/SettleChallenge/FinalizeOracle/FinalizeProposals empty-or-nonce/Claim*/CloseMarket) carry `oracle_nonce u64` where noted. READ each processor for the exact bytes + account order.
- **Errors:** `KassandraError` 0–30 (map to `Custom(u32)`) — mirror as an enum + a decoder for friendly messages.

---

## Conventions
- The SDK is a NEW package; do NOT modify the Rust program (it's the source of truth; if a genuine mismatch/bug is found, STOP and report). 
- TDD-ish: unit tests (codec round-trips, PDA vs known, parity) + litesvm integration. `pnpm test` green + `pnpm typecheck`/`tsc --noEmit` clean before each commit. `pnpm lint`/`prettier` if set up.
- Commit trailer `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`, git author `Kassandra <hexadecifish@gmail.com>`.
- **VERIFY THE REAL APIs:** `@solana/web3.js@3.0.0-rc.2` and `litesvm` (npm) — read the installed packages' types/exports; do NOT assume an API shape from memory. The web3.js major (v3 rc) may differ from v1/v2/kit; confirm `address`/`Address`, transaction building, instruction/account-meta shapes, signing, and how `litesvm` loads a program + submits a tx + reads accounts, and whether litesvm interops with web3.js v3 types or needs adapters.

---

## Tasks

### D0 — Scaffolding + API recon (DO FIRST; stop-and-report if a dep is unusable)
- Create `sdk/` with `package.json` (pin `@solana/web3.js@3.0.0-rc.2`, `litesvm`, `vitest`, `typescript`), `tsconfig.json` (strict), `.gitignore` (node_modules, dist). Use pnpm.
- **API recon (write it down):** install deps; from the INSTALLED packages, determine + document in a short `sdk/NOTES-api.md`: web3.js v3's address type + how you build an instruction (programAddress, accounts/AccountMeta with role/signer/writable, data bytes) + a transaction + sign/send; and litesvm's JS API (new LiteSVM(), addProgramFromFile/add the built .so, how to set/airdrop accounts, sendTransaction, getAccount → bytes). Confirm litesvm ↔ web3.js v3 interop (types/adapters). If `litesvm` npm is incompatible with web3.js v3 or unavailable, STOP and report (fallback: unit-only + documented gap, or a different harness).
- **Smoke test:** a litesvm test that loads `target/deploy/kassandra_program.so` at the program ID and submits an empty/invalid instruction, asserting the program rejects it (`InvalidInstructionData`) — the TS parallel to the Rust smoke test. Proves the .so loads + a tx round-trips via web3.js v3 + litesvm. (Document how to build the .so: `just build` from repo root first.)
- Commit `chore(sdk): scaffold + web3.js v3 / litesvm API recon + smoke test`.

### D1 — Program constants + PDA derivation + parity
- `sdk/src/constants.ts`: program ID, all `Ix` discriminants, `AccountType`, `KassandraError` enum + a `decodeError(custom: number)`, MetaDAO/Squads program IDs (as needed), the config consts the SDK exposes.
- `sdk/src/pda.ts`: a typed derivation fn per PDA (protocol, oracle(nonce), proposer(oracle,authority), fact(oracle,contentHash), factVote(fact,voter), aiClaim(oracle,proposer), stakeVault(oracle), challengeUsdcVault(market), market(aiClaim), mintAuthority) using web3.js v3's PDA API. Document the seed bytes (esp. nonce as u64 LE, the 1-byte/array seeds).
- **Parity test:** assert the discriminant values + the account sizes match the program's pinned values (hardcode the known pinned sizes/discriminants from state_layout.rs + instruction.rs; a mismatch fails the test — this is the drift guard). Optionally derive a couple of PDAs and assert against values the Rust harness produces (if cheap).
- Unit tests for PDA derivation (deterministic; known inputs → known addresses; the nonce-LE encoding correct). Commit `feat(sdk): constants, PDA derivation, parity guard`.

### D2 — Account codecs / decoders (the 7 Pod accounts)
- `sdk/src/accounts/*.ts`: a decoder per account (Protocol, Oracle, Proposer, Fact, FactVote, AiClaim, Market) reading the EXACT pinned offsets (LE), returning a typed object (numbers as bigint for u64/i64, pubkeys as web3.js v3 addresses, the account_type tag verified). Include the fixed `uri[200]` + `uri_len` for Fact. 
- Unit tests: decode known byte buffers (construct a buffer with set fields at the pinned offsets → decode → assert) for every account; assert account-type tag rejection on mismatch; round-trip where an encoder exists. If feasible, decode a REAL account fetched from litesvm (e.g. fabricate a Protocol via init_protocol in a litesvm test → fetch → decode → assert) to prove offsets against the real program. Commit `feat(sdk): Pod account decoders + offset parity`.

### D3a — Instruction builders: protocol + oracle lifecycle
Builders for InitProtocol, CreateOracle, Propose, FinalizeProposals, AdvancePhase, SetGovernance, SetConfig, ResolveDeadend, KassPrice — each producing a web3.js v3 instruction (program address, the EXACT account-meta list in the processor's order with correct signer/writable roles, the payload bytes). Read each processor for the account order + payload. Unit tests: assert the built instruction's data bytes (disc + payload) + account metas match the spec for representative cases. Commit `feat(sdk): protocol + oracle-lifecycle instruction builders`.

### D3b — Instruction builders: dispute + challenge + settlement
Builders for SubmitFact, VoteFact, FinalizeFacts, SubmitAiClaim, FinalizeAiClaims, FinalizeOracle, OpenChallenge, SettleChallenge, ClaimProposer, ClaimFact, ClaimFactVote, CloseAiClaim, CloseMarket — same approach (account order + payload from each processor; the nonce-signed ones include `oracle_nonce`; the MetaDAO-bound ones, OpenChallenge/SettleChallenge, take the externally-composed market accounts). Unit tests for data + metas. Commit `feat(sdk): dispute/challenge/settlement instruction builders`.

### D4 — litesvm integration: end-to-end via the SDK
A litesvm test that builds instructions VIA THE SDK and drives a meaningful real flow against the loaded `.so`: init_protocol → create_oracle → propose (×N) → finalize_proposals (uncontested → Resolved), then decode the Oracle via the SDK and assert the resolved state; plus a dispute-path slice if tractable (the heavy MetaDAO market path can be skipped — covered by the Rust suite — or driven if the SDK's MetaDAO-account composition is in scope). Assert the SDK-built instructions are accepted + the decoders read the resulting accounts. This is the proof the SDK matches the deployed program end-to-end. Commit `test(sdk): litesvm end-to-end lifecycle via the SDK`.

### D5 — Packaging + docs + examples
`sdk/README.md` (install, build-the-.so note, quickstart: derive PDAs, build+send an instruction, decode an account), the package exports (`index.ts`), a build (`tsc`/`tsup` → `dist`), and a short example script. Ensure `pnpm build` + `pnpm test` + typecheck are green. Commit `docs(sdk): readme, exports, example, build`.

---

## Out of scope (later)
- Publishing to npm; the off-chain AI runner; the app/frontend; surfpool/real-validator E2E; auto-generating the SDK from the program (this is the hand-written client by design).

## Execution note
After each task: `pnpm test` + typecheck green, commit. D0 is the riskiest (verify web3.js v3 + litesvm APIs + interop — STOP if a dep is unusable). The parity guard (D1) + decoding a real account (D2) + the litesvm E2E (D4) are what prove the hand-written SDK actually matches the program. The program is read-only truth. Append a D0–D5 delta log here.

---

## Delta log

### D0 — scaffolding + API recon + smoke test (DONE 2026-06-30)

**Resolved deps (pinned/installed):** `@solana/web3.js@3.0.0-rc.2`, `@solana/kit@6.10.0` (added — interop bridge), `litesvm@1.2.0` (latest), `vitest@2.1.9`, `typescript@5.9.3`, `@types/node@22.20.0`. pnpm. `tsconfig`: strict, ESM, `moduleResolution: Bundler`.

**Big finding — web3.js v3 is the CLASSIC API, not kit-style.** `@solana/web3.js@3.0.0-rc.2` is the v1-style API (`PublicKey`, `Keypair`, `Transaction`, `TransactionInstruction`, `AccountMeta{isSigner,isWritable}`, `findProgramAddress`) reimplemented on top of `@solana/kit`. It does NOT expose `createTransactionMessage` / `AccountRole` / `address()` / `getProgramDerivedAddress`. The plan's kit-style terminology (`programAddress`, `AccountRole`, `getProgramDerivedAddress`, branded `Address` strings) actually maps to **`@solana/kit`**, which both web3.js v3 and litesvm depend on (single hoisted `6.10.0` instance → nominal type identity). Full recon in `sdk/NOTES-api.md`.

**litesvm@1.2.0 API (kit-typed):** `new LiteSVM()`; `addProgramFromFile(programId: kit Address, path)` (also `addProgram(id, bytes)`); `airdrop(addr, lamports)` / `setAccount(EncodedAccount)`; `sendTransaction(tx)` where `tx` is the **kit `Transaction` `{messageBytes, signatures}`**, NOT the legacy `Transaction` class; `getAccount(addr): MaybeEncodedAccount` (`.data: Uint8Array` = raw bytes, for D2 decoders); results are `TransactionMetadata` / `FailedTransactionMetadata` (`.err()`, `.logs()`, `.toString()`).

**Interop (verified, no blocker):** litesvm does not take the legacy `Transaction` object. Bridge = build+sign with web3.js v3, `await tx.serialize()` → wire bytes, `getTransactionDecoder().decode(bytes)` → kit `Transaction` → `svm.sendTransaction`. Implemented in `sdk/src/litesvm-interop.ts` (`toLiteSvmTransaction`). For litesvm-facing args use kit helpers: `address("...")`, `lamports(n)`, `svm.latestBlockhash()` (a kit `Blockhash`, accepted directly by legacy `tx.recentBlockhash`). `Keypair.generate()`/`sign()`/`serialize()` are ASYNC (WebCrypto).

**Smoke test (`sdk/test/smoke.test.ts`):** loads the real `target/deploy/kassandra_program.so` at the program ID, builds a web3.js-v3 tx with a single instruction carrying bogus disc `0xFE` + a dummy account, signs, bridges to litesvm, submits. Asserts `FailedTransactionMetadata` with `InvalidInstructionData`. Verified the real program emits `InstructionError(0, InvalidInstructionData)` + log `"invalid instruction data"` (not a false negative). `pnpm test` (2 passed) + `pnpm typecheck` green. Run `just build` (repo root) first to produce the `.so`.

**For D1+:** choose one tx-building style consistently — kit-native (`@solana/kit`: `AccountRole`, `getProgramDerivedAddress`, `compileTransaction`; zero bridge) OR classic web3.js v3 + the `toLiteSvmTransaction` bridge. Both work against the installed deps.

### D1 — constants + PDA derivation + parity guard (DONE 2026-06-30)

**Files added:** `sdk/src/constants.ts`, `sdk/src/pda.ts`, `sdk/test/parity.test.ts`, `sdk/test/pda.test.ts`.

**Verified against the Rust source (code wins; all matched the plan summary — NO discrepancies in the values):**
- `Ix` 0..=21 — verbatim from `instruction.rs` (SubmitFact=0 … CloseMarket=21).
- `AccountType` 0..=7 (Uninitialized=0 … Protocol=7) + `Phase` 0..=8 — from `state.rs`.
- `KassandraError` 0..=30 (NotImplemented=0 … EscrowNotEmpty=30) — from `error.rs`; `decodeError(custom)` maps `Custom(u32)` → `{name,message}`, unknown → `{name:"Unknown"}`.
- `ACCOUNT_SIZES` from `tests/state_layout.rs` `account_sizes_are_stable` (CURRENT, re-read, not stale): Protocol 368, Oracle 392, Proposer 96, Fact 336, FactVote 88, AiClaim 208, Market 416.
- External program IDs (`EXTERNAL_PROGRAM_IDS`) from `cpi/{metadao,metadao_v06}.rs`: conditionalVault `VLTX1ish…` (shared v0.4/v0.6), ammV04 `AMMyu265…`, futarchyV06 `FUTARELBf…`, meteoraDammV2 `cpamdpZC…`, squadsV4 `SQDS4ep6…`.
- Config consts (`CONFIG`): PHASE_WINDOW/PROPOSAL_WINDOW 3600, MAX_PROPOSERS 60, THRESHOLD 2/3, MARKET_THRESHOLD 1/10, FLIP_SLASH 1/2; plus `CLAIM_OPTION_NONE`/`VOTE_*` sentinels.

**PDA API discrepancy vs the task brief:** the task said "`findProgramAddressSync` … (it returns [address, bump])". **web3.js@3.0.0-rc.2 has NO `findProgramAddressSync`** — `Address`/`PublicKey` expose only the ASYNC `static findProgramAddress(seeds, programId): Promise<[Address, number]>` (and async `createProgramAddress`). So every `pda.ts` fn is **async** and returns `Promise<{ address, bump }>`.

**PDA seeds (verified against the processors + `tests/common/mod.rs` `*_pda` helpers):** protocol `[b"protocol"]`, mintAuthority `[b"mint_authority"]`, oracle `[b"oracle", nonce_u64_LE_8]`, stakeVault `[b"vault", oracle]`, proposer `[b"proposer", oracle, authority]`, fact `[b"fact", oracle, content_hash32]`, factVote `[b"vote", fact, voter]`, aiClaim `[b"claim", oracle, proposer]`, market `[b"market", ai_claim]`, challengeUsdcVault `[b"challenge_usdc", market]`. Literal seeds = ASCII bytes (`TextEncoder`), pubkey seeds = `Address.toBytes()` (32 raw), **nonce = `DataView.setBigUint64(…, true)` (LE)**.

**Tests:** `parity.test.ts` (10 tests) hardcodes the program's pinned Ix/AccountType/Phase/error/size values and asserts the SDK constants equal them (drift guard); `pda.test.ts` (14 tests) pins regression base58 anchors (protocol `DUpkpX…`/255, mintAuthority `CyZkoq…`/255, oracle(1) `FZeeaL…`/254, oracle(256) `CQ54BV…`/253), proves nonce-LE (1≠256), determinism, seed order/identity, and the market→escrow chain. `pnpm typecheck` clean; `pnpm test` = 26 passed (parity 10, pda 14, smoke 2).

### D2 — Pod account decoders + offset parity (DONE 2026-06-30)

**Files added:** `sdk/src/accounts/common.ts` (shared readers) + one decoder per account (`protocol.ts`, `oracle.ts`, `proposer.ts`, `fact.ts`, `factVote.ts`, `aiClaim.ts`, `market.ts`) + `sdk/src/accounts/index.ts` (barrel); `sdk/test/accounts.test.ts`.

**Shared read-helper approach (`common.ts`):** one `DataView` per buffer (`view(data)` honoring `byteOffset`/`length`), then `readU8`/`readU16LE`/`readU32LE`/`readU64LE→bigint`/`readI64LE→bigint`/`readBool`, `readBytes(offset,len)→Uint8Array` (copy), and `readPubkey(data,offset)→Address` (`new Address(data.slice(off,off+32))`, web3.js v3 base58). `assertAccount(data, type, size, name)` enforces EXACT length (`=== ACCOUNT_SIZES.X`) + `data[0] === AccountType.X`, throwing a clear error on either — rejects type-confusion (e.g. an Oracle buffer fed to `decodeFact`).

**Per-account field coverage (every non-`_pad` field, at the EXACT pins from `state_layout.rs`):**
- **Protocol (368):** admin/kassMint/usdcMint, feeEma, lastCreationUnix, bump, governanceSet(bool), daoAuthority, kassDao, emission_num/den, totalSupplyCap, fee-EMA params (halflife/perUnit/increment), threshold/marketThreshold/flipSlash num+den, phase/proposalWindow, factVoteSlash num+den, reward proposer/fact weights, challenge fail-USDC + success-KASS fee num+den. (32 fields)
- **Oracle (392, biggest):** creator/mints/stakeVault, deadline/phaseEndsAt/twapWindow, optionsCount, `phaseRaw`+decoded `phase` (Phase enum), proposer/surviving/fact counts, totalOracleStake/bondPool/disputeBondTotal, settled/aiFinalized counts, bump, `resolvedOption` (0xFF dead-end sentinel preserved), openChallengeCount, promptHash[32], the full F2 governable snapshot, the C1 challenge-fee snapshot, S1 totals (totalCorrectProposerStake/totalApprovedFactStake/rewardPool), S3 rewardEmission.
- **Proposer (96):** oracle/authority, bond, originalOption, claimOption (0xFF=none), disqualified/slashed/flipped (bool), bump, aiFinalized (bool), slashedAmount.
- **Fact (336):** oracle/proposer, contentHash[32], stake/approveStake/duplicateStake, uriLen, agreed/duplicate/settled (bool), bump, `uri` (UTF-8 of the first `uri_len` bytes, clamped to 200) + `uriRaw` (full 200 bytes).
- **FactVote (88):** fact/voter, stake, `kindRaw`+decoded `kind` (new `VoteKind{Approve,Duplicate}` enum), bump.
- **AiClaim (208):** oracle/proposer, modelId/paramsHash/ioHash[32], option, challenged (bool), bump, authority (the S4 append @176).
- **Market (416):** all 12 pubkeys (oracle/aiClaim/proposer/challenger/question/kassVault/usdcVault/passAmm/failAmm/oraclePassKass/oracleFailKass/challengerUsdcVault), twapEnd (i64), challengerUsdc (u64), settled (bool), bump.

**REAL-program decode:** `decodeProtocol` is proven against a REAL account — a litesvm test runs the program's `init_protocol` (disc 9; protocol PDA + admin-signer + two `setAccount`-fabricated token-program-owned mints + system program), fetches `svm.getAccount(pda).data`, and asserts the decoded admin/kassMint/usdcMint == the passed keys, bump == the derived PDA bump, and the genesis defaults (feeEma/lastCreationUnix 0, governanceSet false, emissionDen 1, threshold 2/3, challenge fee dens 100). The other 6 are covered by synthetic-buffer round-trips at the exact offsets (Oracle's litesvm path needs the D3 create_oracle builder + KASS mint-authority setup — deferred, synthetic coverage is exhaustive).

**No offset surprises** — every offset matched the `state_layout.rs` pins (computed the non-pinned in-between offsets and they were consistent). Minor API note: web3.js v3 `Address.toString()` returns the base58 string; `new Address(uint8array)` accepts a 32-byte slice. litesvm `getAccount` returns a `MaybeEncodedAccount` (narrow on `.exists` before `.data`); `setAccount` takes a kit `EncodedAccount` (`programAddress` = owner).

**Tests:** `accounts.test.ts` (11 tests) — 7 synthetic round-trips (known values at exact offsets, asserting pubkeys, u64/i64 bigints, LE, Phase/VoteKind enums, the Fact uri slice, the 0xFF sentinels), 3 rejection tests (wrong tag, wrong length, cross-type confusion), 1 real litesvm `init_protocol` decode. `pnpm typecheck` clean; `pnpm test` = 37 passed (parity 10, pda 14, smoke 2, accounts 11).

### D3a — protocol + oracle-lifecycle instruction builders (DONE 2026-06-30)

**Files added:** `sdk/src/instructions/payload.ts` (shared LE payload-writer), `sdk/src/instructions/lifecycle.ts` (the 9 builders + `SetConfigParams`/`encodeSetConfigParams`), `sdk/src/instructions/index.ts` (barrel); `sdk/test/instructions-lifecycle.test.ts`. Added `SYSTEM_PROGRAM_ID` + `TOKEN_PROGRAM_ID` `Address` constants to `constants.ts`.

**Shared payload-writer (`payload.ts`):** small functions returning `Uint8Array` chunks — `u8`, `u16LE`, `u64LE(bigint|number)`, `i64LE(bigint|number)` (DataView `setBig{U,}int64(…, true)`), `pubkeyBytes(AddressInput)` (32 raw via `Address.toBytes()`), `fixedBytes(bytes,len)` (length-checked), `concatBytes(parts)`, and `withDisc(disc, ...payload)` = `[disc, ...payload]`. Builders are **async** (PDA derivation via the async `pda.ts` is awaited internally), returning a classic web3.js v3 `TransactionInstruction({ programId, keys, data })`; `keys` are plain `AccountMeta` `{pubkey, isSigner, isWritable}`. Each builder accepts an ergonomic named-arg object (pubkeys as `Address|string`, ints as `bigint|number`), derives every PDA it can, and takes an optional `programId` override.

**Per-instruction account order (signer S / writable W) + payload — mirrored verbatim from each processor's `# Accounts`/`# Payload` header AND the `tests/common/mod.rs` `*_ix` helpers (matched exactly, NO surprises vs the plan):**
- **InitProtocol (9):** `[0 protocol W, 1 admin W+S, 2 kass_mint ro, 3 usdc_mint ro, 4 system ro]`. Payload: **empty**.
- **CreateOracle (10):** `[0 protocol W, 1 oracle W(PDA), 2 stake_vault W(PDA), 3 creator W+S, 4 kass_mint W, 5 usdc_mint ro, 6 token ro, 7 system ro, 8 creator_kass_token W, 9 mint_authority ro(PDA)]`. Payload **57 B**: `nonce u64 LE ++ prompt_hash[32] ++ options_count u8 ++ deadline i64 LE ++ twap_window i64 LE` (note kass_mint is WRITABLE — burn/emission mutate supply; mint_authority PDA is read-only).
- **Propose (11):** `[0 oracle W, 1 proposer W(PDA), 2 authority W+S, 3 authority_kass W, 4 stake_vault W(PDA), 5 token ro, 6 system ro]`. Payload **9 B**: `option u8 ++ bond u64 LE`.
- **FinalizeProposals (12):** `[0 oracle W, then the FULL proposer set as a READ-ONLY tail]`. Payload: **empty**. (Variable tail: builder appends each given proposer as `ro`.)
- **AdvancePhase (7):** `[0 oracle W]` — **permissionless, no signer**. Payload: empty.
- **SetGovernance (13):** `[0 protocol W, 1 authority ro+S]`. Payload **64 B**: `dao_authority[32] ++ kass_dao[32]` (both pubkeys → 32 raw bytes each).
- **SetConfig (14):** `[0 protocol W, 1 dao_authority ro+S]`. Payload **176 B = 22×8-byte LE** in the FIXED order (set_config.rs indices 0..=21): `emission_num, emission_den, total_supply_cap, fee_ema_halflife (i64), fee_per_ema_unit, fee_ema_increment, threshold_num, threshold_den, market_threshold_num, market_threshold_den, flip_slash_num, flip_slash_den, phase_window (i64), proposal_window (i64), fact_vote_slash_num, fact_vote_slash_den, reward_proposer_weight, reward_fact_weight, challenge_fail_usdc_fee_num, challenge_fail_usdc_fee_den, challenge_success_kass_fee_num, challenge_success_kass_fee_den`. Three i64 fields (indices 3, 12, 13) encode signed; the rest u64. Field order verified against the harness `ConfigParams::to_payload` byte-for-byte.
- **ResolveDeadend (15):** `[0 protocol ro, 1 oracle W, 2 dao_authority ro+S]` — note **protocol is read-only** here (unlike set_config/set_governance). Payload **1 B**: `option u8`.
- **KassPrice (16):** `[0 protocol ro, 1 kass_dao ro]`. Payload: **empty** (read-only; result via return data).

**Surprises vs the plan brief:** none on byte layout. Worth flagging: CreateOracle's `kass_mint` slot is WRITABLE (the brief didn't say); ResolveDeadend's protocol is read-only while set_config/set_governance write it; AdvancePhase + KassPrice + FinalizeProposals carry NO signer. `set_governance`/`set_config`/`resolve_deadend`/`kass_price` derive the singleton protocol PDA internally; the daoAuthority/kassDao in set_governance are pubkeys serialized as 32 raw bytes (NOT re-derived).

**Tests:** `instructions-lifecycle.test.ts` (11 tests) — for each builder, asserts `data` == an INDEPENDENTLY-constructed `[disc, ...payload]` buffer + `keys` == the documented `(pubkey, isSigner, isWritable)` triples with PDA-derived accounts in the right slots; SetConfig uses 22 distinct per-field values (catches any misordering) and cross-checks `encodeSetConfigParams`; plus a `programId`-override test and a **litesvm acceptance test** that builds init_protocol VIA THE SDK, bridges it through `toLiteSvmTransaction`, submits to the real `.so`, and asserts `TransactionMetadata` (accepted) + the resulting Protocol PDA is program-owned. `pnpm typecheck` clean; `pnpm test` = **48 passed** (parity 10, pda 14, smoke 2, accounts 11, instructions 11).

### D3b — dispute + challenge + settlement instruction builders (DONE 2026-06-30)

**Files added:** `sdk/src/instructions/dispute.ts` (submitFact, voteFact, finalizeFacts, submitAiClaim, finalizeAiClaims, finalizeOracle), `sdk/src/instructions/challenge.ts` (openChallenge, settleChallenge), `sdk/src/instructions/settlement.ts` (claimProposer, claimFact, claimFactVote, closeAiClaim, closeMarket); `sdk/test/instructions-dispute.test.ts` (15 tests). `index.ts` now re-exports all three. Hoisted the shared `addr`/`w`/`ro` AccountMeta helpers into `payload.ts` (exported) so all builder modules share them; same async + named-arg + optional-`programId` conventions as D3a. Account orders + payloads mirrored verbatim from each processor's `# Accounts`/`# Payload` header AND cross-checked slot-by-slot against the harness builders (`tests/common/mod.rs` finalize_oracle_ix / claim_*_ix / close_*_ix; `tests/settlement_e2e.rs` submit/vote/finalize fact + ai-claim ix; `tests/challenge_e2e.rs` open_challenge_ix / settle_ix).

**Per-instruction account order (S signer / W writable) + payload:**
- **SubmitFact (0):** `[0 oracle W, 1 fact W(PDA `[b"fact",oracle,content_hash]`), 2 submitter W+S, 3 submitter_kass W, 4 stake_vault W(PDA), 5 token ro, 6 system ro]`. Payload **variable**: `content_hash[32] ++ stake u64 LE ++ uri_len u16 LE ++ uri[uri_len]` (builder accepts `uri: string|Uint8Array`, utf-8-encodes a string, writes the u16 length then the bytes; on-chain cap is 200).
- **VoteFact (1):** `[0 oracle W, 1 fact W, 2 fact_vote W(PDA `[b"vote",fact,voter]`), 3 voter W+S, 4 voter_kass W, 5 stake_vault W(PDA), 6 token ro, 7 system ro]`. Payload **9 B**: `kind u8 ++ stake u64 LE`.
- **FinalizeFacts (2):** `[0 oracle W, then a WRITABLE tail]` — a non-empty subset of the oracle's Fact PDAs, or its Proposer PDAs in the no-facts dead-end branch (harness uses `AccountMeta::new`=writable for the tail). Payload **empty**.
- **SubmitAiClaim (3):** `[0 oracle W, 1 proposer W, 2 ai_claim W(PDA `[b"claim",oracle,proposer]`), 3 authority W+S, 4 system ro]`. Payload **97 B**: `model_id[32] ++ params_hash[32] ++ io_hash[32] ++ option u8`.
- **FinalizeAiClaims (8):** `[0 oracle W, then a WRITABLE proposer-subset tail]`. Payload **empty**.
- **FinalizeOracle (6):** `[0 oracle W(PDA), 1 kass_mint W, 2 stake_vault W(PDA), 3 token ro, then the FULL proposer set as a READ-ONLY tail (`proposer_count`)]`. Payload **8 B**: `oracle_nonce u64 LE` (S3-added — re-derives the oracle PDA that signs the InvalidDeadend emission burn-back). NOTE the 3 fixed burn accounts (kass_mint/stake_vault/token) precede the tail and are required on BOTH terminal branches; the proposer tail is **read-only** here (unlike finalize_facts/finalize_ai_claims).
- **ClaimProposer (17):** `[0 oracle ro, 1 proposer W(closed), 2 dest_kass W, 3 stake_vault W(PDA), 4 rent_recipient W, 5 token ro]`. Payload **8 B**: `oracle_nonce u64 LE`.
- **ClaimFact (18):** same shape as ClaimProposer with the `Fact` at index 1. Payload `oracle_nonce u64 LE`.
- **ClaimFactVote (19):** `[0 oracle ro, 1 fact_vote W(closed), 2 fact W, 3 dest_kass W, 4 stake_vault W(PDA), 5 rent_recipient W, 6 token ro]` — inserts the fact at index 2 (its running voter-stake total is decremented, NOT closed). Payload `oracle_nonce u64 LE`.
- **CloseAiClaim (20):** `[0 oracle ro, 1 ai_claim W(closed), 2 rent_recipient W]` — **payload EMPTY**, NO token program, NO Proposer (S4 fix: rent recipient read off `ai_claim.authority`).
- **CloseMarket (21):** `[0 oracle ro, 1 market W(closed), 2 challenger_usdc_vault W(PDA, closed), 3 rent_recipient W, 4 token ro]`. Payload **8 B**: `oracle_nonce u64 LE`.

**OpenChallenge (4) — 25 accounts, payload `oracle_nonce u64 LE` (8 B).** The C1 rework moved escrow sizing on-chain (`kass_price`), so the payload is nonce-only and the account list grew the protocol/kass_dao/usdc_mint/challenger-USDC-src/escrow tail. Full slot map (`*`=SDK-derived PDA, rest caller-supplied):
`[0 oracle* W, 1 ai_claim* W (`[b"claim",oracle,proposer]`), 2 proposer W, 3 market* W (`[b"market",ai_claim]`), 4 challenger W+S, 5 question ro, 6 kass_vault W, 7 usdc_vault ro, 8 pass_amm ro, 9 fail_amm ro, 10 stake_vault* W (`[b"vault",oracle]`), 11 kass_vault_underlying W, 12 pass_kass_mint W, 13 fail_kass_mint W, 14 oracle_pass_kass W, 15 oracle_fail_kass W, 16 conditional_vault program ro, 17 token ro, 18 system ro, 19 cv_event_authority ro, 20 protocol* ro (`[b"protocol"]`), 21 kass_dao ro, 22 usdc_mint ro, 23 challenger_usdc_src W, 24 challenger_usdc_vault* W (`[b"challenge_usdc",market]`)]`. **SDK-derived:** oracle (from nonce), ai_claim, market, stake_vault, protocol, escrow. **Caller-supplied (MetaDAO market composed in the caller's own txs, exactly as the harness does):** proposer, challenger, question, kass_vault, usdc_vault, pass_amm, fail_amm, kass_vault_underlying, pass/fail_kass_mint, oracle_pass/fail_kass, cv_event_authority, kass_dao, usdc_mint, challenger_usdc_src.

**SettleChallenge (5) — 21 accounts, payload `oracle_nonce u64 LE` (8 B).** The C2 rework added the physical redeem + directional-fee accounts (stake_vault/kass_vault/underlying/mints/holders/escrow/proposer-USDC/challenger-USDC/challenger-KASS). Full slot map:
`[0 oracle* W, 1 market* W (`[b"market",ai_claim]`), 2 ai_claim ro, 3 proposer W, 4 question W, 5 pass_amm ro, 6 fail_amm ro, 7 conditional_vault program ro, 8 cv_event_authority ro, 9 token ro, 10 stake_vault* W, 11 kass_vault W, 12 kass_vault_underlying W, 13 pass_kass_mint W, 14 fail_kass_mint W, 15 oracle_pass_kass W, 16 oracle_fail_kass W, 17 challenger_usdc_vault* W (`[b"challenge_usdc",market]`), 18 proposer_usdc W, 19 challenger_usdc_dest W, 20 challenger_kass W]`. **SDK-derived:** oracle (from nonce), market (from the caller-supplied ai_claim), stake_vault, escrow. **Caller-supplied:** ai_claim, proposer, question, pass/fail_amm, cv_event_authority, kass_vault, kass_vault_underlying, pass/fail_kass_mint, oracle_pass/fail_kass, proposer_usdc, challenger_usdc_dest, challenger_kass. NOTE the program-order quirk: the conditional-vault program + cv_event_authority sit at slots 7-8 (between fail_amm and token program), and token program is slot 9 — NOT grouped with the other fixed ids.

**Surprises vs the plan brief:** none on byte layout. Confirmations worth flagging: FinalizeFacts/FinalizeAiClaims tails are WRITABLE while the FinalizeOracle tail is READ-ONLY; CloseAiClaim has no token program and no Proposer (payload empty); OpenChallenge/SettleChallenge interleave the conditional-vault program/event-authority among the market accounts rather than at the end. All MetaDAO accounts (question/vaults/AMMs/conditional mints/event authority) are caller-supplied — the SDK never composes the MetaDAO market, matching the harness.

**Tests:** `instructions-dispute.test.ts` (15 tests) — each builder asserts `data` == an independently-built `[disc, ...payload]` buffer + `keys` == the documented triples with PDAs in the right slots; SubmitFact covers both string and raw-`Uint8Array` uris (independent u16-len framing); OpenChallenge (25) and SettleChallenge (21) are asserted slot-by-slot with explicit length checks; plus a `programId`-override test. `pnpm typecheck` clean; `pnpm test` = **63 passed** (parity 10, pda 14, smoke 2, accounts 11, lifecycle 11, dispute 15).

### D4 — litesvm end-to-end lifecycle via the SDK (DONE 2026-06-30)

**File added:** `sdk/test/e2e.test.ts` (2 tests). This is the proof the SDK matches the deployed program end-to-end: every instruction is built by an SDK builder (`src/instructions/*`), bridged via `toLiteSvmTransaction`, submitted to the REAL `target/deploy/kassandra_program.so`, and the resulting accounts are decoded by the SDK decoders (`src/accounts/*`) — no synthetic buffers. Mirrors the Rust `tests/lifecycle_e2e.rs` (`e2e_happy_uncontested_resolves` + the `FactProposal` slice of `dispute_via_real_flow`).

**Flow covered (happy / uncontested-resolve):** (1) `initProtocol` → decode Protocol, assert admin/kassMint/usdcMint. (2) `createOracle` (nonce 1, 3 options, near deadline `now+1000`, twap 600) → decode Oracle, assert `phase==Proposal`, creator/mints/optionsCount/promptHash, `proposerCount==0`. (3) `propose` ×3, ALL on option 1, each from a distinct funded authority holding its own KASS token account → decode each Proposer, assert `bond`, `originalOption==1`, `claimOption==CLAIM_OPTION_NONE (0xff)`, not disqualified/slashed. (4) warp past the proposal window. (5) `finalizeProposals` with the full proposer set → decode Oracle, assert `phase==Resolved`, `resolvedOption==1`, `disputeBondTotal==0`. KASS conservation asserted at the proposal boundary by decoding the stake-vault SPL `amount`: `stake_vault == total_oracle_stake == Σ bonds`, and the vault is untouched after the resolve (no token CPI on that path).

**Dispute slice (also done):** `propose` ×2 on DISTINCT options 0/1 → `finalizeProposals` lands the oracle in `Phase::FactProposal` with `disputeBondTotal == total_oracle_stake == Σ bonds` (the fixed fact-quorum denominator). The heavy MetaDAO challenge/market path is OUT OF SCOPE (covered by the Rust suite — the SDK never composes the MetaDAO market).

**Clock-warp approach:** litesvm exposes `getClock(): Clock` / `setClock(Clock)`. The program's `now()` reads `Clock.unix_timestamp` and every phase gate compares `now >= phase_ends_at`, so advancing time = constructing a fresh `Clock` with `unixTimestamp + Δ` (and `slot + 1`) and calling `setClock` — exactly what the Rust harness `warp` does via `set_sysvar::<Clock>`. Used twice per oracle: `+1000` to reach the deadline (open the proposal window, since `create_oracle` sets `phase_ends_at = deadline + PROPOSAL_WINDOW`), then `+3601` to cross `phase_ends_at` before finalize.

**SPL-funding approach:** mirrors the Rust harness, which fabricates SPL bytes directly via `set_account` (never runs `InitializeMint`). `mintBytes` (82-byte `Mint`: COption-Some authority, supply, decimals, is_initialized) and `tokenAccountBytes` (165-byte `Account`: mint, owner, amount, state=Initialized) pack the canonical spl-token layouts; `svm.setAccount` writes them token-program-owned. The program's own CPIs — `create_oracle`'s `InitializeAccount3` on the stake vault, `propose`'s `Transfer` — run against the SPL Token program that `new LiteSVM()` loads by default (`withDefaultPrograms`, confirmed in the litesvm `.d.ts`). KASS mint authority = the mint-authority PDA (mirrors the harness bootstrap) but it is NOT load-bearing: emissions are default-disabled (`emission_num == 0`) so `create_oracle` mints nothing and the `BadMintAuthority` guard is skipped (it only fires when `reward_emission > 0` — confirmed in `create_oracle.rs`); the genesis creation fee is likewise 0 (`fee_ema == 0`) so no KASS is burned and the creator's KASS account is never read. So a plain mint authority would also work — documented in the test.

**SDK bugs / program mismatches:** NONE. Every SDK builder was accepted by the real program first try and every decoder read the real account state correctly — no builder/decoder fix was needed, and no program mismatch surfaced. (The D0–D3 builders + decoders held up end-to-end.)

**Tests:** `e2e.test.ts` (2 tests — happy resolve + dispute slice). `pnpm typecheck` clean; `pnpm test` = **65 passed** (parity 10, pda 14, smoke 2, accounts 11, lifecycle 11, dispute 15, e2e 2).

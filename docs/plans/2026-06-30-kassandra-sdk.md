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

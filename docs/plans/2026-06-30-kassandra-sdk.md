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

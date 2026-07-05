# On-chain Oracle Metadata — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Store an oracle's subject + option labels on-chain in a companion
`oracle_meta` PDA (program-readable), remove `prompt_hash`, and host the extended
metadata JSON (prompt template, interpretation) off-chain behind a hash-bound URL
served by the public app server, indexed by the (private) indexer.

**Architecture:** Companion variable-sized PDA `[b"oracle_meta", oracle]` written
by a new `write_oracle_meta` instruction (write-once), closed at `sweep_oracle`.
The chain is authoritative; the indexer keeps indexing + the app server fronts a
`uri` JSON host validated against the on-chain `uri_hash`. Full design:
`docs/plans/2026-07-05-onchain-oracle-metadata-design.md`.

**Tech Stack:** Pinocchio (Solana, no-Anchor), LiteSVM tests, Rust SDK
(`kassandra-sdk`), TS SDK (`@kassandra/sdk`), Carbon indexer (axum + Postgres),
React/Vite app.

**Conventions:** TDD where practical (LiteSVM + vitest). `just build` before
`cargo test -p kassandra-program` (LiteSVM `include_bytes!` the `.so`). Commit at
the end of each task. Run `cargo fmt` + `pnpm --filter ./app lint` before commits.

---

## Phase 1 — Program: `oracle_meta` account + `write_oracle_meta` + drop `prompt_hash`

### Task 1.1 — `AccountType::OracleMeta` + `Ix::WriteOracleMeta`

**Files:**
- Modify: `programs/kassandra/src/state.rs` — add `OracleMeta = 8` to `AccountType`.
- Modify: `programs/kassandra/src/instruction.rs` — add `WriteOracleMeta = 23` to
  `Ix` (append; never renumber) + its `from_u8` arm.
- Modify: `programs/kassandra/src/processor/mod.rs` — add
  `Ix::WriteOracleMeta => write_oracle_meta::process(...)` + `mod write_oracle_meta;`.

**Steps:** add the variants → `cargo build -p kassandra-program` (expect an
unresolved `write_oracle_meta` module error, fixed in 1.3) → commit after 1.3.

### Task 1.2 — Remove `prompt_hash` from `Oracle` + `create_oracle`

**Files:**
- Modify: `programs/kassandra/src/state.rs` — delete `pub prompt_hash: [u8; 32]`
  from `Oracle` (subsequent offsets shift; update the struct docstring/LEN note).
- Modify: `programs/kassandra/src/processor/create_oracle.rs` — remove the
  `prompt_hash` parse (`payload[8..40]`) and the `oracle.prompt_hash = …` write;
  shift the payload layout to `nonce[8] ++ options_count[1] ++ deadline[8] ++
  twap_window[8]` (57 → 25 bytes); update `EXPECTED_LEN` + the doc comment.

**Test:** update `programs/kassandra/tests/*` create-oracle payload builders that
hardcode 57 bytes / a prompt_hash. Grep: `rg "prompt_hash|40\]|57" programs/kassandra/tests`.

**Verify:** `just build && cargo test -p kassandra-program` (create-oracle tests
green with the shorter payload).

### Task 1.3 — `write_oracle_meta` processor (the core)

**Files:**
- Create: `programs/kassandra/src/processor/write_oracle_meta.rs`
- Test: `programs/kassandra/tests/oracle_meta.rs`

**Account layout** (variable; written as raw bytes, NOT a Pod struct):
```
0   account_type u8   = AccountType::OracleMeta
1   bump u8
2   oracle Pubkey (32)
34  subject_len u16
36  subject bytes
    options_count u8
    repeated: option_len u16 ++ option bytes
    uri_len u16
    uri bytes
    uri_hash [u8;32]
```
Caps (reject over): `MAX_SUBJECT_LEN = 512`, `MAX_OPTION_LEN = 128`,
`MAX_URI_LEN = 256`. Encoded-size helper computes exact bytes.

**Instruction payload** = the same body after the header (subject_len ++ subject
++ options_count ++ options ++ uri_len ++ uri ++ uri_hash[32]).

**Accounts:** `[creator(signer,w), oracle(ro), oracle_meta(w,PDA), system_program]`.

**Processor logic (model on `submit_fact.rs`):**
1. Parse the payload with a length-prefixed parser (reject trailing bytes,
   over-cap lengths, `options_count == 0`).
2. `load_oracle(oracle_ai, program_id)` (owner/type/size check); require
   `parsed.options_count == oracle.options_count` (`KassandraError::InvalidOptionsCount`).
3. Derive `[b"oracle_meta", oracle_ai.address()]`; `assert_key(meta_ai, expected)`.
   Write-once: `if meta_ai.lamports() != 0 || !meta_ai.is_data_empty()` →
   `KassandraError::AlreadyInitialized`.
4. `let space = encoded_len(&parsed); create_pda(creator_ai, meta_ai, seeds,
   minimum_rent(space)?, space, program_id)`.
5. Borrow `meta_ai.try_borrow_mut()` and write the header + body bytes.

**Add error** if needed: reuse `KassandraError::AlreadyInitialized` +
`InvalidOptionsCount` (already exist — verify names in `error.rs`).

**Tests (`tests/oracle_meta.rs`, LiteSVM):**
- `writes_and_reads_back`: create oracle + write_oracle_meta; fetch the PDA,
  parse, assert subject/options/uri/uri_hash round-trip.
- `write_once`: a second write_oracle_meta fails.
- `options_count_must_match`: labels count ≠ oracle.options_count fails.
- `rejects_oversize` / `rejects_trailing_bytes`: parser guards.

**Verify:** `just build && cargo test -p kassandra-program oracle_meta`.
**Commit:** `feat(program): oracle_meta account + write_oracle_meta; drop prompt_hash`.

### Task 1.4 — `sweep_oracle` closes `oracle_meta`

**Files:**
- Modify: `programs/kassandra/src/processor/sweep_oracle.rs` — accept an extra
  `oracle_meta_ai` account (optional/last); if present + owned + PDA-correct +
  non-empty, `drain_lamports(meta_ai, creator_ai)` + `close()` (mirror the Oracle
  close at ~line 173). Tolerate its absence (oracles created without meta).

**Test:** extend a sweep test: after sweep, `oracle_meta` is closed and its rent
landed on `creator`.
**Verify + commit:** `feat(program): sweep_oracle reclaims oracle_meta rent`.

---

## Phase 2 — Rust SDK (`sdk-rs`)

**Files:** `sdk-rs/src/{pda,instructions,accounts}.rs` (match existing module layout).
- `pda::oracle_meta(oracle) -> [b"oracle_meta", oracle]`.
- `write_oracle_meta(args) -> Instruction` builder (encode the length-prefixed body).
- `OracleMeta` decoder: parse the account bytes → `{ oracle, subject, options,
  uri, uri_hash }`.
- Remove `prompt_hash` from the `Oracle` decoder; adjust offsets.

**Test:** a Rust unit test encoding a `write_oracle_meta` body and decoding it back
(round-trip), plus an `Oracle` decode without `prompt_hash`.
**Verify:** `cargo test -p kassandra-sdk`. **Commit** per module.

---

## Phase 3 — TS SDK (`@kassandra/sdk`, dir `sdk/`)

**Files:** `sdk/src/{pda.ts,instructions/lifecycle.ts,accounts/oracle.ts}` + a new
`accounts/oracleMeta.ts`.
- `pda.oracleMeta(oracle)`.
- `writeOracleMeta({ oracle, creator, subject, options, uri, uriHash })` builder
  (mirror `createOracle`; encode the length-prefixed body via the existing byte
  helpers `u16`, `fixedBytes`, etc.).
- `decodeOracleMeta(data)` → `{ oracle, subject, options, uri, uriHash }`.
- Remove `promptHash` from `accounts/oracle.ts` (drop the `readBytes(…,200,32)` +
  the field; shift any later offsets).
- Drop `promptHash` from `CreateOracleArgs` + the payload writer.

**Test:** `sdk/test/*` — a vitest asserting `writeOracleMeta` bytes + a
`decodeOracleMeta` round-trip; update any create-oracle byte-parity test.
**Verify:** `pnpm --filter ./sdk build && pnpm --filter ./sdk test`. **Commit.**

---

## Phase 4 — Indexer: index `oracle_meta` + JSON host (replace memo path)

**Files:** `indexer/src/{decoder.rs (account decode? currently tx-crawler),db.rs,api.rs,processor.rs}`.

NOTE: the oracle indexer is a TRANSACTION crawler (events), not an account
indexer. `oracle_meta` is an ACCOUNT. Two options — pick the lighter:
- **(a)** In the tx processor, on `write_oracle_meta`, decode the account from the
  ix data (the payload IS the body) + accounts[2] (the meta PDA) → upsert
  `oracle_metadata` (subject, options, uri, uri_hash). No new datasource. **Preferred.**
- (b) Add a gpa/account pipeline for `OracleMeta` (heavier).

**Changes (option a):**
1. `db.rs`: repurpose the `oracle_metadata` table → columns `(oracle PK, subject,
   options JSONB, uri, uri_hash, slot, signature)`. Add `insert_oracle_meta` (from
   the ix) + `get/list` (already exist from the memo work — adapt).
2. `processor.rs`: **remove the SPL-memo capture**; instead, on
   `ix.name == "write_oracle_meta"`, parse the ix `data` (length-prefixed body) +
   `ix.accounts[2]` (oracle_meta PDA) — actually key by the ORACLE = `ix.accounts[1]`
   — and `insert_oracle_meta`.
3. `api.rs`: keep `/oracles/meta` + `/oracles/{pubkey}/meta`. Add the JSON host:
   - `POST /api/oracle/{oracle}/metadata.json` — store the posted JSON (new
     `oracle_meta_json` table: `(oracle PK, json JSONB, sha256 TEXT)`).
   - `GET  /api/oracle/{oracle}/metadata.json` — serve it, but only if
     `sha256(json) == on-chain uri_hash` from `oracle_metadata` (else 409/404).

**Remove:** `extract_memo`, the memo processor tests, the memo-specific columns.
**Test:** `cargo test -p kassandra-indexer` (parser + a hash-gate unit test).
**Verify + commit** per step.

---

## Phase 5 — App: create flow + reads (replace memo + hash-verify)

**Files:** `app/src/data/actions/create.ts`, `app/src/pages/CreateOracle.tsx`,
`app/src/hooks/useOracleMeta.ts`, `app/src/data/indexer.ts`,
`app/src/pages/{Oracles,OracleDetail}.tsx`, `app/server.mjs` (verify `/api` proxy
covers `/api/oracle/*`).

1. `create.ts`: remove `buildOracleMetaMemoIx`; add the metadata bundle:
   - Build the JSON `{ version, subject, options, promptTemplate, interpretation?,
     category?, createdAt }`; `uriHash = sha256(canonical json)`;
     `uri = ${origin}/api/oracle/${oracle}/metadata.json`.
   - Emit `writeOracleMeta` ix into `ixs` (replacing the memo).
   - Return the json + uri so the page can `POST` it after send.
2. `CreateOracle.tsx`: keep the option-label list; add an **Advanced** disclosure
   with `promptTemplate` (defaulted) + `interpretation` + `category`; remove the
   prompt-hash preview. On success, `POST` the JSON to `uri`.
3. `useOracleMeta.ts`: stop calling the indexer + drop the hash-verify; read the
   `oracle_meta` **account from chain** (getAccountInfo on the PDA; decode via SDK).
   For the browse list keep the indexer `/oracles/meta` (fast) as-is.
4. `Oracles.tsx` / `OracleDetail.tsx`: unchanged rendering (subject + option chips),
   sourced from the new reads. OracleDetail additionally fetches `uri`→JSON for the
   Advanced fields, verifying `sha256 == uri_hash`.

**Verify:** `pnpm --filter ./app typecheck && pnpm --filter ./app test &&
pnpm --filter ./app build`. **Commit** per step.

---

## Phase 6 — `make dev` seeding + e2e + cleanup

1. `app/e2e/seed.ts` `createOracleReal`: replace the memo (`buildOracleMetaMemoIx`)
   with `writeOracleMeta` (subject + generated labels) — same tx — and `POST` a
   minimal JSON to the local indexer host.
2. Remove the now-dead memo helpers/tests across app + indexer.
3. Surfpool smoke (temp, then delete): create-with-meta → read `oracle_meta`
   on-chain (decode) → indexer `/oracles/{pk}/meta` returns it → the JSON host
   serves the hash-verified JSON.
4. Full gate: `just build && cargo test --workspace`,
   `pnpm --filter ./sdk build && pnpm --filter ./app build && pnpm --filter ./app test`.

**Commit** the cleanup; open the PR when green.

---

## Risks / watch-outs

- **Offset shifts** from removing `prompt_hash`: update BOTH SDK decoders + any
  test that hardcodes an offset ≥ the old `prompt_hash` position. Grep `rg "200|promptHash|prompt_hash"`.
- **LiteSVM stale `.so`**: always `just build` before `cargo test -p kassandra-program`.
- **Runner**: it takes the prompt via config today; nothing reads `oracle.prompt_hash`,
  so removal is safe. (Optional later: read subject from `oracle_meta`.)
- **dev vs prod uri**: `origin` is localhost in dev (fine for dev oracles), the app
  domain in prod.

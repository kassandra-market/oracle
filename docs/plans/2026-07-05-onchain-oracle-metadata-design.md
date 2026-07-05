# On-chain oracle metadata — design

**Date:** 2026-07-05
**Status:** Design (supersedes the memo-based metadata capture shipped in `0e0e7d0`)

## Goal

Rework how an oracle's descriptive metadata (subject, option labels, prompt
template, interpretation) is stored so that:

- **On-chain programs** can read the fields they need (subject, option labels)
  directly from account bytes — no dependency on our indexer, no URL to
  dereference (a program can't fetch a URL).
- **Off-chain clients** can read everything from chain + a URL-referenced JSON,
  again without needing *our specific* indexer.
- Long, rarely-program-read data (prompt template, interpretation) lives off-chain
  behind a URL, so it doesn't cost permanent rent — while short-lived / eventually
  closed accounts keep their bytes on-chain because the rent is reclaimed at close.

The chain is the source of truth; the indexer becomes a convenience + a host, not
a requirement for the core data.

## Decisions (from brainstorming)

- Consumers split by field: **subject + option labels → on-chain bytes**;
  interpretation / prompt template / extras → off-chain JSON behind a URL.
- Storage mechanism: a **companion `oracle_meta` PDA sized to fit** (not inline
  fixed buffers, not a realloc tail on `Oracle`).
- **Remove `prompt_hash` from `Oracle`** — it's written at `create_oracle` and
  never read on-chain; the on-chain subject is now authoritative, so the hash is
  redundant.
- The indexer **keeps indexing** metadata (now from the `oracle_meta` account, not
  a memo).
- The public URI host is the **existing public app web service** (`server.mjs`),
  which already proxies `/api/*` to the private indexer — the indexer is a Render
  `pserv` (private) and cannot serve a public URL itself.
- Metadata JSON schema: `{ version, subject, options, promptTemplate,
  interpretation?, category?, createdAt }` — `promptTemplate` defaulted, behind an
  "Advanced" disclosure in the create form.

## The `oracle_meta` account

A program-owned PDA **`[b"oracle_meta", oracle_pubkey]`**, created alongside the
oracle and allocated to the **exact** encoded size (rent ∝ bytes). `Oracle` stays
a fixed-size zero-copy Pod; `oracle_meta` is a **length-prefixed variable buffer**
parsed with explicit offsets (the existing `submit_fact` `uri` pattern), because
zero-copy Pod can't hold variable arrays.

```
account_type : u8         // AccountType::OracleMeta
bump         : u8
oracle       : Pubkey     // back-reference (32)
subject_len  : u16
subject      : [u8]       // UTF-8 question text
options_count: u8
options      : repeated { len: u16, bytes: [u8] }   // one per outcome
uri_len      : u16
uri          : [u8]       // {app-origin}/api/oracle/{oracle}/metadata.json (or any host)
uri_hash     : [u8; 32]   // sha256 of the canonical off-chain JSON; zeroed if no uri
```

- **Programs** derive the PDA and parse `subject` / `options` directly.
- **Off-chain clients** read those from chain too, then fetch `uri`→JSON and verify
  `sha256(json) == uri_hash`.
- **Rent:** the creator pays; reclaimed at `sweep_oracle`.

## Instructions & lifecycle

- **`write_oracle_meta`** (new): accounts `creator (signer)`, `oracle (ro)`,
  `oracle_meta (w, PDA, created here)`, `system_program`. Payload: length-prefixed
  `subject` + `options` + optional `uri`/`uri_hash`. The SDK/app send it in the
  **same transaction** as `create_oracle` (atomic — created and described
  together). A raw caller can still create an oracle without meta; clients degrade
  gracefully.
- **Write-once / immutable:** refuses if `oracle_meta` already exists — the subject
  and options are the oracle's identity, so on-chain reads are trustworthy
  (program-owned, unchangeable). No update path.
- **Consistency check:** `write_oracle_meta` requires the meta's `options_count ==
  oracle.options_count`.
- **`create_oracle`:** drop `prompt_hash` from the payload (57 → 25 bytes) and from
  the `Oracle` struct (subsequent field offsets shift).
- **`sweep_oracle`:** also close `oracle_meta` (lamport drain → `oracle.creator`,
  `close()`), passed as an extra account — metadata rent reclaimed at end-of-life.

## Indexer + the public URI host

- **On-chain uri:** `{app_public_origin}/api/oracle/{oracle}/metadata.json`. The app
  sets it at creation (it knows its own origin + the derived oracle PDA).
- **App web service (`server.mjs`, public):** its existing `/api/*` proxy already
  fronts the private indexer for both `GET` (serve the JSON) and the app's `POST`
  (store it) — essentially no new plumbing.
- **Indexer (private `pserv`), two jobs:**
  1. **Keep indexing** — read `oracle_meta` accounts into Postgres (`subject`,
     `options`, `uri`, `uri_hash` per oracle). `/oracles/meta` still powers the
     browse list.
  2. **JSON host** — accept the app's `POST` of the full metadata JSON (keyed by
     oracle), store it, and serve it. It **self-validates**: once it has indexed the
     oracle's on-chain `uri_hash`, it serves the JSON only if
     `sha256(stored) == uri_hash` (rejects spam / mismatch).
- **Trust model:** clients read `subject`/`options` from chain (authoritative) and
  verify the fetched JSON against on-chain `uri_hash`, so hosting it ourselves is
  tamper-evident, not trusted blindly. A non-app creator can point `uri` at their
  own host / IPFS.

## Create flow & metadata bundle

```jsonc
{
  "version": 1,
  "subject": "…",            // on-chain (bytes) + in JSON
  "options": ["Yes", "No"],  // on-chain (bytes) + in JSON
  "promptTemplate": "…",     // AI-runner interpretation template — DEFAULTED
  "interpretation": "…",     // human resolution rules (optional)
  "category": "…",           // optional tag a client can filter on
  "createdAt": 1712345678
}
```

1. The form collects `subject`, `options`, and the extended fields.
   `promptTemplate` is pre-filled with a sensible categorical-resolution default
   templated over `{subject}`/`{options}`/`{interpretation}`, editable behind an
   "Advanced" disclosure so the common case is one click.
2. App canonicalizes the JSON, computes `uri_hash = sha256(json)`, sets
   `uri = {origin}/api/oracle/{oraclePubkey}/metadata.json`.
3. Tx: `create_oracle` + `write_oracle_meta(subject, options, uri, uri_hash)`.
4. `POST {uri}` with the JSON → app server → private indexer stores it (later
   verified against the indexed on-chain `uri_hash`).

## Reading (app)

- **Detail:** read the single `oracle_meta` account **directly from chain**
  (authoritative), and fetch `uri`→JSON for the Advanced fields (prompt template /
  interpretation), verifying against `uri_hash`.
- **Browse:** use the indexer's indexed meta (fast, queryable) with a chain
  `getProgramAccounts(OracleMeta)` fallback — verifiable either way.
- The prior `useOracleMeta` hash-verification against `prompt_hash` is removed
  (the on-chain bytes are authoritative).

## Migration & rollout

Supersedes the memo path from `0e0e7d0`: remove `buildOracleMetaMemoIx`, the memo
capture in the indexer processor, and the memo `oracle_metadata` table; the app
create flow switches from the SPL memo to `write_oracle_meta` + the JSON `POST`;
`make dev` seeding switches from memos to `write_oracle_meta` + a POST.

This is a **breaking on-chain layout change** (Oracle struct, new account, new
instruction, shorter `create_oracle` payload). Pre-mainnet, so a clean break — no
migration of existing oracles.

## Testing

- **Program (LiteSVM):** write/round-trip decode; immutable (second write fails);
  `options_count` consistency; `sweep_oracle` closes `oracle_meta` with rent →
  `creator`; `Oracle` decodes with `prompt_hash` gone; fuzz the length-prefixed
  parser (truncated / oversized).
- **SDK parity (Rust + TS):** byte-exact `OracleMeta` decode; `Oracle` without
  `promptHash`; `writeOracleMeta` + `oracleMeta` PDA builders.
- **Indexer:** indexes `oracle_meta`; the JSON host validates
  `sha256 == uri_hash` before serving; `/oracles/meta` still powers browse.
- **App / e2e:** create → subject/options on-chain + JSON POST lands; detail reads
  `oracle_meta` from chain + `uri`→JSON; browse via indexed meta. Surfpool smoke:
  create-with-meta → read `oracle_meta` on-chain → indexer serves the
  hash-verified JSON.

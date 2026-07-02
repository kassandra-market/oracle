# Kassandra UI

The web UI for **Kassandra** — a decentralized, AI-assisted **optimistic oracle** on Solana.
Vite + React 19 + TypeScript + Tailwind v4 SPA, styled in the **Delphi** visual language
("warm parchment editorial with ember sparks").

## Run / build

> **Build the SDK first.** The app links `@kassandra/sdk` via the pnpm workspace and
> resolves its types from `sdk/dist/` (which is gitignored). On a fresh clone, run
> `pnpm --filter @kassandra/sdk build` (or `pnpm -r build`) **before** the app's
> typecheck/build, or the SDK import won't resolve. (Slice 1 only link-proofs the
> import; the functional-dApp milestone will depend on the SDK types for real, so CI
> must build the SDK first.)

```bash
pnpm --filter @kassandra/sdk build   # build the SDK first (types → sdk/dist)
pnpm --filter app dev        # dev server (HMR)
pnpm --filter app typecheck  # tsc -b
pnpm --filter app lint       # oxlint
pnpm --filter app build      # tsc -b && vite build && verify-css guard
pnpm --filter app preview    # serve the production build
```

`build` runs `scripts/verify-css.mjs` after `vite build`: it asserts the Tailwind v4
`@tailwindcss/vite` plugin actually compiled (real utilities + lowered `@theme` vars in the
emitted CSS, no literal `@theme{}`/`@tailwind` leaks). If it fails, the app would ship unstyled.

Fonts are bundled locally via `@fontsource` (Cormorant Garamond 300/400, Inter 400/500,
Roboto Mono 400) — the build is fully offline (no hotlinked CDNs or images).

## Routes

- `/` — the Kassandra landing page (`src/pages/Landing.tsx`).
- `/oracles` — the oracle browser (`src/pages/Oracles.tsx`): a responsive grid of Delphi
  cards, one per decoded on-chain oracle (phase chip, relative deadline, proposer/fact/option
  counts, resolved option). Read-only.
- `/oracles/:pubkey` — the oracle detail view (`src/pages/OracleDetail.tsx`): an editorial
  layout of one oracle + its facts, proposers, AI claims, and challenge market, with
  copy-on-click truncated pubkeys/hashes. Read-only browsing works fully disconnected; the
  wallet-signed **write forms** (below) are additive on top.

### RPC / cluster config

The browse views read the chain through the connection wired in FA1: the NavBar cluster
selector (`localnet` / `devnet` / `mainnet-beta`, persisted in `localStorage`) drives the
`Connection` from `useConnection()`. Localnet resolves to `VITE_RPC_URL` (default
`http://127.0.0.1:8899`). The data layer (`src/data/oracles.ts`, FA2) enumerates + decodes
oracle accounts via `getProgramAccounts`; the query hooks (`src/hooks/useOracles.ts`) wrap it
with loading/error/refetch and re-fetch when the cluster/connection changes.

**Point at a seeded surfpool:** run the FA2 gated integration test's seed flow (surfpool on
`127.0.0.1:8899` with the program deployed + oracles seeded — see
`app/test/oracle-data.e2e.test.ts`), then `pnpm --filter app dev` with the cluster on
**Localnet** (or `VITE_RPC_URL` pointed at the surfpool RPC) and open `/oracles`.

### Write flows (wallet-signed)

The dApp covers the **full oracle lifecycle** as wallet-signed actions, each gated on a connected
wallet **and** the oracle's current phase. Every action wraps a pure `build*Ixs` action layer
(`src/data/actions/*.ts` + `src/data/actions.ts`) and sends via wallet-adapter's `sendTransaction`.

**Create** (`/oracles/new`, linked from the list): a question (hashed to the on-chain
`prompt_hash`) + options count + deadline + KASS/USDC mints (defaulted from the Protocol) → a new
oracle; navigates to its detail on success.

**Participate** (the detail page's Participate surface + per-fact controls):
- **Propose** (Proposal phase): pick an option + escrow a **KASS** bond.
- **Submit fact** (FactProposal phase): a content hash (hash pasted text, or paste a 32-byte hex
  hash) + an off-chain URI (≤200 bytes) + a KASS stake.
- **Vote** (FactVoting phase): Approve or flag Duplicate on each fact + a KASS stake.

**Crank / finalize** (permissionless, one per pre-Resolved phase): finalize proposals → advance →
finalize facts → finalize AI claims → finalize oracle, advancing the oracle toward Resolved.
Near-cap proposer sets (past ~24) show a v0/ALT note instead of a legacy-tx button. The oracle
**nonce** (needed by finalize-facts/oracle and not stored on-chain) is persisted at create time
(`src/lib/nonceStore.ts`, per-browser localStorage) and recalled before the bounded PDA-scan
fallback.

**Challenge + AI claim:**
- **Submit AI claim** (AiClaim phase): the three 32-byte model/params/io hashes + the option (hex
  fields, or paste the runner's JSON output); the proposer PDA is derived from the connected wallet.
- **Challenge** (Challenge phase): a deliberately **thin** open + settle-crank + status surface —
  a browser can't compose a MetaDAO v0.4 conditional-vault market, so the externally-composed
  account set is pasted as runner-emitted JSON (parsed safely, never `eval`'d). Settle is withheld
  until the market's TWAP window closes.

**Claim / close / sweep** (Resolved/InvalidDeadend phase): on each card, a **Claim** control
(shown only to the owning wallet — `authority == connected`) pays a participant's KASS reward/refund
and closes the account; permissionless **Close** (AI claim / settled market) and a grace-gated,
governance-checked **Sweep** (residual → the DAO treasury; rent → the creator) finish cleanup.

Every staking action **requires KASS** — the bond/stake is escrowed to the oracle's stake vault (amounts
are raw base units, matching the read view; a missing KASS ATA is created idempotently on the
first action). Forms wrap the pure WF1 action layer (`src/data/actions.ts` `build*Ixs`) and send
via wallet-adapter's `sendTransaction`; `src/hooks/useWriteAction.ts` + `src/data/writeAction.ts`
drive the status **idle → building → signing (wallet prompt) → confirming → success/error**.
On success the confirmation line shows the signature (+ a Solana-Explorer link off localnet) and
the oracle detail **refetches**. Errors are human-readable: validation shows inline before submit,
a user cancel reads "Transaction rejected in wallet.", and a failed send shows the message + the
program logs. **Disconnected** → the read view is unchanged and each form shows "Connect a wallet
to participate."; **wrong phase** → a muted "Participation is closed — this oracle is in the
{phase} phase." Ember is used only for the error accent; chestnut for the submit button.

### Offline preview (mock mode)

There is no standing deployment, so the browse views ship a mock affordance for offline design
review that does **not** touch the real data path: set **`VITE_MOCK=1`** at build/dev time, or
append **`?mock`** to any browse URL at runtime (e.g. `/oracles?mock`). Fixtures live in
`src/data/mockOracles.ts` (decoded-shaped oracles covering every phase + a fully-populated
detail with facts/proposers/AI-claims/market; a bogus `:pubkey?mock` exercises the not-found
state). Without the flag, the pages always go through `fetchOracles`/`fetchOracleDetail` over
the live connection.

Mock mode also drives the **write-form states** for design review (a real browser wallet can't
be scripted): under `?mock`, append `&wallet=connected` for a scripted connected wallet, and
`&tx=success|error|reject|failconfirm|slow` to script the send/confirm outcome (see
`src/lib/mockWrite.ts` — swapped in for the real `WalletProvider` only under mock mode).

## The Delphi design system

- **Tokens** live in `src/index.css` as a Tailwind v4 CSS-first `@theme` block: the color
  palette (parchment canvas, chestnut the only button fill, ember/saffron accents…), the type
  scale, the radii vocabulary `{4,8,12,16,70}px`, the three font families, and the peach
  `--shadow-bloom`.
- **Primitives** in `src/components/ui/` (barrel `index.ts`): `Button`
  (PrimaryChestnut / GhostOutline / NavPill), `Card`, `EyebrowTag`, `SectionHeader`,
  `AvatarBubble` (+ `VerifiedDot`), `TriggerPreviewCard`.
- **Oracle-browse components** in `src/components/oracles/`: `Chip` (on-brand status tones —
  ember reserved for the single "Challenged" moment), `PhaseChip` (`Phase` → label + tone),
  and `Truncated` (copy-on-click pubkeys/hashes). Presentation helpers (phase mapping, relative
  deadline, digit grouping, hash previews) live in `src/lib/oracleView.ts`.
- **Landing sections** in `src/components/landing/`: `NavBar`, `Hero` (the signature
  constellation of scattered question cards), `HowItWorks`, `WhyKassandra`, `TrustPanel`
  (the centered portrait panel — the one place a gradient is allowed), `SiteFooter`.

Design rules (from `docs/design/delphi-style-guide.md`): parchment everywhere (pure-card only
for lifted cards); chestnut is the ONLY button fill; flat surfaces + hairline pebble borders
(no heavy drop shadows — only the peach button bloom + the portrait-panel gradient); serif only
for display ≥20px, Inter for all body; ≤2 text colors per block; ember/saffron as 1–2
punctuation moments per viewport.

## What's built vs next

The dApp is layered across four slices: **slice 1** the Delphi design system + landing; **slice 2**
wallet connect (`AppProviders` → wallet-adapter) + the read layer (`src/data/oracles.ts`, the
`/oracles` browse + `/oracles/:pubkey` detail); **slice 3** the participation write flows
(propose / submit-fact / vote-fact); **slice 4** the **complete write surface** — create-oracle,
the finalize/crank progression, challenge (open/settle) + submit-AI-claim, and claim/close/sweep
payouts. Every write wraps the pure action layer (`src/data/actions/*.ts` `build*Ixs`
→ `sendAndConfirm` → `useWriteAction`) and is proven by a keypair-driven gated surfpool E2E
(`KASSANDRA_E2E=1`), including a **forked-mainnet** challenge settle over the real MetaDAO v0.4 AMM.
Read-only browsing still works fully disconnected.

**Next / deferred:** a standing devnet deployment (the app points at a configurable cluster; the
E2Es use surfpool); a KASS-balance affordance on the staking forms (the tx error surfaces cleanly
without it); route-level code-splitting (the main chunk is over the 500 kB warning); and a richer
challenge-market trading UI (the current open/settle surface is intentionally thin). The app only
ever consumes the built `@kassandra/sdk`; programs/runner/SDK-src are untouched.

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
  copy-on-click truncated pubkeys/hashes. Read-only.

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

### Offline preview (mock mode)

There is no standing deployment, so the browse views ship a mock affordance for offline design
review that does **not** touch the real data path: set **`VITE_MOCK=1`** at build/dev time, or
append **`?mock`** to any browse URL at runtime (e.g. `/oracles?mock`). Fixtures live in
`src/data/mockOracles.ts` (decoded-shaped oracles covering every phase + a fully-populated
detail with facts/proposers/AI-claims/market; a bogus `:pubkey?mock` exercises the not-found
state). Without the flag, the pages always go through `fetchOracles`/`fetchOracleDetail` over
the live connection.

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

## Slice 1 (done) vs the next milestone

**Slice 1 (this UI):** the design-system foundation + the landing page — static, composed
from the primitives. Wallet-adapter and `@kassandra/sdk` deps are present and linked
(`workspace:*`), but **not wired**: the nav "Connect wallet" pill is a placeholder.

**Next milestone:** the functional dApp — wallet connect + real RPC reads/writes via
`@kassandra/sdk` (browse oracles/disputes, propose/challenge/vote/settle).

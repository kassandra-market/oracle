# Kassandra UI

The web UI for **Kassandra** — a decentralized, AI-assisted **optimistic oracle** on Solana.
Vite + React 19 + TypeScript + Tailwind v4 SPA, styled in the **Delphi** visual language
("warm parchment editorial with ember sparks").

## Run / build

> **Build the SDK first.** The app links `@kassandra-market/oracles` via the pnpm workspace and
> resolves its types from `sdks/oracles/ts/dist/` (which is gitignored). On a fresh clone, run
> `pnpm --filter @kassandra-market/oracles build` (or `pnpm -r build`) **before** the app's
> typecheck/build, or the SDK import won't resolve. (Slice 1 only link-proofs the
> import; the functional-dApp milestone will depend on the SDK types for real, so CI
> must build the SDK first.)

```bash
pnpm --filter @kassandra-market/oracles build   # build the SDK first (types → sdks/oracles/ts/dist)
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

The build is **route-code-split + vendor-chunked**: `src/App.tsx` wraps each page in
`React.lazy` + `<Suspense>` (nav/shell outside the boundary, so it stays instant), and
`vite.config.ts` `build.rollupOptions.output.manualChunks` groups the heavy libs into
separate cacheable chunks (`solana` = wallet-adapter + web3.js + deps, `sdk` =
`@kassandra-market/oracles`, `react-vendor` = React + router). The entry chunk is ~21 kB (from a prior
752 kB single chunk); each page ships as its own lazily-loaded chunk. No chunk exceeds the
500 kB warning.

## Routes

- `/` — the Kassandra landing page (`src/pages/Landing.tsx`).
- `/oracles` — the oracle browser (`src/pages/Oracles.tsx`): a **dashboard stats strip** over a
  responsive grid of Delphi cards, one per decoded on-chain oracle (phase chip, relative
  deadline, proposer/fact/option counts, resolved option). The stats strip (`DashboardStats`,
  computed by the pure `src/lib/oracleStats.ts` `deriveStats` — client-side over the already
  fetched list, no extra RPC) shows the by-phase-group counts, the total bonds/stake at risk
  across active oracles (raw base units, unscaled), and the resolved count. A toolbar composes
  a **text search** + accessible **phase-filter chips** (All / Proposal / In dispute / AI claim /
  Challenged / Resolved — `aria-pressed` toggle buttons) + a **sort** (deadline / bonds at risk):
  search → filter → sort all apply to the grid client-side. Read-only.
- `/oracles/:pubkey` — the oracle detail view (`src/pages/OracleDetail.tsx`): an editorial
  layout of one oracle + its facts, proposers, AI claims, and challenge market, with
  copy-on-click truncated pubkeys/hashes. Under the title it carries an at-a-glance **verdict
  banner** (Resolved · Option N / dead-ended / the in-flight phase + what's next), a flat
  **phase timeline** (`components/oracles/PhaseTimeline.tsx` over the pure
  `lib/phaseTimeline.ts` model — Proposal → … → Resolved with the current phase highlighted +
  its deadline and a distinct dead-end terminal), and an **economic picture**
  (`components/oracles/EconomicPanel.tsx` — flat div/token bars of bond pool vs dispute bonds
  vs total stake, plus the proposer bond-by-option split with the leading option accented; no
  chart lib). Read-only browsing works fully disconnected; the wallet-signed **write forms**
  (below) are additive on top.

  The **challenge-market** section now shows a **live visualization** on top of the existing
  market card (`components/oracles/ChallengeMarketPanel.tsx` over the CU1 read layer): the pass
  vs fail pools' **instantaneous (spot) price** + slot-weighted **TWAP** (decimals-aware; a
  pre-start-delay pool reads "TWAP forming…"), a flat Delphi **TWAP → disqualify-margin bar**
  (how close the FAIL TWAP is to exceeding PASS by the market margin `num/den`; the single ember
  accent lights when it nears/clears the margin — settling would disqualify the proposer), a
  **countdown** to the TWAP window close, the **challenger's escrowed USDC**, and each pool's raw
  **reserves**. The v0.4 MetaDAO `Amm` accounts are decoded in `src/data/ammV04.ts`
  (`decodeAmmV04` + the `instantaneousPrice` / `twapPrice` / `marginProgress` helpers, mirroring
  the on-chain `get_twap()` / `settle_challenge` math + the `cpi/metadao.rs` offsets), fetched by
  the `src/hooks/useMarketAmms.ts` hook (guarded, light polling, no websocket). Chart-lib-free
  (divs + tokens); read-only; degrades gracefully (no market, pre-start-delay, or settled).

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

Each of these three staking forms shows the connected wallet's **KASS balance** (`Your KASS: …`,
raw base units) below the bond/stake input and **gates the submit** when the entered amount exceeds
that balance (or the wallet holds no KASS) — an inline message instead of a doomed on-chain tx.
The check is additive to the existing client-side validation and never hard-blocks on a still-loading
balance (the tx remains the ultimate guard); the balance refetches after a successful bond/stake
(`src/hooks/useKassBalance.ts` over `src/data/balance.ts`).

**Crank / finalize** (permissionless, one per pre-Resolved phase): finalize proposals → advance →
finalize facts → finalize AI claims → finalize oracle, advancing the oracle toward Resolved.
Near-cap proposer sets (past ~24) show a v0/ALT note instead of a legacy-tx button. The oracle
**nonce** (needed by finalize-facts/oracle and not stored on-chain) is persisted at create time
(`src/lib/nonceStore.ts`, per-browser localStorage) and recalled before the bounded PDA-scan
fallback.

**Challenge + AI claim:**
- **Submit AI claim** (AiClaim phase): the three 32-byte model/params/io hashes + the option (hex
  fields, or paste the runner's JSON output); the proposer PDA is derived from the connected wallet.
- **Challenge** (Challenge phase): the challenge-market surface is now a full **live viz +
  trade/crank/settle + CLIENT-SIDE compose→open**:
  - **Open a challenge — no runner JSON.** A real form
    (`components/oracles/actions/ChallengeComposeForm.tsx`) composes the entire MetaDAO v0.4 market
    from the browser: the binary question → KASS + USDC conditional vaults → the challenger's +
    oracle-holder ATAs → `split_tokens` (into pass/fail conditional tokens) → 2× `create_amm` +
    `add_liquidity` (seed) → `open_challenge`. The choreography far exceeds one transaction, so it
    runs as an **ordered, staged sequence** of wallet-signed txs with **per-step progress**
    (Question ✓ → KASS vault ✓ → USDC vault ✓ → Fund + split ✓ → Pass pool ✓ → Fail pool ✓ → Open
    ✓) and **retry-from-the-failed-step** on a mid-sequence failure (the idempotent ATA-creates +
    deterministic PDAs make a resume safe). The seed/TWAP math mirrors the proven recipe
    (`twap_initial_observation = quote·1e12/base`, max-change `(2^64−1)·1e12`, start-delay 0).
  - **Trade / crank** (CU2): swap the pass/fail pools + crank their TWAP.
  - **Settle — ONE CLICK, no JSON paste.** Once the market's TWAP window closes, the permissionless
    settle is a single button: the full 15-account settle set is **derived client-side from the
    decoded Market + Oracle** (`data/actions/challengeSettle.ts::buildSettleFromMarketIxs`) — 8
    accounts are Market fields (aiClaim / proposer / question / pass+fail AMM / kassVault / oracle
    pass+fail KASS holders) and 7 are derived (pass/fail conditional-KASS mints, the KASS vault
    underlying ATA, the conditional-vault event authority, and the proposer-USDC / challenger-USDC /
    challenger-KASS payout ATAs). The challenger-USDC **destination** is the challenger's own USDC
    ATA (`ATA(market.challenger, usdc_mint)`), distinct from the Market's SDK-derived USDC escrow.
    The three payout ATAs are idempotently created before settle so an absent destination never
    fails the crank. After SD1 the **challenge UI has NO JSON paste anywhere**.

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
(`KASSANDRA_E2E=1`), including a **forked-mainnet** challenge settle over the real MetaDAO v0.4 AMM
driven through the **one-click derive-from-Market settle** (`buildSettleFromMarketIxs`).
Read-only browsing still works fully disconnected.

**Next / deferred:** a standing devnet deployment (the app points at a configurable cluster; the
E2Es use surfpool). The challenge-market surface is now complete — a **live viz** (CU1) +
**trade/crank/settle** (CU2, settle is **one-click**: the account set is derived from the Market,
no JSON paste) + **client-side compose→open** (CU3, no runner JSON) — each proven by
a gated forked-mainnet E2E over the real MetaDAO v0.4 conditional-vault + AMM. The app only ever
consumes the built `@kassandra-market/oracles`; programs/runner/SDK-src are untouched.

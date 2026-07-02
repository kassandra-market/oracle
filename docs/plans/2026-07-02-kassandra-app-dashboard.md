# Kassandra dApp — Dashboard + Richer Oracle Detail (Milestone A) — Design + Plan

> **For Claude:** REQUIRED SUB-SKILL: subagent-driven-development (per-task implement + review).

**Goal:** Enrich the two busiest read surfaces — the oracle **browse/dashboard** and the oracle **detail** — with at-a-glance stats, filters, a lifecycle **phase timeline**, and an **economic picture**, all in the Delphi language. Pure presentation over the already-decoded data (reuses `fetchOracles`/`fetchOracleDetail` + the SDK decoders). NO new on-chain surface, NO core (programs/runner/sdk) change, NO new dep, NO write-path change. (Milestone B — the deep challenge-market trading UI — is a separate follow-on slice.)

## Context / what exists
- **List** — `app/src/pages/Oracles.tsx`: `useOracles()` → a responsive grid of `OracleCard`s (each: `PhaseChip`, deadline, counts) + a client-side `search`. `SectionHeader` intro.
- **Detail** — `app/src/pages/OracleDetail.tsx`: stat tiles (options/counts/bond-pool/params) + Facts/Proposers/AiClaims/Market sections + the write controls (RF1-4). `PhaseChip`, `Truncated`, `groupDigits`, `phaseView` (`app/src/lib/oracleView.ts`).
- **Decoded data:** `Oracle` (phase/deadline/phaseEndsAt/optionsCount/proposerCount/survivingCount/factCount/bondPool/disputeBondTotal/totalOracleStake/settledCount/resolvedOption/thresholds/…), the `Phase` enum (Proposal→FactProposal→FactVoting→AiClaim→FinalRecompute→Challenge→Resolved/InvalidDeadend — read the exact values/order from `sdk/src/constants.ts`).
- **Delphi rules** (`docs/design/delphi-style-guide.md`): parchment; chestnut the only button fill; flat + hairline pebble borders (no heavy shadow); serif ≥20px; radii `{4,8,12,16,70}`; ≤2 text colors/block; ember/saffron only 1–2 punctuation moments per viewport. A simple bar/proportion viz built from divs + tokens (NO chart lib — no new dep) is on-brand; keep it flat + quiet.

## Tasks (RU1 ∥ RU2 — independent pages, parallel-safe)

### RU1 — Dashboard stats + filters on the browse page (`/oracles`)
- **A stats header** above the oracle grid (a `SectionHeader`-adjacent stats strip): compute from the already-fetched `fetchOracles` list (a pure helper `app/src/lib/oracleStats.ts` `deriveStats(summaries)`):
  - **By-phase counts** (Proposal / dispute [FactProposal+FactVoting] / AiClaim / Challenge / Resolved / InvalidDeadend — group sensibly), rendered as small labelled tiles/chips.
  - **Total bonds/stakes at risk** — sum the `bondPool` (+ `disputeBondTotal`/`totalOracleStake` as apt) across active (non-terminal) oracles, shown as a headline figure (raw base units + the "unscaled" convention; `groupDigits`).
  - **Recent resolutions** — a count (or a short list) of Resolved oracles (+ their resolved option), e.g. "N resolved".
  - Keep it compact + quiet Delphi (bronze/sepia; ember only for maybe ONE punchy figure if any — don't over-spark). Loading → skeleton; empty → a graceful "No oracles on {cluster} yet."
- **Phase filters + sort:** add filter chips (All / Proposal / In dispute / AiClaim / Challenge / Resolved) that filter the grid (client-side, on the already-fetched list) + a sort control (by deadline, by bonds-at-risk) — on TOP of the existing `search`. Accessible (the filter chips are real buttons/toggles with aria-pressed; keyboard-reachable). A "Create oracle" CTA already exists — keep it.
- **Offline unit tests** (`app/test/oracleStats.unit.test.ts`): `deriveStats` over a fixture list → correct by-phase counts, bonds-at-risk sum, resolved count; the filter/sort predicates. Pure, offline.
- **Verify + render:** typecheck + test + build (verify-css OK) + lint green; headless-render `/oracles?mock` (the mock fixtures cover multiple phases) — the stats header shows correct counts/figures, the filter chips filter the grid, sort works, 0 console errors, one h1, Delphi-faithful (quiet stats, ember ≤1–2). Read-only intact.
- Update README + append an RU1 delta. Commit `feat(app): dashboard stats + phase filters on the oracle browser`.

### RU2 — Richer oracle detail (phase timeline + economic viz + verdict)
- **Phase timeline** (`app/src/components/oracles/PhaseTimeline.tsx`): a horizontal (stacks on mobile) lifecycle strip — Proposal → FactProposal → FactVoting → AiClaim → (Challenge / FinalRecompute) → Resolved (or a distinct InvalidDeadend end). The CURRENT phase highlighted (chestnut/ember accent — ONE punctuation moment), past phases muted-done, future phases faint; show the current phase's `phaseEndsAt`/deadline (relative, reuse `relativeDeadline`). Terminal (Resolved/InvalidDeadend) shows the end state. Flat + hairline; a pure `phaseTimelineModel(oracle)` helper (in `oracleView.ts` or a new `phaseTimeline.ts`) computes the step list + which is current/done/future (unit-testable).
- **Economic picture** (`app/src/components/oracles/EconomicPanel.tsx` or fold into the detail): a small flat proportion/bar viz (divs + tokens, NO chart lib) of the economics — e.g. the bond pool vs dispute bonds vs total stake, or the proposer options' relative stake/bond split (from the decoded proposers — how the vote/bond is distributed across options). Quiet Delphi (bronze/pebble bars, maybe ONE ember accent for the leading/winning option). Raw base units (`groupDigits`) + labels; degrade gracefully when counts are 0.
- **At-a-glance verdict** (a header banner on the detail): for **Resolved** — the resolved option prominently ("Resolved: Option N", a calm confirmed tone); for **InvalidDeadend** — a clear "No resolution — dead-ended" (muted); for in-flight — the current phase + what happens next (one line). Additive to the existing header (keep the one `<h1>` = the oracle title; the verdict is an h2/banner).
- **Offline unit tests** (`app/test/phaseTimeline.unit.test.ts`): `phaseTimelineModel` for several phases (Proposal / FactVoting / Resolved / InvalidDeadend) → the correct current/done/future split + the terminal end state; the verdict-label logic. Pure, offline.
- **Verify + render:** typecheck + test + build (verify-css OK) + lint green; headless-render `/oracles/:pubkey?mock` (the fully-populated challenged mock detail + a resolved one if the fixtures allow) — the timeline shows the current phase highlighted + deadline, the economic viz renders, the verdict banner reads correctly, 0 console errors, one h1, Delphi-faithful (ONE ember accent max), the existing detail sections + write controls unaffected. Read-only intact disconnected.
- Update README + append an RU2 delta. Commit `feat(app): oracle-detail phase timeline + economic viz + verdict banner`.

## Out of scope / deferred
- The challenge-market trading UI (Milestone B — client-side market composition + live pass/fail prices + trading; separate slice).
- Any write-path/behavior change, new on-chain read, core change, or new dep (NO chart lib — build viz from divs + tokens).
- Real-time subscriptions (poll/refetch is fine); a standing devnet deployment.

## Execution note
Pure Delphi presentation over the existing decoded data — reuse `fetchOracles`/`fetchOracleDetail`, the decoders, `phaseView`/`relativeDeadline`/`groupDigits`, the slice-1 primitives. RU1 (Oracles.tsx + oracleStats.ts) ∥ RU2 (OracleDetail.tsx + PhaseTimeline/EconomicPanel + phaseTimeline.ts) are INDEPENDENT pages → parallel. NO chart lib (build viz from divs + tokens — no new dep). Delphi do/don'ts (ember only 1–2 punctuation moments per viewport — don't spark every tile/bar). Keep the default suite offline + green; `verify-css` green; read-only browse + the write controls (RF1-4) + slices 1-3 intact. Don't touch programs/runner/sdk-src or the action/write layer. Append RU1/RU2 deltas.

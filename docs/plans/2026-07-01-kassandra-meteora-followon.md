# Meteora Follow-Ons (F1 remove/claim live + F2 futarchy→Meteora treasury) — Design + Plan

> **For Claude:** REQUIRED SUB-SKILL: subagent-driven-development (per-task implement + review).

**Goal:** Close the two Meteora follow-ons from the spot-path milestone: **F1** — drive the last two cp-amm builders (`removeLiquidity`, `claimPositionFee`) live through the real deployed program (they're unit-tested but not driven live). **F2** — wire REAL Meteora treasury liquidity into a futarchy E2E: pin the futarchy `collect_meteora_damm_fees` CPI wire format (currently UNDETERMINED), build the SDK builder, and drive the futarchy→Meteora fee-collection CPI on a mainnet fork. NO on-chain program change (SDK/test only).

## Context / honest scope
- Meteora is the DAO's SPOT-LIQUIDITY / TREASURY side, PERIPHERAL to the oracle protocol (the program doesn't CPI Meteora; `kass_price` reads the futarchy EMBEDDED oracle; the governance verdict is the embedded AMM — G3 already proves that loop real). M1/M2 already proved the 6 cp-amm builders against the deployed program.
- **F1 is clean** (extends the existing `meteora-spot-e2e.test.ts`; the M2 swap ALREADY accrued real LP fees). **F2 carries real uncertainty**: the futarchy `collect_meteora_damm_fees` discriminator/accounts/args are NOT pinned anywhere (only named as a string in `cpi/metadao_v06.rs`); F2a must pin it AUTHORITATIVELY or STOP-report.

## Source of truth
- Deployed futarchy `FUTARELBfJfQ8RDGhg1wdhddq1odMAJUePHFuBYfUxKq` (v0.6.1 — `sdk/src/futarchy/constants.ts:43` notes v0.6.1 deployed; `metadao_v06.rs:82`). Meteora cp-amm `cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG`.
- The M1 Meteora SDK module `sdk/src/meteora/{constants,pda,instructions,accounts,index}.ts` (the 6 builders + decoders, byte-sourced from `MeteoraAg/damm-v2@bdd8a1e`). `removeLiquidity`/`claimPositionFee` builders EXIST (unit-tested) — F1 drives them live.
- The M2 E2E `sdk/test/surfpool/meteora-spot-e2e.test.ts` (init→add→swap→createPosition through the real cp-amm; the swap accrues LP fees — the M2 delta noted vault B > tracked reserve by the fee). F1 extends this.
- `sdk/src/futarchy/{constants,instructions}.ts` (the futarchy builders — has `provide_liquidity` for the EMBEDDED AMM, NOT Meteora; NO `collect_meteora_damm_fees`). `sdk/test/surfpool/futarchy-governance-e2e.test.ts` (G3 — the verdict loop; fabricates the Dao spot state) + `harness.ts`.
- MetaDAO futarchy is OPEN SOURCE (`github.com/metaDAOproject/futarchy` — the v0.6 tree; find the tag/commit matching the v0.6.1 DEPLOYED program). Anchor programs also publish an ON-CHAIN IDL (fetchable) — either source pins `collect_meteora_damm_fees`.

## Tasks

### F1 — removeLiquidity + claimPositionFee live coverage (extend the Meteora fork E2E)
- Extend `sdk/test/surfpool/meteora-spot-e2e.test.ts` (the gated `KASSANDRA_E2E=1` suite): after the existing init→add→swap arm (the swap accrued LP fees to the position holding liquidity), add:
  - **`claimPositionFee`** on the first position → decode the owner's token accounts + the Position before/after; ASSERT the accrued fee (fee_a and/or fee_b) is transferred out (owner balance rose / the pool's tracked fee cleared). The swap direction (A→B) accrues the fee in the input token — assert the correct side. If the accrued fee is 0 (fee tier/amount too small), do a larger or repeated swap first so a NONZERO fee is claimable — the assertion must prove a real, nonzero fee claim (not a no-op).
  - **`removeLiquidity`** from the first position (e.g. remove half or all `unlocked_liquidity`) → decode Pool + Position before/after; ASSERT the position's `unlocked_liquidity` dropped by the delta, the pool `liquidity` dropped, both token reserves fell, and the owner's token accounts received the withdrawn amounts (respecting the threshold args). 
- Drive both through the REAL cp-amm over RPC (`skipPreflight:false`, confirm-throws). This completes live coverage of ALL 6 builders against the deployed binary.
- STOP-report if a genuine blocker (e.g. claimable fee is unavoidably 0 on this config, or remove needs an un-drivable precondition) — don't fake a passing no-op. Update the E2E header + `sdk/test/surfpool/README.md` + `sdk/src/futarchy/NOTES.md` (remove/claim now DRIVEN LIVE, not just unit-tested).
- `cd sdk && pnpm typecheck` + default `pnpm test` (offline green) + gated `KASSANDRA_E2E=1 pnpm exec vitest run test/surfpool/meteora-spot-e2e.test.ts` (RUN it). Commit `test(e2e): drive removeLiquidity + claimPositionFee live on forked cp-amm`.

### F2a — Pin the futarchy `collect_meteora_damm_fees` CPI + SDK builder (STOP-report if undeterminable)
- **Authoritatively determine** the futarchy `collect_meteora_damm_fees` wire format for the DEPLOYED v0.6.1 program: the discriminator (`sha256("global:collect_meteora_damm_fees")[..8]` — trivial; but CONFIRM the exact instruction NAME from the source/IDL) + the Borsh ARG layout + the ACCOUNT list (order + roles + which are PDAs — the Dao, the Meteora pool/position/vaults, the DAO treasury token accounts, the cp-amm program, token programs, event authority) + the PDA seeds. SOURCES (use ≥2 to cross-confirm): (a) the MetaDAO futarchy open-source repo (`github.com/metaDAOproject/futarchy` — pin the tag/commit matching v0.6.1 deployed; read `programs/futarchy/src/instructions/…collect_meteora_damm_fees…` + `lib.rs`), and/or (b) the ON-CHAIN Anchor IDL for `FUTAREL…` (fetch from mainnet — the Anchor IDL account, or via a known IDL mirror). 
- **If the wire format CANNOT be authoritatively pinned** (repo tag for v0.6.1 not findable AND no fetchable IDL, or the account list is genuinely ambiguous), **STOP and report** exactly what was tried + why — do NOT guess a CPI that moves real DAO funds. (This is the flagged uncertainty.)
- If pinned: add `collectMeteoraDammFees` to `sdk/src/futarchy/{constants,instructions}.ts` (disc + args + account metas in the exact order, cite the source file:line/IDL) + an offline byte-layout unit test (data == disc ++ borsh(args) + the metas/roles/PDA derivations). Document the pinned source (repo commit or IDL) in the module + NOTES.
- `cd sdk && pnpm typecheck && pnpm test` (offline green + the new builder test). Commit `feat(sdk): futarchy collect_meteora_damm_fees builder (wire format pinned from <source>)`.

### F2b — Futarchy→Meteora treasury E2E (drive the fee-collection CPI live)
- (DEPENDS on F2a — only if F2a pinned the wire format.) New gated `sdk/test/surfpool/futarchy-meteora-treasury-e2e.test.ts` (or extend the governance E2E): on a mainnet fork with the real futarchy + cp-amm programs, set up a scenario where a futarchy `Dao` holds a Meteora position (the DAO's treasury spot liquidity — created via the M1 builders with the Dao PDA as the position owner/authority; clone the real Config), generate trading fees (swaps against the pool), then drive the futarchy `collect_meteora_damm_fees` CPI (the F2a builder) so the DAO collects the fees into its treasury.
- ASSERT the fee-collection worked over RPC: the DAO treasury token account received the accrued Meteora fees (decode before/after), driven through the REAL futarchy program (`skipPreflight:false`, confirm-throws). This proves the futarchy→Meteora CPI end-to-end.
- **This is the involved/uncertain part** — the Dao-owns-a-Meteora-position setup + the exact authority/signer the futarchy CPI expects may need cloning real futarchy Dao state or fabricating it (mirror how G3 set up the Dao). If a genuine blocker surfaces (the Dao-position ownership can't be constructed on the fork, or the CPI needs an un-clonable dependency), STOP-and-report with the exact error + a documented partial (e.g. the F2a builder is offset/byte-verified against the IDL + a real futarchy account decode, even if the full live CPI can't be driven) — do NOT fake it.
- Update `sdk/test/surfpool/README.md` + `sdk/src/futarchy/NOTES.md` + `sdk/README.md`: futarchy→Meteora treasury fee-collection now covered (or the documented partial). Append the F1/F2a/F2b delta + a covered-vs-deferred note to this plan.
- `cd sdk && pnpm typecheck` + default `pnpm test` (offline green) + the gated E2E (RUN it). Commit `test(e2e): futarchy→Meteora DAO treasury fee-collection on forked mainnet`.

## Out of scope / deferred
- On-chain program change (none).
- Making the GOVERNANCE verdict use Meteora (it correctly uses the embedded AMM; Meteora is treasury only).
- Meteora reward-emission / dynamic-fee mechanics beyond the spot + fee-collection path.

## Execution note
SDK/test only; default `pnpm test` stays offline + green; the E2Es are gated. F1 + F2a are INDEPENDENT (F1 = the meteora-spot E2E; F2a = the futarchy builder) → can run in parallel. F2b DEPENDS on F2a (needs the pinned builder) — sequential. F2a is the risk: pin the wire format AUTHORITATIVELY (≥2 sources) or STOP-report — never guess a fund-moving CPI. Append an F1/F2a/F2b delta log here.

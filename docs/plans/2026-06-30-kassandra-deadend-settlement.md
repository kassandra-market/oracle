# Dead-end Economic Settlement — Design + Plan

> **For Claude:** REQUIRED SUB-SKILL: subagent-driven-development (per-task implement + review).

**Goal:** Fix the dead-end settlement gap so a terminal `InvalidDeadend` oracle (and a governance-resolved-from-dead-end oracle) FULLY DRAINS its `stake_vault` with no stranded funds: **non-slashed principal is returned to each staker; all slashed amounts (`bond_pool`) are BURNED; the `reward_emission` is burned; the creator fee stays burned.** Today the slashed `bond_pool` (and, on the no-facts path, the emission) is never moved out of the vault → stranded.

**The economic rule (decisions locked with the user + documented intent):**
- A dead-end is a non-outcome: **no rewards, no distribution**. Stakers get their **non-slashed principal** back; the governance-chosen `resolved_option` (if `resolve_deadend` ran) is recorded for downstream consumers but does NOT drive reward/slash (documented: design §7/§9, settlement-economics, futarchy F4).
- **Slashed amounts are BURNED on a dead-end** (USER DECISION): misbehavior slashes have no recipient (no winner), so they are burned like the creator fee — including the **no-facts case where every disputing proposer is slashed → all those bonds burned** (deterrent against propose-conflict-then-abandon). `reward_emission` is burned too.
- Net: vault drains to dust; `Σ returned principal + dust + (bond_pool burned) + (emission burned) == Σ stakes + emission`.

## The gap (from investigation, file:line)
- `resolve_deadend.rs:73-79` only sets `resolved_option` + flips `Phase::InvalidDeadend→Resolved`; no token movement. NO marker distinguishes it from an organic Resolved — but it always runs AFTER the oracle is already `InvalidDeadend`, and a dead-end always has `reward_pool == 0`.
- **Case A — no-facts dead-end** (`finalize_facts::finalize_no_facts`, ~`finalize_facts.rs:129-185`): every proposer `disqualified, slashed_amount=bond`, all bonds → `bond_pool`; terminates directly to `InvalidDeadend` and **never burns `reward_emission`**. On claim, every disqualified proposer gets base 0 → **Σ bonds + emission stranded (entire vault)**.
- **Case B — tie/no-survivor dead-end** (`finalize_oracle.rs:256-279`): survivors get `bond − slashed_amount`, agreed-fact stakers get stake; emission already burned here; but **`bond_pool` (disqualified bonds + rejected-fact stakes + approve-voter slashes) is stranded** because `reward_pool == 0` (no reward distribution to carry it out). This strands even on a plain (non-governance) InvalidDeadend when the dispute proceeded then tied.
- Claims (`claims.rs`): `claim_proposer` base = `is_disqualified()?0:bond−slashed_amount` (SAME on both phases — `resolved` only gates the reward); reward terms all scale from `reward_pool` via `reward::reward_buckets`, so **`reward_pool==0 ⇒ every reward term is 0`**.

## Preferred fix (verify first — likely NO marker / NO claims / NO layout / NO SDK change)
KEY INSIGHT to verify: because a governance-resolved dead-end has `reward_pool == 0`, the EXISTING claim path already pays **zero reward + only non-slashed principal** on BOTH `InvalidDeadend` and the governance-resolved `Resolved` state. So the fix is simply to **burn the misrouted funds at the InvalidDeadend finalize sites** so the vault holds only returnable principal:
1. **`finalize_oracle` InvalidDeadend branch (Case B):** in addition to the existing `reward_emission` burn, **burn `bond_pool`** from `stake_vault` (SPL Burn, oracle-PDA-signed) so the slashed amounts leave the vault.
2. **`finalize_facts::finalize_no_facts` (Case A):** **burn `bond_pool` (= Σ bonds) AND `reward_emission`** from `stake_vault` when terminating to `InvalidDeadend` (symmetric with finalize_oracle). This repairs the plain no-facts InvalidDeadend too (currently strands).
3. **Claims unchanged** — verify: on a dead-end, `claim_proposer` returns `bond−slashed_amount` for survivors / 0 for disqualified (their bond was in the now-burned `bond_pool`); fact/vote claims return non-slashed principal. Reward terms are 0 (reward_pool=0). The vault, after the finalize burns, holds exactly the returnable principal → claims drain it to dust.
4. **`resolve_deadend` unchanged** (no token movement — the burns happened at finalize; it just flips the phase + records the option). Update its + `require_terminal`'s docstrings (the "F4 pays stakes-back only, no special-casing" claim is the now-falsified assumption).

**Verification gate (DS1):** confirm by test that with the finalize burns in place, BOTH a plain `InvalidDeadend` AND a governance-resolved-from-dead-end oracle fully drain (survivors/honest stakers get non-slashed principal, disqualified/rejected get 0, vault → dust). If — and only if — a governance-resolved dead-end is found to pay something WRONG via the Resolved path (it shouldn't, since reward_pool=0), fall back to adding a minimal `Oracle.resolved_from_deadend` marker (append at offset 392, re-pin state_layout, update the SDK `decodeOracle`) and branch claims on it. PREFER the no-marker approach; only add the marker if verification proves claims diverge.

## Tasks

### DS1 — Burn the slashed bond_pool + emission at the InvalidDeadend finalize sites (program) + conservation
- Implement the two finalize burns (above): `finalize_oracle` InvalidDeadend branch burns `bond_pool`; `finalize_no_facts` burns `bond_pool` + `reward_emission`. Use the existing oracle-PDA-signed SPL Burn pattern (mirror the emission burn already in `finalize_oracle`). Account lists may need the `kass_mint` + `stake_vault` + token program on `finalize_facts` (the no-facts path) if not already present — add them (ABI change to finalize_facts if needed; update the SDK `finalizeFacts` builder + any harness `*_ix`).
- **Verify the no-marker insight** with tests; if it holds, claims/resolve_deadend/Oracle-layout/SDK-decoder are UNCHANGED.
- Update the **conservation invariant** + docstrings (`claims.rs` require_terminal; resolve_deadend.rs). 
- Tests (`programs/kassandra/tests/`): 
  - **No-facts dead-end:** create→propose conflicting→finalize_no_facts→assert `bond_pool` + emission BURNED (supply down, vault drained of bonds), every (disqualified) proposer claims 0, vault → dust. (User decision: no-facts proposer bonds burned.)
  - **Tie dead-end with slashes:** a dispute that proceeds (facts, a rejected fact / disqualified proposer / slashed voter) then ties → InvalidDeadend → assert survivors/agreed-stakers get non-slashed principal, the `bond_pool` (slashed amounts) is burned, vault → dust.
  - **Governance-resolved dead-end:** the tie/no-facts dead-end then `resolve_deadend(option)` → Resolved → claims still pay non-slashed principal only (no reward), vault → dust; `resolved_option` recorded.
  - **Conservation fuzz arm:** extend `invariants.rs` (the settlement fuzz) to cover the slashed-then-deadend + governance-resolved cases: `Σ returned principal + dust == Σ stakes + emission − (bond_pool burned + emission burned)`. Fuzz disqualified/rejected/slashed combinations.
- `just build` + `cargo test -p kassandra-program` (all green incl. new) + clippy + fmt; if the finalizeFacts ABI changed, `cd sdk && pnpm typecheck && pnpm test` green. Commit `fix(settlement): burn slashed bond_pool + emission on dead-end (no stranding)`.

### DS2 — SDK/E2E touch (only if needed) + docs + covered-vs-deferred
- If DS1 changed the `finalizeFacts` account list or the Oracle layout, update the SDK builder/decoder + parity + add a litesvm/SDK assertion that a governance-resolved dead-end drains. If DS1 needed no SDK change, this is docs-only.
- Update `docs/plans/2026-06-29-kassandra-settlement-economics.md` (or the staker-settlement plan) covered-vs-deferred: dead-end economic settlement now DONE (the burn rule + the no-facts-burn decision); note the governance-resolved path drains. Append the final note to this plan. Commit `docs(settlement): dead-end settlement covered (burn slashed + emission)`.

## Out of scope / deferred
- Dust sweeping / closing the terminal Oracle + stake_vault accounts (the NEXT deferred milestone).
- Any change to the normal (non-dead-end) Resolved economics.

## Execution note
After each task: `just build` + `cargo test -p kassandra-program` green; default `pnpm test` stays green (88) if the SDK is touched. DS1 is the substantive program fix — VERIFY the no-marker insight (reward_pool==0 ⇒ existing claims already correct) before adding any marker; the conservation fuzz over slashed-then-deadend is the proof. Append a DS1/DS2 delta log here.

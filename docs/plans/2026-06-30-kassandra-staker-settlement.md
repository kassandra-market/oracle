# Kassandra Staker Settlement — Design + Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** The final economic layer — turn the counter-only ledger into real token movements: per-staker **claims** (bond/stake returns + rewards), **bond_pool reward distribution** to honest cohorts, **emissions** (minted at oracle creation from the supply reservoir), and **account closure** (rent reclaim). Closes the "stranded bonds" gap and completes the protocol economics. Final step of the roadmap: KASS futarchy ✅ → challenge-market rework ✅ → **staker settlement**.

**Architecture:** Extends the existing Pinocchio program. Per-staker **pull** model (each participant claims their own account, claim-and-close). Entitlements computed from the claimant's own account + a few oracle-level totals stamped at resolution — no global iteration at claim time.

**Tech Stack:** Rust, `pinocchio` 0.8, `bytemuck`, `litesvm`, `solana-sdk` (test-only), `spl-token`.

**Source of truth:** the agreed economics in `docs/plans/2026-06-29-kassandra-settlement-economics.md` (THIS plan implements it) + the **conservation hand-off contract** from `docs/plans/2026-06-29-kassandra-challenge-rework.md` (Out-of-scope section). Plus the dispute-core/happy-path/futarchy/challenge live-state deltas. FOLLOW THE LIVE STATE.

---

## Agreed economic model (from the settlement note — implement exactly)

### Trigger: per-staker PULL, claim-and-close
After an oracle is terminal (`Resolved`/`InvalidDeadend`), each participant calls a `claim_*` for their own account; the program computes entitlement from that account + oracle-level totals stamped at resolution, transfers KASS from `stake_vault` (program-signed by the oracle PDA), and **closes the account** (rent → claimant). No global iteration; no one-tx cap.

### CONSERVATION CONTRACT (from the challenge milestone — CRITICAL)
`total_oracle_stake` is an **IDEALIZED accumulator, NOT physical KASS** (a successful challenge sent `kass_fee` out; external donations can inflate balances). **Settlement MUST source payouts from the real `stake_vault`/`bond_pool` balances + the per-proposer `slashed_amount` ledger — NEVER from `total_oracle_stake` or live token-account balances.** The authoritative per-proposer figure is `slashed_amount == bond_pool contribution`.

### Reward pool (only on `Resolved`)
`reward_pool = bond_pool + reward_emission`, split into **cohort buckets** by config weights (`reward_proposer_weight` PW, `reward_fact_weight` FW; default PW>FW): `proposer_bucket = reward_pool·PW/(PW+FW)`, `fact_bucket = reward_pool·FW/(PW+FW)`. Each bucket pro-rata within its cohort via two totals stamped at resolution: `total_correct_proposer_stake`, `total_approved_fact_stake`. **Empty cohort's bucket rolls into the proposer cohort** (always ≥1 winner on Resolved).

### Per-actor matrix (on `Resolved`)
| Actor | Entitlement |
|---|---|
| Correct proposer (survived, `claim_option == resolved_option`) | `bond + bond·proposer_rate` |
| Wrong-but-survived proposer (`claim_option != resolved_option`, not disq) | `bond` (no reward) |
| Disqualified/slashed proposer | `bond − slashed_amount` |
| Approved-fact submitter (fact `agreed`) | `stake + stake·fact_rate` |
| Approve-voter on an `agreed` fact | `stake + stake·fact_rate` |
| Duplicate-voter / staker on a duplicate-dominant fact | `stake` (no reward, no slash) |
| Rejected-fact submitter | `0` (funded bond_pool) |
| Approve-voter on a **rejected** fact | `stake·(1 − fact_vote_slash_frac)` |

`proposer_rate = proposer_bucket / total_correct_proposer_stake`; `fact_rate = fact_bucket / total_approved_fact_stake` (u128; pro-rata by the staker's own stake). 

### On `InvalidDeadend`
Every staker reclaims their **full stake** (proposers: full bond; fact stakers: full stake); **no rewards**; `reward_emission` **burned back** to the reservoir; creator fee stays burned.

### Emissions — minted at CREATION from the reservoir
`reward_emission = (TOTAL_SUPPLY_CAP − kass_supply) · emission_num/den`, computed in `create_oracle` AFTER the EMA fee burn (so burning boosts the same-tx emission), **minted immediately into `stake_vault`** (program-PDA mint authority), recorded as `Oracle.reward_emission`. On `Resolved` it joins `bond_pool` in the reward pool; on `InvalidDeadend` it is burned back. No epochs; live supply is the reservoir. Mint authority = the program PDA `[b"mint_authority"]` (seed already defined as `config::MINT_AUTHORITY_SEED`); the KASS mint's authority MUST be that PDA (bootstrapping requirement).

---

## Conventions (unchanged)
TDD; `just build` before `cargo test`; clippy + fmt clean; commit trailer `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`, git author `Kassandra <hexadecifish@gmail.com>`; append-only Ix/error discriminants; re-pin `tests/state_layout.rs` on layout change. rust-analyzer false positives — rely on real cargo runs.

## Live-state entry points
- `Ix` up to `KassPrice=16`; `KassandraError` up to `InvalidConfig=26`. `Protocol` LEN 368 (emission_num/den, total_supply_cap, fee params, reward_proposer_weight/fact_weight [default 0], fact_vote_slash_num/den [reserved], etc.). `Oracle` LEN 360 (phase, resolved_option, bond_pool, total_oracle_stake, dispute_bond_total, proposer_count, surviving_count, the governable snapshots incl. reward weights + fact_vote_slash + challenge fees, stake_vault, kass_mint). `Proposer` LEN 96 (bond, original_option, claim_option, disqualified/slashed/flipped/ai_finalized, slashed_amount). `Fact` LEN 336 (stake, approve_stake, duplicate_stake, agreed, duplicate, settled, proposer). `FactVote` LEN 88 (fact, voter, stake, kind). `Market` LEN 416.
- `finalize_facts` (incremental; settles facts → bond_pool slash on rejected SUBMITTER stake; advances to AiClaim). `finalize_oracle` (plurality → Resolved+resolved_option / InvalidDeadend). `create_oracle` (EMA fee burn; loads Protocol; creates stake_vault). Guards: `load_oracle/proposer/fact/protocol/ai_claim`, `assert_*`, `assert_token_account`, `create_pda`. Oracle-PDA signing via `[b"oracle", nonce_le, [bump]]` (nonce in payload, re-derived).

---

## Tasks (full settlement, phased)

### S1 — Resolution totals + reward-pool math
- **Real reward-weight defaults:** set `config` defaults `REWARD_PROPOSER_WEIGHT`/`REWARD_FACT_WEIGHT` (e.g. 2/1, PW>FW) and `FACT_VOTE_SLASH_NUM/DEN` (e.g. 1/2 — the rejected-fact approve-voter slash); `init_protocol` defaults them; (set_config already covers reward weights + fact_vote_slash with bounds). Confirm bounds.
- **Add stamped totals to `Oracle`** (re-pin layout): `total_correct_proposer_stake: u64`, `total_approved_fact_stake: u64`, `reward_pool: u64` (finalized at resolution). 
- **`finalize_facts`:** when a fact is **agreed**, accumulate `oracle.total_approved_fact_stake += fact.stake + fact.approve_stake` (submitter + approve-voter stake that will earn the fact_rate). When a fact is **rejected** (non-duplicate), ALSO add the approve-voters' slash to bond_pool: `bond_pool += fact.stake (submitter, full) + fact.approve_stake·fact_vote_slash_frac` (aggregate — no per-vote iteration). Document; keep the existing submitter-full-slash.
- **`finalize_oracle`:** on the `Resolved` branch, stamp `total_correct_proposer_stake = Σ bond over surviving proposers with claim_option == resolved_option` (computed from the proposer tail it already iterates) and finalize `reward_pool = bond_pool + reward_emission` + the bucket split (store the buckets or recompute in claims from reward_pool + weights + totals — your call; storing `reward_pool` + reading weights/totals at claim time is fine and avoids extra fields).
- A **pure reward-math helper** (`src/reward.rs`, unit-tested): given (reward_pool, PW, FW, total_correct_proposer_stake, total_approved_fact_stake), compute proposer_bucket/fact_bucket with the empty-cohort roll-into-proposer rule; and `proposer_reward(bond)`, `fact_reward(stake)`. u128, overflow-safe, floor division (document rounding/dust direction — dust stays in stake_vault, claimable by no one / swept later; note it).
- Tests: the pure helper (buckets, empty-fact-cohort roll-in, pro-rata, dust); finalize_facts accumulates the totals + the rejected approve-voter slash; finalize_oracle stamps total_correct_proposer_stake + reward_pool. NO token movement yet (S2).

### S2 — claim_proposer / claim_fact / claim_fact_vote (returns + rewards, claim-and-close)
Three permissionless claim instructions (anyone can crank a claim for an account; funds go to the account's owner). Each: require terminal phase; load+verify the claimant account belongs to this oracle + not already claimed; compute entitlement per the matrix; transfer KASS from `stake_vault` → the claimant-owner's KASS account (program-signed by oracle PDA); **close the account** (reclaim rent → a rent-recipient; the account's authority/owner). Source ALL payouts from `stake_vault` (the contract). Decrement a "claims outstanding" sense if needed, but simplest: each claim is independent + idempotent via closing the account.
- `claim_proposer`: matrix rows for proposers (correct→bond+reward; wrong-survived→bond; disqualified→bond−slashed_amount). On `InvalidDeadend`→ full bond. Reward only on Resolved + correct. Close the Proposer account.
- `claim_fact` (submitter): agreed→stake+reward; rejected→0 (but still close + reclaim rent to submitter); duplicate-dominant→stake. InvalidDeadend→ full stake.
- `claim_fact_vote`: approve-voter on agreed→stake+reward; approve-voter on rejected→stake·(1−fact_vote_slash_frac); duplicate-voter→stake. InvalidDeadend→ full stake. Close the FactVote.
- Accounts per claim: oracle(w), the claimant account(w, closed), the owner's KASS token account(w), stake_vault(w), token program, + oracle nonce in payload for signing. Validate the destination token account ↔ the account's authority + mint==kass_mint.
- Tests: each matrix row (Resolved + InvalidDeadend); double-claim → fails (account closed/gone); rent reclaimed; KASS conservation per claim (stake_vault decremented by exactly the entitlement). Drive from a real resolved oracle (seed + finalize, or the e2e harness).

### S3 — Emissions: mint-authority bootstrap + mint-at-creation + burn-back
- **Mint-authority bootstrap:** the KASS mint's authority must be the program PDA `[b"mint_authority"]`. Update the test harness to create `kass_mint` with that PDA as the mint authority (currently the payer). Add a guard/assert that the mint authority matches at mint time.
- **`create_oracle`:** after the EMA fee burn, compute `reward_emission = (total_supply_cap − kass_supply)·emission_num/den` (u128, read from Protocol; if total_supply_cap==0 or emission_num==0 → emission 0, harmless), **mint it into `stake_vault`** (program-signed by the mint-authority PDA), record `oracle.reward_emission`. This deducts the reservoir at creation (deterministic, reservation-serialized). Set real `config` emission defaults (e.g. total_supply_cap = some cap, emission_num/den = a small fraction) — document; keep genesis sane.
- **`finalize_oracle`:** the `reward_pool = bond_pool + reward_emission` already folds it (S1). On the **InvalidDeadend** branch, **burn `reward_emission` back** from `stake_vault` (program-signed) to return it to the reservoir (so a dead-end doesn't leak emission). 
- Tests: create_oracle mints reward_emission into stake_vault (supply rises by emission, reservoir shrinks); burning fee first boosts the same-tx emission; reward_pool includes emission on Resolved (a correct proposer's reward reflects bond_pool+emission); InvalidDeadend burns the emission back (supply returns); claim payouts now include emission-funded rewards. Mint-authority mismatch → rejected.

### S4 — Account closure: close_ai_claim + Market/escrow rent reclaim
- **`close_ai_claim`** (permissionless, post-resolution): close each `AiClaim` account (reclaim rent → its proposer/authority). Incremental/per-claim (no one-tx cap). 
- **Market/escrow closure:** a `close_market` (or fold into the last claim) that, once the market is `settled` AND the oracle terminal, closes the `Market` PDA + the `challenger_usdc_vault` escrow (reclaim rent → challenger), per the challenge-milestone deferral. Ensure the escrow is empty (settle already drained it).
- Tests: close_ai_claim after Resolved reclaims rent; before resolution → fails; close_market after settle reclaims rent; non-empty/unsettled → fails.

### S5 — End-to-end settlement + conservation fuzz
- **E2E:** full lifecycle (create → propose → [resolve uncontested] AND [dispute → ... → finalize_oracle Resolved]) → every staker claims → assert each entitlement (matrix), all accounts closed, `stake_vault` drained to dust, KASS conservation. Also an InvalidDeadend path → everyone reclaims full stake + emission burned.
- **Conservation fuzz:** a proptest arm that, for a resolved oracle with fuzzed bonds/fact-stakes/votes/outcome, asserts: `Σ all claim payouts + dust == stake_vault initial (= Σ stakes + reward_emission, on Resolved)`; on InvalidDeadend `Σ payouts == Σ stakes` (emission burned). Independent reference for the bucket/pro-rata math. Modest case count.
- Update the existing `invariants.rs` conservation arms to account for the now-physical settlement (no longer counter-only).

---

## Out of scope (future)
- KASS bootstrapping presale-avoidance details beyond the emission curve; the runner/SDK/app; MetaDAO futarchy proposal-lifecycle on a real validator (seam already done); migrating challenge markets to v0.6.
- Dust sweeping (floor-division remainder in stake_vault) — note it; a governance sweep can reclaim it later.

## Execution note
After each task: `just build` → `cargo test` → clippy/fmt, green, commit. Re-pin layouts. The riskiest spots: the mint-authority bootstrap + mint-at-creation (S3, real KASS supply change — validate the harness mint-authority setup), and the conservation math across the whole settlement (S5). Source payouts from stake_vault/bond_pool, NEVER total_oracle_stake. Append an S1–S5 delta log here.

---

## Delta log

### S1 — Resolution totals + reward-pool math (DONE; no token movement)

**Real config defaults** (`src/config.rs`): `REWARD_PROPOSER_WEIGHT = 2`,
`REWARD_FACT_WEIGHT = 1` (PW>FW), `FACT_VOTE_SLASH_NUM/DEN = 1/2`. `init_protocol`
now defaults the Protocol copies to these consts (was 0/0 weights, 0/1 slash);
`create_oracle` snapshots them onto each Oracle as before. All `set_config`
bounds hold (at least one reward weight > 0; fact_vote_slash den > 0, num ≤ den;
the joint flip+success-fee ≤ 1 is unaffected) — every existing set_config bound
test stays green.

**Oracle layout re-pinned** — `Oracle::LEN 360 → 384`. Three new `u64` fields
appended after the C1 challenge-fee block (all 0 at create / pre-resolution):
`total_correct_proposer_stake @360`, `total_approved_fact_stake @368`,
`reward_pool @376`. `tests/state_layout.rs` updated (LEN 384 + the 3 offsets).
Other struct LENs unchanged.

**`finalize_facts` accumulation** (still NO token CPI; `bond_pool` a counter):
- AGREED fact → `total_approved_fact_stake += fact.stake + fact.approve_stake`
  (checked adds): submitter + approve-voter stake that earns the fact rate.
- REJECTED (non-duplicate) fact → `bond_pool += fact.stake` (submitter full, as
  before) **AND** `bond_pool += fact.approve_stake · fact_vote_slash_num /
  fact_vote_slash_den` (u128 floor) — the aggregate approve-voter slash, no
  per-vote iteration. Approve-voters later (S2) reclaim `stake·(1 − slash_frac)`.
- DUPLICATE-dominant → unchanged (no slash, not counted into approved totals).

**`finalize_oracle` (Resolved branch)**: stamps `total_correct_proposer_stake =
Σ proposer.bond over survivors with claim_option == resolved_option` (gathered
alongside the existing vote scan) and finalizes `reward_pool = bond_pool` (clear
comment: S3 folds `reward_emission` in here). InvalidDeadend leaves both 0.

**`src/reward.rs`** (new `pub mod reward;`, pure/allocation-free, mirrors
`plurality.rs`):
- `reward_buckets(reward_pool, pw, fw, total_correct, total_approved) ->
  (proposer_bucket, fact_bucket)` — split by PW/(PW+FW), FW/(PW+FW); empty-cohort
  roll-in (approved==0 → all to proposer; correct==0 → all to fact; both-empty &
  pw+fw==0 → proposer fallback, no divide-by-zero). u128, floor.
- `proposer_reward(bond, bucket, total)` / `fact_reward(stake, bucket, total)` —
  pro-rata, u128 floor, 0 when total==0.
- Rounding/dust: floor everywhere → `Σ rewards ≤ reward_pool`; the remainder
  stays in `stake_vault`, un-claimable this milestone (future sweep — see Out of
  scope). 13 unit tests (split, dust, empty-cohort roll-in, pro-rata, zero/denom
  guards, overflow).

**Tests**: reward.rs unit tests; `finalize_facts.rs` — agreed accumulates
`stake+approve_stake`, rejected adds `stake + approve·slash_frac` (via new
`set_fact_vote_slash` harness setter; default seed stays 0/1 so existing
fixtures + `invariants.rs` Arm A remain pure counters), duplicate doesn't;
`finalize_oracle.rs` — Resolved stamps the correct-proposer total + `reward_pool
== bond_pool` (incl. a wrong-but-survived exclusion + non-zero bond_pool case),
InvalidDeadend leaves 0/0. Full suite: 199 passed / 0 failed; clippy + fmt clean.

> NOTE for S5: the harness `seed_disputed_oracle` keeps `fact_vote_slash = 0/1`
> (pure-counter) deliberately, so `invariants.rs` Arm A was NOT touched. When S5
> makes settlement physical, fold the approve-voter slash into that reference
> model (and consider flipping the harness default to the real 1/2).

### S2 — claim_proposer / claim_fact / claim_fact_vote (DONE; first physical payouts)

**Ix discriminants appended** (`instruction.rs`, stable contract): `ClaimProposer
= 17`, `ClaimFact = 18`, `ClaimFactVote = 19` (+ `from_u8` arms). Dispatched in
`processor/mod.rs` to `claims::claim_proposer/claim_fact/claim_fact_vote`. NO
layout change (no new account fields) — `state_layout.rs` untouched.

**New processor** `src/processor/claims.rs` — three permissionless claim-and-close
instructions. Each: `require_terminal` (Resolved/InvalidDeadend, else `WrongPhase`;
returns a `resolved: bool` so rewards apply ONLY on Resolved); re-derive +
verify the oracle PDA from the payload nonce (same scheme as `settle_challenge`);
`assert_key(stake_vault == oracle.stake_vault)`; load + type-check the claimant +
bind it to this oracle; `assert_token_account(dest, oracle.kass_mint,
claimant.authority)` + `assert_key(rent_recipient == claimant.authority)`; compute
the entitlement; `payout_and_close` — program-signed `Transfer` from `stake_vault`
(oracle-PDA signer `[b"oracle", nonce_le, [bump]]`; skipped when amount==0) then
CLOSE the claimant account.

**Account orders / payload** (payload = `oracle_nonce` u64 LE for every claim):
- `claim_proposer` (17): `[0] oracle(ro) [1] proposer(w, closed) [2] dest_kass(w)
  [3] stake_vault(w) [4] rent_recipient(w == proposer.authority) [5] token prog`.
- `claim_fact` (18): same shape with `[1] fact(w, closed)`, dest/rent bound to
  `fact.proposer` (the SUBMITTER authority).
- `claim_fact_vote` (19): `[0] oracle(ro) [1] fact_vote(w, closed) [2] fact(w —
  decremented, NOT closed) [3] dest_kass(w) [4] stake_vault(w) [5] rent_recipient
  (w == fact_vote.voter) [6] token prog`. FactVote carries no `oracle` field, so
  it is bound through the fact: `vote.fact == fact_ai` AND `fact.oracle == oracle`.
  Oracle is READ-ONLY everywhere (claims never mutate it — they only read the
  resolution stamps + sign as the PDA).

**Matrix impl** (reward buckets via `reward::reward_buckets(reward_pool, pw, fw,
total_correct, total_approved)`, then `proposer_reward`/`fact_reward`; u128 floor):
- proposer (UNIFORM base, corrected in the S2 follow-up): `entitlement = (bond −
  slashed_amount) + (resolved && !is_disqualified() && claim_option ==
  resolved_option ? proposer_reward(bond, proposer_bucket, total_correct) : 0)`,
  on BOTH terminal phases. Any `slashed_amount` (no-show/flip/challenge-fail/
  no-facts) already funded `bond_pool`, so deducting it everywhere prevents the
  flip-slashed-but-surviving double-pay (full bond returned AND the slash paid out
  as rewards). Honest survivor → `bond` (+reward if correct); disqualified →
  `bond − slashed_amount`; flipped survivor → `bond − flip_slash` (+reward if
  correct). Reward weight stays `bond` (== S1's `total_correct_proposer_stake =
  Σbond`), so `Σ(bond − slashed) + reward_pool = Σbond` — conservation holds.
- fact (submitter): InvalidDeadend→`stake`; agreed→`stake + fact_reward`;
  duplicate→`stake`; rejected→`0` (still close + reclaim rent).
- fact vote: InvalidDeadend→`stake`; `VOTE_DUPLICATE`→`stake`; approve+agreed→
  `stake + fact_reward`; approve+duplicate-fact→`stake`; approve+rejected→
  `stake − floor(stake·fact_vote_slash_num/den)`.

**Close pattern** (pinocchio 0.8 `AccountInfo::close`): drain the claimant's
lamports into the rent recipient (`*to += *from; *from = 0` in a scoped lamports
borrow), then `claimant.close()` (zeros owner/lamports/data_len). Idempotent BY
CLOSURE — a second claim finds the account reaped (0 lamports → owner no longer
the program) and fails the load guard with `InvalidAccount`.

**Fact-close ordering hazard + fix** (the genuinely new bit): `claim_fact` closes
the `Fact`, but `claim_fact_vote` must READ the Fact's disposition. A submitter
claiming first would strand every voter. Fix: each `claim_fact_vote` decrements
the Fact's running `approve_stake`/`duplicate_stake` (its own stake), and
`claim_fact` refuses to close while either is non-zero → new error
`VotersOutstanding = 27` (`error.rs`). So the submitter's claim runs LAST,
permissionlessly safe (no one can close the Fact early). Vote rewards read the
oracle-level `total_approved_fact_stake` (immutable stamp), so decrementing the
per-fact total never perturbs an entitlement.

**Conservation**: every payout sourced from `stake_vault` (+ the per-account
`slashed_amount` ledger + the oracle stamps); `total_oracle_stake` is NEVER read.
`reward_pool` (stamped) == the physically-slashed KASS, so Σ entitlements + floor
dust == vault on Resolved, and Σ == Σ stakes (vault drained to 0) on InvalidDeadend.

**Harness** (`tests/common/mod.rs`): `seed_terminal_oracle(phase, resolved_option,
&[ClaimProposerSpec], &[ClaimFactSpec{votes:[ClaimVoteSpec]}], slash_num,
slash_den) -> TerminalSeed` fabricates a terminal oracle with a vault funded to
EXACTLY Σ bonds+stakes and self-consistent stamps (computes `reward_pool`,
`total_correct_proposer_stake`, `total_approved_fact_stake`, and each claimant's
expected entitlement via the program's own `reward` helpers). Plus
`claim_proposer_ix`/`claim_fact_ix`/`claim_fact_vote_ix`, `lamports`, `is_closed`.

**Tests** (`tests/claims.rs`, 8): `resolved_proposer_matrix` (2666/1000/0),
`resolved_fact_and_vote_matrix` (submitters 1416/0/1000; votes 708·708 / 200·300 /
500·300), `resolved_conservation_sweep` (Σ + 2 dust == 8800 vault; dust ≤
reward_pool), `invalid_deadend_full_returns` (full stakes, vault drained to 0,
Σ == vault), `double_claim_fails_account_gone` (`InvalidAccount`),
`submitter_before_voters_rejected` (`VotersOutstanding`), `dest_owner_mismatch_rejected`
(`InvalidAccount`), `non_terminal_oracle_rejected` (`WrongPhase`). For each matrix
row the test asserts the exact dest KASS delta, the account closed (rent reclaimed
to its authority), and the vault decremented by exactly the entitlement. Full
suite: 207 passed / 0 failed; clippy + fmt clean.

> FOLLOW-UP 1 (FIXED): the flip-slashed-but-surviving over-pay flagged here is now
> closed — `claim_proposer` deducts `slashed_amount` for surviving proposers
> (uniform base), and the builder's `reward_pool` sums `slashed_amount` over every
> proposer (not just disqualified). Test `flipped_survivor_not_overpaid`:
> honest-correct 1500, flipped-correct 1000 (= 1000 − 500 + 500, NOT 1500),
> flipped-wrong 500 (= 1000 − 500, no reward); Σ + dust == vault with the flipped
> survivors present.

### S2 review fixes — disqualified forfeit + ceil voter slash (DONE)

**C1 (Critical, fund-safety) — disqualified base is 0, NOT `bond − slashed_amount`.**
FOLLOW-UP 1 over-corrected: for a CHALLENGE-disqualified proposer `settle_challenge`
sets `slashed_amount = bond − kass_fee` (the bond_pool contribution) AND separately
sent `kass_fee` OUT of `stake_vault` to the challenger. So `bond − slashed_amount =
kass_fee` would re-pay the fraudster KASS that already left the vault → shortfall.
Fix (`claims.rs`): `base = is_disqualified() ? 0 : bond − slashed_amount`. No-op for
no-show / no-facts dead-end (`slashed_amount == bond`); corrects only the
challenge-disqualify row to 0. A disqualified proposer FORFEITS the whole bond.
Test `disqualified_forfeits_full_bond` (seeded `slashed_amount = 900 < bond 1000`,
kass_fee 100): claim pays **0**, the 100 stays as conservation-safe dust. (The real
`settle_challenge → finalize_oracle → claim` chain is MetaDAO-CPI-heavy; the seeded
minimum the review authorized is used.)

**I1 (Important, rounding/conservation) — CEIL the per-voter rejected-fact slash.**
`finalize_facts` credits `bond_pool` with the AGGREGATE `floor(Σ approve_stake·r)`,
but each approve-voter is slashed per-voter. With `floor` per voter, `Σ floor ≤
floor(Σ)`, so the vault could retain LESS than the bond_pool credit → last reward
claimant short. Fix (`claims.rs::slash_amount`): slash each rejected-fact
approve-voter `ceil(stake·num/den) = (stake·num + den − 1)/den` (u128), so `Σ ceil ≥
(Σ stake)·r ≥ floor(Σ·r)` — vault never short, excess is sub-unit dust. Test
`ceil_voter_slash_no_shortfall` (odd stakes 401/601, r=1/2): bond_pool credit 501,
floor-per-voter would retain 500 (proposer reward claimed LAST would fail by 1);
ceil retains 502, every claim succeeds, 1 dust.

**M1 (doc):** `require_terminal` now documents that an oracle force-resolved via
`resolve_deadend` (F4) carries `reward_pool == 0` + zero totals, so claims pay
stakes-back / no rewards (matches the deferred dead-end-settlement intent; no
behavior change).

**M2 (doc + test):** `flipped_survivor_invalid_deadend_strands_to_dust` — a
flip-slashed SURVIVING proposer that ties into `InvalidDeadend` gets `bond −
slashed_amount` (reward_pool 0); the flip-slash portion stays as conservation-safe
vault dust (under-pay, never over-pay).

Full suite **211 passed / 0 failed**; clippy + fmt clean.

### S3 — Emissions: mint-auth PDA + mint-at-creation + deadend burn-back (DONE; real KASS supply changes)

**Oracle layout re-pinned** — `Oracle::LEN 384 → 392`. One new `u64`,
`reward_emission @384`, appended after the S1 `reward_pool`: the KASS minted into
`stake_vault` at creation. `tests/state_layout.rs` updated (LEN 392 + offset 384).
All other struct LENs unchanged. 0 at create when emission is disabled.

**Emission config defaults / reservoir formula** (`src/config.rs`): added
`TOTAL_SUPPLY_CAP = 1e9·1e9 = 1e18` (1e9 KASS at 9 dp), `EMISSION_NUM = 1`,
`EMISSION_DEN = 1_000_000` (mint 1/1_000_000 of the remaining reservoir per
oracle). The reservoir formula:
`reward_emission = floor((TOTAL_SUPPLY_CAP − kass_supply) · EMISSION_NUM/DEN)`
(u128 intermediate, overflow-safe; `emission ≤ reservoir ≤ cap ≤ u64::MAX`).
**Decision — emission DISABLED at genesis:** `init_protocol` was left UNCHANGED
(`emission_num = 0`, `emission_den = 1`, `total_supply_cap = 0`), so a fresh
`Protocol` mints nothing. Reasoning: a supply cap below the live supply is
meaningless and the cap is a deliberate governance choice, so genesis cap 0 is
the only safe default — and it keeps every existing real-flow conservation test
(`stake_vault == total_oracle_stake == Σ bonds`) exactly true. The consts are the
RECOMMENDED governance values, enabled via `set_config` (its existing emission
bounds — `emission_den > 0`, `emission_num ≤ emission_den` — already cover the
rate; `total_supply_cap` stays unbounded/0-allowed, no new bound). `governance_setup.rs`'s
"defaults are 0/1/0" assertion stays green.

**Mint-authority bootstrap (harness):** the KASS mint's SPL authority is now the
program **mint-authority PDA** `[b"mint_authority"]` (was the payer). Because the
harness fabricates ALL token balances directly (`create_token_account` writes the
balance, `add_mint_supply` rewrites the mint supply field) and the creation-fee
`Burn` is authorized by the token-account OWNER (the creator), NOT the mint
authority, handing the mint authority to the PDA changed no existing funding —
every staker is still funded as before, and ONLY the program's emission `MintTo`
uses the authority. USDC's authority stays the payer. (So the "mint to payer
first, then set_authority" dance the brief described is unneeded here — this
harness never does a real `MintTo`.) New harness helpers: `mint_authority_pda`,
`set_reward_emission` (stamps the field + funds the vault + bumps supply so the
deadend burn has real tokens), `set_kass_mint_authority` (points the mint at a
non-PDA key for the mismatch test), `add_token_balance`, and a shared
`finalize_oracle_ix`.

**`create_oracle` — mint emission (after the fee burn):** reads `kass_mint`
supply AFTER the `Burn` CPI (so the burn boosts the same-tx reservoir), computes
`reward_emission`; if `> 0`, asserts the mint-auth PDA == `kass_mint.mint_authority`
(`BadMintAuthority` else) and CPI `MintTo`s it into `stake_vault`, **program-signed
by `[b"mint_authority", bump]`** (the MintTo signer). Records `oracle.reward_emission`.
NEW account appended: **9. mint_authority PDA** (`new_readonly`). New error
**`BadMintAuthority = 28`**. SPL `Mint` read at fixed offsets (authority COption
tag@0/key@4, supply@36).

**`finalize_oracle` — fold-in / burn-back:** Resolved branch now stamps
`reward_pool = bond_pool + reward_emission` (checked_add). InvalidDeadend branch
**burns `reward_emission` back** from `stake_vault` against `kass_mint`, **signed
by the ORACLE PDA `[b"oracle", nonce_le, bump]`** (the vault's token authority —
a DIFFERENT signer from the MintTo's mint-auth PDA), only when `> 0`; `reward_pool`
stays 0. New payload: **`oracle_nonce: u64 LE` (8 bytes)** (re-derives the oracle
PDA signer). New account order: **`[0] oracle(w) [1] kass_mint(w) [2] stake_vault(w)
[3] token program [4..] proposer tail`**. `load_oracle` runs BEFORE the payload/
fixed-account parse so a bad-owner oracle still fails `InvalidAccount` (dispatch
contract preserved); all gating errors (WrongPhase/WindowNotElapsed/Challenges
Outstanding) also precede the payload parse. The 3 local `finalize_oracle_ix`
test builders (finalize_oracle/invariants/lifecycle_e2e) now delegate to the
shared harness builder.

**Conservation:** on Resolved, `stake_vault_in = Σ stakes + reward_emission`, and
`reward_pool` includes the emission, so Σ claims + dust == vault. On InvalidDeadend
the emission is burned, leaving `Σ stakes` for the full-stake claims (no leak).

**Tests** (`tests/emissions.rs`, 7): `create_oracle_mints_emission_into_vault`
(supply +E, reservoir −E, vault == E, stamp set), `fee_burn_boosts_emission`
(E uses post-burn supply, strictly > the pre-burn value → ordering proof),
`resolved_folds_emission_into_reward_pool_and_claim` (reward_pool == bond_pool + E;
chained claim pays `bond + emission-funded reward`), `invalid_deadend_burns_emission_back`
(vault back to Σ stakes, supply −E, stamp retained), `mint_authority_mismatch_rejected`
(`BadMintAuthority`), `cap_zero_emits_nothing` + `emission_num_zero_emits_nothing`
(disabled is harmless). Full suite **218 passed / 0 failed**; clippy + fmt clean.

### S4 — Account closure: close_ai_claim + close_market/escrow rent reclaim (DONE)

**Ix discriminants appended** (`instruction.rs`, stable contract): `CloseAiClaim
= 20`, `CloseMarket = 21` (+ `from_u8` arms). Dispatched in `processor/mod.rs` to
`close_ai_claim::process` / `close_market::process`. NO layout change (no new
account fields) — `state_layout.rs` untouched.

**New errors** (`error.rs`, appended): `MarketNotSettled = 29` (close_market on an
unsettled Market), `EscrowNotEmpty = 30` (close_market while the escrow USDC
balance ≠ 0).

**Part A — `close_ai_claim` (Ix 20)** `src/processor/close_ai_claim.rs`.
Permissionless, post-resolution rent reclaim of one `AiClaim` (it holds NO tokens
→ NO token movement, pure lamport drain + `close()`). Accounts: `[0] oracle(ro,
terminal) [1] ai_claim(w, closed) [2] proposer(ro) [3] rent_recipient(w ==
proposer.authority)`. EMPTY payload (no PDA signature needed). Gating: oracle
TERMINAL (`Resolved`/`InvalidDeadend`, else `WrongPhase`); `ai_claim.oracle ==
oracle`; the rent recipient is bound by reading the still-present `Proposer`:
`ai_claim.proposer == proposer_ai.key()` AND `proposer.oracle == oracle`, then
`rent_recipient == proposer.authority`. Idempotent by closure (2nd call → reaped →
`InvalidAccount`).
- **Rent-binding + ordering DECISION:** `AiClaim` stores `proposer` = the Proposer
  PDA key, NOT the human authority. Rather than duplicate the authority onto the
  `AiClaim` (a layout change), we read `proposer.authority` from the live Proposer.
  CONSEQUENCE: `close_ai_claim` MUST run BEFORE `claim_proposer` closes that
  Proposer (else `load_proposer` fails `InvalidAccount` and the caller just cranks
  in order). This mirrors the S2 fact-close ordering (`claim_fact` runs last after
  every `claim_fact_vote`): a cheap permissionless ordering constraint, no stranded
  rent (the rent is the proposer's regardless of order).

**Part B — `close_market` (Ix 21)** `src/processor/close_market.rs`. The
challenge-milestone DEFERRAL ("Deferred rent reclamation"): `settle_challenge`
sets `market.settled=1` + drains the escrow but never closes `Market` /
`challenger_usdc_vault`. Accounts: `[0] oracle(ro, terminal; escrow's token
authority) [1] market(w, closed) [2] challenger_usdc_vault(w, closed SPL acct)
[3] rent_recipient(w == market.challenger) [4] token program`. Payload =
`oracle_nonce: u64 LE` (re-derives the oracle PDA signer `[b"oracle", nonce_le,
[bump]]`). Gating: oracle TERMINAL; `market.oracle == oracle`; `market.settled ==
1` (else `MarketNotSettled`); `challenger_usdc_vault == market.challenger_usdc_vault`;
`rent_recipient == market.challenger`; escrow SPL `amount == 0` read at offset 64
(else `EscrowNotEmpty`).
- **Escrow CloseAccount approach:** the escrow is a TOKEN-program-owned account, so
  its rent is reclaimed via the SPL `CloseAccount` CPI (pinocchio-token 0.3
  `CloseAccount{account, destination, authority}` → data `[9]`, accounts
  `[escrow(w), rent_recipient(w), oracle(signer)]`), **program-signed by the oracle
  PDA** (the escrow's token authority), sending the escrow's rent to the recipient.
  SPL requires a 0 balance to close; we assert `amount == 0` first so the failure is
  loud + local. CLOSE ORDER: escrow `CloseAccount` FIRST, THEN the `Market` PDA
  (manual lamport drain + `close()`), both rents → challenger. No fund movement
  beyond rent + the (already-zero) escrow close. Idempotent by closure.

**Part C — S3-review polish (2 fixes):**
1. `create_oracle.rs::compute_reward_emission` — the reservoir→u64 emission now
   `.min(reservoir)` so the `as u64` cast is self-protecting: a future bad config
   (`emission_num > emission_den`) can never mint MORE than the reservoir holds. No
   behavior change for any valid config (`num ≤ den` already keeps `emission ≤
   reservoir`).
2. `finalize_oracle.rs` (~line 164) — the oracle-PDA nonce bump var was named
   `_bump` but used (`_bump != oracle.bump`); renamed to `bump`. Cosmetic.

**Tests** (`tests/closure.rs`, 9): close_ai_claim — `after_resolved_reclaims_rent`
(closed + rent → proposer authority), `non_terminal_fails` (`WrongPhase`),
`double_close_fails` (`InvalidAccount`), `other_oracle_fails` (`InvalidAccount`);
close_market — `after_settle_reclaims_rent` (Market + escrow closed, both rents →
challenger), `unsettled_fails` (`MarketNotSettled`), `nonempty_escrow_fails`
(`EscrowNotEmpty`), `double_close_fails` (`InvalidAccount`), `non_terminal_fails`
(`WrongPhase`). New harness helpers: `seed_ai_claim`, `seed_market`,
`seed_usdc_escrow`, `close_ai_claim_ix`, `close_market_ix`, `airdrop`. Full suite
**227 passed / 0 failed**; clippy `--all-targets` clean; fmt applied.

### S4 review fix — store `authority` on `AiClaim` (close_ai_claim never strands rent) (DONE)

**Finding:** the first cut of `close_ai_claim` routed rent to `proposer.authority`
read from the LIVE `Proposer`, but `claim_proposer` closes the `Proposer` with no
outstanding-AiClaim guard — so cranking `claim_proposer` first permanently
stranded the AiClaim's rent (~0.0016 SOL, the proposer's own), and the doc wrongly
implied the ordering was merely a convention.

**Fix (order-independent, no Proposer dependency):**
- **`AiClaim` layout re-pinned** `176 → 208`: appended `authority: Pubkey @176`
  (the proposer's human authority). Clean ABI addition — all prior offsets
  unchanged. `tests/state_layout.rs` updated (LEN 208 + offset 176).
- **`submit_ai_claim`** now stamps `claim.authority = *authority_ai.key()` (==
  `proposer.authority`, already asserted at submit). Harness `seed_ai_claim` gained
  an `authority` param; the `settle_challenge.rs` AiClaim fabricator needs no change
  (never reads `authority`; the larger zeroed struct seeds fine).
- **`close_ai_claim` rewritten** to bind `rent_recipient == ai_claim.authority`
  DIRECTLY and DROP the `Proposer` account. New account order: `[0] oracle(ro,
  terminal) [1] ai_claim(w, closed) [2] rent_recipient(w == ai_claim.authority)`.
  Empty payload. Works regardless of whether `claim_proposer` already closed the
  Proposer; rent never stranded. Doc fixed to state the truth (order-independent).
- **New test** `close_ai_claim_after_proposer_closed_still_reclaims`: cranks
  `claim_proposer` (closing the Proposer) THEN `close_ai_claim` → AiClaim rent
  still reclaimed to `authority`. Other closure tests updated for the simplified
  account list.

Full suite **228 passed / 0 failed**; clippy `--all-targets` clean; fmt applied.

### S5 — End-to-end settlement + conservation fuzz (DONE; test-only, no production change)

**No production change.** S5 is pure TEST work — no genuine settlement bug
surfaced. The full real-instruction claim/close/emission path drains a resolved
oracle, the matrix + conservation hold, and the independent-reference fuzz found
no conservation violation.

**E2E lifecycle (`tests/settlement_e2e.rs`, 5 tests).** Each drives a lifecycle
to a terminal state then runs EVERY real claim + close, asserting the per-actor
matrix, all accounts closed, the vault drained to dust, and KASS conservation
sourced ONLY from `stake_vault`:
- `e2e_resolved_full_settlement_real_dispute` — the gold standard: the WHOLE
  chain is real (`create_oracle → propose×2 conflict → finalize_proposals →
  submit_fact → advance_phase → vote_fact → finalize_facts → submit_ai_claim×2 →
  finalize_ai_claims → finalize_oracle Resolved`), then real `claim_fact_vote` /
  `claim_fact` / `claim_proposer` + `close_ai_claim`. Exercises the correct
  survivor (bond+reward), the flip-slashed-but-correct survivor
  (`bond−slash+reward`), an agreed-fact submitter + approve-voter (stake+reward).
  `Σ claims + dust == vault_initial` (3 base units of floor dust).
- `e2e_invalid_deadend_full_returns_real_dispute` — same real chain, AI claims
  0/1 → plurality tie → InvalidDeadend; every staker reclaims full stake, vault
  drains to 0.
- `e2e_resolved_with_emission_real_finalize_and_claims` — emission placed in the
  vault, REAL `finalize_oracle` folds it into `reward_pool`, a correct proposer's
  claim reflects the emission-boosted reward, `Σ claims + dust == Σ stakes +
  emission`.
- `e2e_invalid_deadend_emission_burned_full_returns` — REAL `finalize_oracle`
  BURNS the emission back (supply returns), full-stake returns, vault drains to 0.
- `e2e_deadend_after_settled_challenge_with_emission` — **the S3-flagged combo**:
  a settled-challenge disqualify (`kass_fee` out, bond_pool credit), survivors
  tie → InvalidDeadend, REAL emission burn-back, the disqualified proposer
  forfeits (0), survivors reclaim full bonds, + REAL `close_ai_claim` /
  `close_market` (escrow `CloseAccount`). Full accounting: `payouts + dust +
  kass_fee_out + emission_burned == Σ bonds + emission`.

**Real vs seeded (e2e):** tests 1-2 are fully real (no `set_phase`; only `warp`).
Tests 3-5 SEED the disputed oracle (dispute mechanics covered by
`lifecycle_e2e` / `invariants` Arm A) but keep the emission MOVEMENT
(`finalize_oracle` fold/burn) + all claims/closes REAL. The mint-AT-CREATION half
of emission is covered by `tests/emissions.rs` (real `create_oracle`).

**Conservation fuzz (`tests/invariants.rs`, Arms D + E, 48 cases each).** A
PHYSICAL-settlement fuzz with emission enabled, asserted against an INDEPENDENT
reference that REIMPLEMENTS the bucket / pro-rata / ceil-slash math (`ref_buckets`
/ `ref_share` / `ref_ceil_slash` — it never calls `kassandra_program::reward`):
- **Arm D `resolved_settlement_conservation`** — fuzzes proposers (bond ×
  correct/wrong × disqualified × slash%), facts (agreed/rejected/duplicate × votes
  approve/duplicate), and emission (0..2000). Seeds the terminal oracle, folds the
  emission, then runs every real claim (+ `close_ai_claim`). Asserts each payout
  == the independent reference, that NO claim runs the vault short (reward
  receivers claimed last would expose a shortfall), and `Σ payouts + dust == Σ
  stakes + reward_emission`. The dust bound allows the conservation-SAFE surpluses
  (floor reward remainder + disqualified-forfeit `bond−slashed_amount` + the S2
  per-voter ceil-slash margin).
- **Arm E `deadend_settlement_conservation`** — fuzzes bonds + emission; a
  2-proposer disputed oracle whose survivors tie, REAL `finalize_oracle` burns the
  emission back, full-stake returns: `Σ payouts == Σ stakes`, vault → 0, supply
  returns by the burned emission.

**invariants.rs arm split (documented in the module header + an in-file banner):**
Arms **A/B/C** are LEFT as the original counter-only / pre-settlement fuzz
(emission disabled, no claims, asserting the dispute-core ledger at the terminal
counter state) — unchanged and still green. Arms **D/E** are ADDED for the
post-settlement physical sweep. The split is explicit so each arm's contract is
clear.

**Harness additions (`tests/common/mod.rs`):** `fold_reward_emission` (mirrors
the create-mint + finalize-fold: adds emission to vault+supply, stamps
`reward_emission`, folds into `reward_pool` — for the Resolved fuzz seed),
`seed_challenge_disqualify` (the post-settle state: disqualify + slashed_amount +
bond_pool credit + surviving_count-- + remove `kass_fee` from the vault), and a
private `sub_token_balance`.

**Two seeded-model dust quirks surfaced + understood (NOT bugs):** (1) a seeded
disqualified proposer with `slashed_amount < bond` strands the un-credited
remainder (the `kass_fee` that, in the real flow, already left the vault) as dust;
(2) the rejected-fact CEIL voter slash retains up to 1 unit per voter over the
FLOOR bond_pool credit. Both are conservation-SAFE under-pays, modelled in the
Arm D dust bound. The exact-accounting equation `Σ payouts + dust == vault_initial`
holds in every case.

Full suite **235 passed / 0 failed** (228 baseline + 5 e2e + 2 new fuzz arms);
`cargo clippy --all-targets` clean; `cargo fmt` applied.

---

## Staker settlement: covered vs deferred (final)

**Covered (real instructions, end to end):**
- Per-staker pull claims — `claim_proposer` / `claim_fact` / `claim_fact_vote`:
  every matrix row (correct/wrong/disqualified/flipped proposer; agreed/rejected/
  duplicate fact submitter; approve-agreed / approve-rejected ceil-slash /
  duplicate / approve-on-duplicate voter), on both terminal phases.
- Reward distribution from `bond_pool` (cohort buckets + pro-rata) and emission
  folded into `reward_pool` on Resolved / burned back on InvalidDeadend.
- Emission minted at creation from the reservoir (fee-burn boost, mint-authority
  guard, disabled-at-genesis) — `tests/emissions.rs`.
- Account closure — `close_ai_claim` (order-independent) + `close_market` (escrow
  `CloseAccount` + Market close).
- KASS conservation sourced from `stake_vault` / `slashed_amount` / resolution
  stamps (NEVER `total_oracle_stake`), proven by the e2e sweeps + the
  independent-reference fuzz (Resolved + InvalidDeadend + deadend-after-settled-
  challenge, all with emission).

**Deferred (known, documented):**
- **Dust sweeping** — floor-division remainders (bucket split + pro-rata) and the
  conservation-safe surpluses (disqualified forfeit remainder, ceil-slash margin)
  remain in `stake_vault`, claimable by no one this milestone. A future
  governance sweep can reclaim the residual (noted in `reward.rs` + "Out of
  scope"). Always an under-pay — never an over-pay, so no claimant is ever short.
- KASS bootstrapping presale-avoidance beyond the emission curve; the runner/SDK/
  app; MetaDAO proposal-lifecycle on a real validator; v0.6 market migration
  (all per the original Out-of-scope).

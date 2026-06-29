# Kassandra — Design Document

**Date:** 2026-06-29
**Status:** Design (pre-implementation)

## 1. Overview

Kassandra is a decentralized, AI-assisted **optimistic oracle on Solana** that resolves
**binary / categorical** questions. The common case is cheap: uncontested proposals settle
with no AI and no markets. The dispute machinery — fact agreement, AI claims, and
decision markets — only fires when proposers disagree.

The novel idea: **interpretation is fixed at oracle creation**, so disputes reduce to
*which evidence is real/relevant* (objective) rather than *what the evidence means*
(subjective). An AI applies the fixed interpretation to an agreed fact set, and a
MetaDAO-style decision market is the ultimate arbiter that can override a faulty AI claim.

No zkTLS, no TEEs. Honesty is enforced **economically** (KASS staking/slashing) and by
**markets** (the final arbiter of truth).

### Scope

- **Question type:** binary / categorical only. (Median = mode/plurality over discrete options.)
- **v1 deliverable:** this full design doc. Implementation milestones scoped afterward.

## 2. Architecture

### Monorepo layout

- `programs/kassandra/` — core program in **Pinocchio** (not Anchor). Owns: oracle
  requests, proposal windows, fact proposal/voting, AI-claim registry, plurality
  computation, slash/recompute, KASS staking & emissions, dynamic fee.
- **MetaDAO integration** — reuse MetaDAO's deployed **conditional-vault + AMM** programs
  for the pass/fail decision markets via **CPI**. We do not reimplement the vault/AMM.
- `runner/` — open-source AI runner (CLI + library). Input: creation-time prompt +
  interpretation + agreed fact set + options → pinned deterministic model → categorical
  answer + claim metadata. Anyone runs it to propose *or* to verify before challenging.
- `sdk/` — hand-written TypeScript client (no IDL; account layouts shared with the program).
- `app/` — frontend for creating oracles, proposing, fact-voting, trading markets.

### Pinocchio implications

- Manual account deserialization/validation and manual instruction dispatch (no Anchor
  macros/IDL).
- **CPI into MetaDAO's Anchor programs by hand:** construct their 8-byte Anchor sighash
  discriminators + account metas + Borsh args ourselves ("rewriting MetaDAO client code").
- Hand-written SDK and shared account-layout definitions.
- Trade-off accepted: more manual/unsafe serialization in exchange for a smaller,
  cheaper, dependency-light program.

### On-chain vs off-chain

- **On-chain:** request config (prompt, interpretation rules, options, deadline), all
  stakes/bonds (KASS) and market collateral (USDC), fact set & approvals, AI-claim
  metadata (model, params, hashes — closed on resolution), plurality result, market
  triggers, emissions, dynamic fee state.
- **Off-chain:** model inference, private to each runner. No raw AI output on-chain — only
  the categorical claim and verifiable metadata.

### Trust model

Economic + market-based. KASS slashing for bad facts/claims; MetaDAO decision markets as
the ultimate arbiter over a faulty AI claim.

## 3. Lifecycle / State Machine

1. **Created** — creator posts: prompt, interpretation rules, categorical options, bonding
   params, **deadline**, and pays the dynamic **KASS creation fee** (burned). All config
   is immutable after creation.
2. **Deadline gate** — proposals are rejected before the creation-time deadline.
3. **Proposal window** (opens at deadline) — proposers submit a categorical value + KASS
   bond, **no proofs**.
   - If **≥1 proposal at window end**:
     - *All agree* → **Resolved** (one mode). Proposers split reward; bonds returned;
       emissions minted. No AI, no markets.
     - *Conflict (≥2 distinct values)* → **Dispute**. Proposers are now **locked in**
       (bonds committed, no withdrawal).
   - If **zero proposals at window end** → request stays open and waits for the **first
     proposal**, which validates/seeds the value and opens the normal bounded window.
     (No Unresolved terminal state from emptiness.)
4. **Dispute → Fact Proposal window** (disjoint, no voting) — participants post candidate
   facts (curated evidence), each with a KASS stake. Voting is impossible here — enforces
   the anti-grief rule (no watching approvals then dropping a near-duplicate to split votes).
5. **Dispute → Fact Voting window** (fact set frozen) — participants stake-approve
   individual facts; a vote may also mark a fact **"duplicate."** See §4.
6. **AI-claim resubmission window** — each locked-in proposer reruns the runner over the
   **agreed fact set** and resubmits a value + KASS stake + AI claim metadata. Value may
   change from the original (may collapse the dispute). See §5.
7. **Challenge window (single round)** — every AI claim is challengeable **in parallel**;
   each challenge instantiates a MetaDAO decision market. See §6.
8. **Resolved / Invalid dead-end** — after the last market settles, final plurality is
   computed. See §7.

## 4. Fact Phase

A fact = a piece of **curated evidence** (URL, quoted text, data point). Interpretation
rules are **not** facts — they are fixed at creation.

### Fact Proposal window (disjoint, no voting)

- Anyone posts candidate facts → `Fact` account: content hash + URI/pointer + proposer +
  KASS stake.
- Voting impossible during this window (anti-grief: a whale cannot watch approvals land
  then drop a near-duplicate to split the vote and slash early stakers).

### Fact Voting window (set frozen)

- Per-fact **approval voting** by stake. A vote can be **approve** or **duplicate**.
- **Quorum denominator = the dispute's fixed bond weight** (`dispute_bond_total` = sum of
  proposer bonds, locked when the dispute starts). A fact joins the **agreed set** iff
  `approve_stake > duplicate_stake` AND `approve_stake ≥ threshold × dispute_bond_total`
  (threshold a protocol-global config; default supermajority 2/3). The denominator is the
  bond weight rather than `total_oracle_stake` because the latter grows with every escrowed
  fact/vote stake — under non-exclusive voting that would dilute every fact below threshold
  and hand a risk-free griefing vector to large `duplicate` votes. The fixed bond weight is
  stable, vote-independent, and means "approval comparable to the disputed stake."
- **Non-exclusive approval:** a staker's stake is not split — one staker may approve all
  facts, and full stake counts toward each approved fact.
- **Open participation:** any external KASS holder may stake to weigh in (deepens the
  honest pool; harder for a biased bloc to dominate).
- **Duplicate handling:** facts whose **duplicate** votes dominate are **ignored**; their
  stakers are **not slashed** (no penalty for honest redundancy).
- **Settlement:** stakers on approved facts earn rewards (bond pool + emissions). A
  **rejected** (non-duplicate) fact's **submitter forfeits 100% of their fact-submission
  stake** to the bond pool — the penalty that enforces "smallest set necessary" (staking on
  marginal facts is costly). Approve-voter stake settlement on rejected facts is a separate
  deferred concern. (Slash fractions are governance-tunable; v1 uses full submitter slash.)
  `finalize_facts` records the slash in the `bond_pool` *counter*; actual KASS stays
  escrowed until per-staker claims (a later task). Finalization is **incremental** — facts
  are settled in batches (tracked by `settled_count`); the phase advances to AI-claim only
  once `settled_count == fact_count`, so an arbitrarily large fact set can never wedge the
  oracle in a single oversized transaction.
- **Empty set is impossible** unless *no proposer submitted any fact*; in that case all
  proposers are slashed and the oracle → **Invalid dead-end** (§7).

### Known attack surface (documented, mitigated not eliminated)

Approval voting is a Schelling / keynesian-beauty-contest game; a coordinated stake bloc
could approve a biased subset. Mitigations: fixed interpretation, open external approver
pool, and the downstream decision market that can override the resulting AI claim.

## 5. AI Resolution Phase

### Actor model

- **Proposers** = the early stakers from the proposal window. They are the **only** ones
  who put values on-chain. **One proposer ↔ one value (mode).**
- **Dispute ⟺ ≥2 distinct proposed values.**
- "Anyone can run the AI" = anyone may run the open-source runner to **verify and then
  challenge**; only proposers propose values.
- **Plurality over proposers** (one proposer = one vote for their value). *Not*
  stake-weighted.

### Runner (off-chain, open-source)

- Input: creation-time prompt + interpretation rules + agreed fact set + options.
- Calls the pinned **deterministic model** with declared parameters (model id,
  temperature/seed, etc.).
- Output: one categorical option + metadata bundle.

### On-chain claim (`AiClaim` account)

- `proposer`, KASS stake, chosen `option`.
- **Declared metadata:** model id, parameters/seed, **hash of (prompt + agreed facts +
  raw response)**.
- **Closed on resolution** (rent reclaimed).
- Purpose: a challenger reproduces by running the *declared* model/params over the agreed
  facts; if the categorical answer differs, they open a decision market. No on-chain
  verification of the raw response — fabrication is caught economically.

## 6. Challenge, Decision Market & Recompute

### Challenge → market instantiation (MetaDAO CPI)

- Markets are **dormant by default** — zero trade, zero cost on uncontested claims.
- A challenger opens a claim's market by posting **USDC**; the proposer's
  **already-locked KASS** is the conditional collateral.
- MetaDAO's conditional-vault splits KASS → **pass-KASS / fail-KASS** and USDC →
  **pass-USDC / fail-USDC**, seeding the pass and fail AMM pools.
- Trading: **sell pass-KASS → pass-USDC** (bet pass) or **buy fail-KASS with fail-USDC**
  (bet fail). Fraud-believers drive **fail price up, pass price down**.

### Slash trigger & recompute

- Settlement compares a **TWAP** of fail vs pass (TWAP resists last-block manipulation;
  **TWAP window configurable per oracle**).
- **Threshold is a protocol-global config.** If `fail > pass + threshold`, the claim is
  **disqualified**: proposer's KASS → **bond pool**; fail-side bettors settle in their
  favor.
- **Single challenge round, no cascade.** All claims are challengeable in parallel within
  one window; each opened market trades for the per-oracle TWAP duration. As each market
  resolves it **incrementally updates oracle state** (current surviving modes). The last
  market's end is the structural upper bound — termination guaranteed, no max-rounds cap.
- After the last market settles → **final plurality recomputation** over surviving
  proposers.

## 7. Terminal States & Edge Cases

- **Resolved (valid):** ≥1 value survives all phases → resolve to surviving plurality.
- **Invalid dead-end:** all values disqualified, OR final plurality tie, OR no proposer
  submitted any fact. Fixable only by **KASS governance** (ideally itself run via MetaDAO
  futarchy).
- **No proposals at window end** → wait for first proposal (no Unresolved state).
- **Single uncontested proposal** → resolves to that value (happy path).
- **Tie in final plurality** → Invalid dead-end (no guessing).
- **Proposer fails to resubmit AI claim** → **fully slashed** (abandoning mid-dispute is
  likely what caused the costly dispute).
- **Proposer flips value after AI claim** → **partially slashed**.
- **Empty agreed fact set** → impossible unless no facts at all → all proposers slashed →
  Invalid dead-end.
- **Challenge market with no counter-trading** → fail never crosses threshold → claim
  survives (uncontested = honest by assumption).
- **Dead-end settlement:** all **bonds and stakes returned**; **creator fee stays burned**.

## 8. Economics / Tokenomics

**KASS** — SPL token for staking, slashing, and decision-market collateral. **No
presale**; fair-launch via participation emissions.

### Demand

Must hold/stake KASS to propose, to stake on facts, and (as proposer) it is your
conditional market collateral.

### Emissions (work-to-earn)

KASS minted to honest participants on resolution:
- correct proposers (survived to the resolved value),
- stakers on approved facts,
- successful challengers (fail-side winners).

**Schedule (default):** fixed total supply with a **decaying emission curve** (e.g.
halving epochs); early honest participation rewarded more, supply asymptotes.

### Dynamic creation fee (KASS, burned)

- Oracle creation fee is **paid in KASS** and **burned**.
- **Fee ∝ EMA of recent oracle creations** → 0 at genesis (free bootstrap, earn KASS via
  emissions without acquiring it first), rises with demand, relaxes as demand falls.
- Burn is the **deflationary counterweight** to emissions. **No USDC floor.**

### Bond pool

Slashed KASS (bad facts, disqualified/abandoning proposers) flows here and funds rewards
to honest actors — adversarial redistribution making dishonesty net-negative.

### Governance

KASS governs protocol config (threshold, fee params, emission params) and is the only
authority that can resolve **Invalid dead-end** oracles — ideally via **MetaDAO futarchy**.

## 9. Invariants (to be fuzzed)

1. **Phase ordering:** no instruction executes out of phase; deadlines gate proposals;
   fact proposal and fact voting windows are strictly disjoint.
2. **Termination:** the single challenge round always terminates (bounded by last market's
   TWAP window); no unbounded recompute cascade.
3. **Conservation of KASS:** for any oracle, `Σ stakes_in = Σ returned + Σ to_bond_pool +
   Σ burned + Σ emitted`. No KASS created or destroyed outside emissions (mint) and fee
   burn.
4. **Stake locking:** locked-in proposer bonds cannot be withdrawn during dispute.
5. **Fee monotonicity:** creation fee moves only as a function of the creation-rate EMA;
   never negative; 0 at genesis.
6. **Quorum correctness:** a fact is agreed iff `approve_stake > duplicate_stake` AND
   `approve_stake ≥ threshold × dispute_bond_total` (the fixed dispute bond weight, NOT the
   vote-inflated `total_oracle_stake`); duplicate-dominant facts are excluded and their
   stakers unslashed.
7. **Plurality correctness:** resolution = plurality over surviving proposers; ties →
   Invalid dead-end.
8. **Slash trigger correctness:** a claim is disqualified iff `fail_TWAP > pass_TWAP +
   threshold`.
9. **Terminal exclusivity:** an oracle ends in exactly one terminal state (Resolved or
   Invalid dead-end); dead-end returns all bonds/stakes; creator fee always stays burned.
10. **Closure:** `AiClaim` metadata accounts are closed (rent reclaimed) on resolution.

## 10. Testing & Verification Strategy

- **Program unit tests — LiteSVM only.** Each instruction's account validation, phase-gate
  enforcement, arithmetic (EMA fee, plurality, slashing).
- **Invariant fuzzing:** fuzz phase transitions against the explicit invariants in §9.
- **MetaDAO CPI integration — LiteSVM.** Download MetaDAO's program binaries and load them
  into LiteSVM: conditional-vault split/redeem, market instantiation, TWAP read, slash
  trigger.
- **Adversarial / economic simulations:** Schelling-bloc fact attacks, thin-liquidity
  market griefing, fee-EMA manipulation, Sybil proposers — verify dishonesty is
  net-negative.
- **Runner tests:** determinism harness (same inputs → same categorical output),
  metadata-hash reproducibility, golden prompts.
- **End-to-end — surfpool + the AI proposal runner** (not devnet): full happy path + full
  dispute path against real MetaDAO programs.

## 11. Open Items / Future Work

- Exact emission curve parameters (halving period, total supply).
- EMA window length and proportionality constant for the creation fee.
- Semantic near-duplicate facts beyond the in-vote "duplicate" flag.
- MetaDAO futarchy wiring for governance / dead-end resolution.
- Reward-split ratios (emissions vs bond pool) across honest roles.

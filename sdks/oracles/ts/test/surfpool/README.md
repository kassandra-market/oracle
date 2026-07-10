# Kassandra surfpool E2E (gated)

End-to-end tests that drive the **real** Kassandra oracle lifecycle against a
**real RPC validator** ([surfpool](https://github.com/txtx/surfpool)), built by
the merged SDK's web3.js v3 instruction builders and sent as real transactions,
with the **off-chain runner in the loop** producing AI claims from a controllable
**mock Anthropic server**. The forked-MetaDAO challenge-market path is pushed as
far as tractable, and the **full futarchy governance loop** (proposal → real TWAP
verdict → Squads execute → Kassandra config change) runs end to end on forked
mainnet — see "Full futarchy governance" below.

This suite is **GATED / opt-in**: the default `pnpm test` (127 tests) stays fast,
offline, and never spawns surfpool. The E2E suite only runs under
`KASSANDRA_E2E=1` (see `sdk/vitest.config.ts`, which excludes
`test/surfpool/**` otherwise) and **skips cleanly** (does not fail) when surfpool
/ the program `.so` / the runner binary are absent.

## Prerequisites

1. **surfpool** on `PATH` (or set `SURFPOOL_BIN`). Tested against surfpool
   `1.0.0` (`~/.local/bin/surfpool`).
2. **The program artifact:** `just build` → `target/deploy/kassandra_program.so`
   (deployed at the fixed program id via the `surfnet_setAccount` cheatcode).
3. **The runner binary** (for the lifecycle / runner-against-mock arms):
   `cargo build -p kassandra-runner` → `target/debug/kassandra-runner`.
4. **Network access.** surfpool 1.0.0 always boots against a datasource
   (mainnet by default), so even the standalone core path needs network at boot.
   The **challenge-market** and **futarchy-governance** arms **fork mainnet**
   (`--network mainnet`) and lazily fetch the deployed MetaDAO programs over RPC —
   so they need network and are **slower** than the local core path. The
   futarchy-governance arm requires the deployed futarchy **v0.6.1**.

## How to run

```sh
cd sdk
pnpm test          # default: 127 tests, offline, no surfpool
KASSANDRA_E2E=1 pnpm test:e2e   # gated E2E: 115 tests (spawns surfpool, needs network for the forks; lifecycle/runner arms skip without the runner binary)
# a single arm:
KASSANDRA_E2E=1 pnpm exec vitest run test/surfpool/challenge-market-e2e.test.ts
KASSANDRA_E2E=1 pnpm exec vitest run test/surfpool/futarchy-governance-e2e.test.ts
KASSANDRA_E2E=1 pnpm exec vitest run test/surfpool/meteora-spot-e2e.test.ts
KASSANDRA_E2E=1 pnpm exec vitest run test/surfpool/futarchy-meteora-treasury-e2e.test.ts
KASSANDRA_E2E=1 pnpm exec vitest run test/surfpool/dao-meteora-treasury-e2e.test.ts
```

The harness (`harness.ts` `SurfpoolHarness`) spawns `surfpool start --no-tui
--block-production-mode transaction --no-deploy [--network mainnet]`, polls
`getHealth`, writes the `.so` at the fixed id, and tears the process down. Each
suite owns a distinct port (smoke 8899, lifecycle 8901, challenge 8920,
futarchy-governance 8921, meteora-spot 8922, futarchy-meteora-treasury 8923,
dao-meteora-treasury 8924) so they never collide.

## Files

| file | what |
| --- | --- |
| `harness.ts` | `SurfpoolHarness` (spawn → wait → deploy → teardown), cheatcode helpers (`setAccount`, `airdrop`, `timeTravel`/`advanceToUnix`), SPL byte fabrication. `fork: "mainnet"` boots a forked simnet (T4). |
| `mock-anthropic.ts` | A local `node:http` Anthropic Messages mock (`POST /v1/messages`) returning the exact shape the runner's `parse_messages_response` consumes; `setOption(N)` / `setRefusal(...)`. |
| `run-runner.ts` | Invoke the real runner binary (`AnthropicProvider` → the mock) and capture the claim metadata. |
| `surfpool-smoke.test.ts` | T1: surfpool up → `.so` deployed → `initProtocol` over RPC → decode Protocol. |
| `runner-mock-anthropic.test.ts` | T2: the real runner against the mock (success + refusal). No surfpool. |
| `lifecycle-e2e.test.ts` | T3: full core lifecycle on a standalone simnet — uncontested resolve + dispute→AI-claim (runner in the loop). |
| `challenge-market-e2e.test.ts` | T4 + CS2: the challenge-market path against **forked-mainnet** MetaDAO programs — opens a challenge, then drives `settle_challenge` end to end through a **real swap-driven v0.4 AMM TWAP** (both arms: disqualify + survive). Runs in `clock` block-production mode so the on-chain execution slot advances for the slot-based AMM crank. |
| `futarchy-governance-e2e.test.ts` | G3: the FULL futarchy governance loop against **forked-mainnet** MetaDAO programs — bootstrap → staged Squads VaultTransaction → proposal → real TWAP verdict → `vault_transaction_execute` → Kassandra `set_config` + `resolve_deadend` applied on-chain. Requires futarchy **v0.6.1** (the deployed program). |
| `meteora-spot-e2e.test.ts` | M2 + F1: the Meteora **DAMM v2 (cp-amm)** spot path against the **forked-mainnet** real program — clones a real public `Config`, drives `initializePool → addLiquidity → swap → createPosition → claimPositionFee → removeLiquidity` over RPC, and decodes the resulting `Pool`/`Position` to VERIFY the M1 zero-copy offsets (`sqrt_price`@456, reserves@680/688, `unlocked_liquidity`@152, `fee_b_pending`@144) against the DEPLOYED binary. F1 adds a NONZERO fee claim + a full liquidity removal. Also decodes a genuine mainnet pool. |
| `futarchy-meteora-treasury-e2e.test.ts` | F2b: the futarchy→Meteora DAO-treasury fee-collection (`collect_meteora_damm_fees`) as a **documented-partial with a live deployed-verification proof**. The full sweep can't be driven on a fork (the `production` handler requires the MetaDAO-controlled `admin` signer `tSTp6B6k…`), so instead it **reaches the admin gate**: real `initialize_dao` (genuine `Dao` + Squads multisig/vault) + fabricated fee-recipient ATAs → the F2a `collectMeteoraDammFees` builder is submitted to the DEPLOYED futarchy with a STAND-IN admin → asserted rejected SPECIFICALLY at `InvalidAdmin` (Anchor custom **6020**, `collect_meteora_damm_fees.rs:119`). Since `#[access_control(validate())]` runs only AFTER `try_accounts` accepts the full 27-account layout (order/count/roles + the typed-account, PDA-seed & address constraints), reaching 6020 PROVES the 27-account wire format is accepted on the deployed binary. Plus a real-mainnet-`Dao` cross-verification of the Squads PDA derivations. NOTE: `collect_meteora_damm_fees` is **MetaDAO's protocol-rake op** (fees → MetaDAO's vault `6awyHMsh…`, gated on MetaDAO's keeper `tSTp6B6k…`), NOT a Kassandra dependency — the DAO collects its OWN Meteora fees admin-free via D1 below. Full live sweep DEFERRED (production admin). |
| `dao-meteora-treasury-e2e.test.ts` | **D1: the FIX — a futarchy DAO collects its OWN Meteora treasury fees admin-free**, driven live. Real `initialize_dao` → the DAO's Squads vault; a cp-amm position whose OWNER is that vault (route (a): `initialize_pool` with `creator == the vault` mints the FUNDED first position's NFT straight to the vault — `creator` is an unchecked non-signer, `token::authority = creator`; verified by decoding the NFT account authority == the vault); A→B swaps accrue a NONZERO token-B (quote) LP fee (decoded > 0 on a payer probe). The cp-amm `claim_position_fee` (owner == the vault, recipients == the DAO's OWN vault-owned ATAs) is staged in a Squads `vault_transaction_create → proposal_create`, then a REAL futarchy proposal is driven to a PASS TWAP verdict so `finalize_proposal` CPI-approves the Squads proposal (threshold 1; the sole Vote member is the Dao PDA), then `vault_transaction_execute` (member = the public permissionless member) `invoke_signed`s the claim as the vault. ASSERTS: the DAO's ATA received the accrued fee (NONZERO delta), the vault position's `fee_b_pending` cleared to 0, and NO MetaDAO admin (`tSTp6B6k…`) or MetaDAO vault (`6awyHMsh…`) appears in ANY account of the claim / staged message / execute remaining-accounts. Governance-authorized, admin-free, DAO-owned. |

## Full futarchy governance (G3)

`futarchy-governance-e2e.test.ts` is the headline loop: it proves that a **real
MetaDAO futarchy proposal**, decided by a **real swap-driven TWAP verdict**,
drives a **real Squads v4 `vault_transaction_execute`** that applies a **real
Kassandra `set_config` + `resolve_deadend`** — end to end on **forked mainnet**,
through the actually-deployed programs (futarchy **v0.6.1** `FUTARELBf…`,
conditional_vault `VLTX1ish…`, Squads v4 `SQDS4ep6…`).

### What the loop proves

1. **Bootstrap (real).** `bootstrapGovernance` runs the real `initialize_dao`
   (which atomically creates the `Dao` + the Squads multisig with
   `create_key==Dao` + vault; the Squads `ProgramConfig.treasury` is fetched LIVE
   from the on-chain account) then the **G1-hardened `set_governance`** handoff.
   Asserts on-chain `governanceSet==1`, `daoAuthority==vault`, `kassDao==dao` —
   i.e. G1's hardened linkage check validated against the REAL Squads vault /
   futarchy DAO (owner==`FUTARCHY_ID` + Dao discriminator; `dao_authority` == the
   vault PDA derived `create_key==Dao → multisig → vault 0`).
2. **Stage (real).** A Kassandra `set_config` (sentinel `total_supply_cap`) **and**
   a `resolve_deadend` are staged as **two inner CPIs in ONE Squads
   `VaultTransaction`** (a hand-encoded compact `TransactionMessage`), with a
   `proposal_create(draft:false → Active)`, signed by MetaDAO's public
   permissionless member (`EP3SoC2…`).
3. **Proposal + markets (real).** `initialize_question` (oracle == the futarchy
   Proposal PDA) + base/quote `initialize_conditional_vault` → `initialize_proposal`
   → `launch_proposal` stands up the embedded conditional pass/fail AMM markets.
4. **Verdict (FULLY REAL, swap-driven TWAP).** A trader splits USDC into
   conditional pass/fail quote tokens, then runs **4 real `conditional_swap`
   Buy-Pass** transactions spaced **>60s apart** (via `surfnet_timeTravel`, the
   oracle's 60s rate-limit) to raise the pass observation, then jumps past
   `enqueue + 86400` and a final swap stamps the oracle beyond the
   ProposalTooYoung / MarketsTooYoung windows. `finalize_proposal` resolves
   **Passed** (and CPIs Squads `proposal_approve`).
5. **Execute (real).** `vault_transaction_execute` (member = permissionless)
   `invoke_signed`s BOTH CPIs as the Squads vault. **Headline assertion:**
   on-chain `Protocol.total_supply_cap` == the sentinel; **second arm:** the
   dead-ended oracle is now `Phase::Resolved` with the governance-chosen
   `resolved_option`.
6. **Live `kass_price` (real Dao).** Reads the futarchy spot TWAP from the REAL
   `Dao` (not a fabricated blob) and asserts it is > 0.

### How to run

```sh
cd sdk
KASSANDRA_E2E=1 pnpm exec vitest run test/surfpool/futarchy-governance-e2e.test.ts
```

Needs **network** (it forks mainnet to load MetaDAO's deployed programs) and the
deployed futarchy program is **v0.6.1** — the SDK builders are pinned to that
on-chain IDL (see `sdk/src/futarchy/NOTES.md`, "G3 ADDENDUM"). Skips cleanly
(does not fail) when surfpool / the `.so` is absent.

### Honesty notes (read before trusting the assertion)

1. **The pass margin is thin (determinism over economic width).** The DAO is
   bootstrapped with `passThresholdBps=0` and a ~1.0 starting TWAP on BOTH legs,
   so the pass margin the swaps need to manufacture is narrow. The verdict is
   **genuinely swap-driven** — a falsification run confirms it: removing
   `vault_transaction_execute` makes the headline assertion FAIL, so the config
   change is not seeded — but the test deliberately optimizes for a deterministic
   pass on a fork rather than a wide economic margin. Treat it as a proof of the
   *mechanism*, not of economic robustness.
2. **Input state is fabricated; the GOVERNED OUTCOMES are real.** The dead-end
   oracle and the token / LP balances are `surfnet_setAccount` fabrications — the
   established T4 input-materialization pattern (owner / size / type-tag only,
   canonical SPL or Kassandra bytes). What flows through the REAL programs are the
   **outcomes**: the `set_config` change, the oracle resolution, and the TWAP
   verdict itself all execute through the real futarchy / Squads / Kassandra
   programs. Do not mistake input-fabrication for a faked result — the inputs are
   fabricated, the outcomes are real.
3. **`kass_price` is read via `simulateTransaction`.** The live `kass_price` value
   is a **read-only price query** (the instruction's return data, fetched through
   `simulateTransaction`), NOT part of the verdict / execution path. It confirms a
   real on-chain DAO's spot TWAP is readable; it does not gate the proposal.

## Covered vs deferred

### Covered (proven, real over RPC)

- **FULL futarchy governance loop, on forked mainnet (G3).** The real
  proposal → swap-driven TWAP verdict → Squads `vault_transaction_execute` →
  Kassandra `set_config` + `resolve_deadend` applied on-chain, end to end — see
  "Full futarchy governance" above (incl. the three honesty notes: thin pass
  margin, fabricated-inputs-vs-real-outcomes, `kass_price`-via-simulate).
- **Live `kass_price` from the REAL futarchy Dao (G3).** Read via
  `simulateTransaction` return data from the genuine on-chain `Dao` (no fabricated
  `Dao` blob) — a read-only query, not the verdict path.
- **The G1-hardened `set_governance` handoff, validated live (G3).** The
  on-chain linkage check (`kass_dao` owned by `FUTARCHY_ID` + Dao discriminator;
  `dao_authority` == the derived Squads vault) is exercised against the REAL
  Squads vault / futarchy DAO produced by `bootstrapGovernance`.

- **Core lifecycle, fully real (T3).** On a standalone simnet, every phase is
  driven by REAL Kassandra instructions over RPC — no `setAccount` seeding of any
  Kassandra program account or phase. Two arms:
  - **Uncontested resolve:** `initProtocol → createOracle → propose×3 (same
    option) → finalizeProposals` ⇒ Oracle `Resolved` + the agreed option (decoded
    over RPC); the stake vault holds Σ bonds.
  - **Dispute → AI-claim (runner in the loop):** `create → propose×2 conflicting
    → finalizeProposals → submitFact → advancePhase → voteFact → finalizeFacts →`
    **run the real runner** (genuine `AnthropicProvider` → the mock server,
    `setOption(N)`) `→ submitAiClaimFromRunner → finalizeAiClaims →
    finalizeOracle` ⇒ Oracle `Resolved` with the AI's option, and the on-chain
    `AiClaim` decodes to the runner's exact model_id/params_hash/io_hash/option.
  - The only fabricated state is SPL plumbing (mints + funded KASS token
    accounts), packed as canonical SPL bytes; the program's own SPL CPIs run
    against the real Token program. Phase windows are crossed with
    `surfnet_timeTravel` (it moves the Clock `unix_timestamp` at ~0.4 s/slot, the
    value the program's `now()` reads).
- **Runner real-provider path (T2).** The real `AnthropicProvider` HTTP + parse
  path is exercised against controllable mock responses (success + refusal).
- **Challenge-market on FORKED MetaDAO (T4).**
  - **Programs load.** All five MetaDAO program ids (conditional-vault `VLTX1ish…`,
    AMM v0.4 `AMMyu265…`, futarchy v0.6 `FUTARELBf…`, Meteora DAMM v2, Squads v4)
    are fetched from the mainnet fork as `executable` BPF-upgradeable programs.
  - **Conditional-vault EXECUTES.** A real `initialize_question` CPI against the
    forked vault creates the on-chain `Question` (decoded `oracle`/`num_outcomes`
    match) — far past "program not found".
  - **A challenge is OPENED.** The full dispute core is driven to `Challenge`,
    the MetaDAO market is COMPOSED over RPC (real `initialize_question` +
    KASS/USDC `initialize_conditional_vault` CPIs), and the Kassandra
    `openChallenge` instruction is sent. Its **program-signed `split_tokens`
    CPI runs against the forked conditional-vault**, physically splitting the
    proposer's KASS bond into pass/fail conditional KASS (each == bond, underlying
    in the vault). Asserted: `Market` PDA created + bound, `ai_claim.challenged`
    flipped, USDC escrow funded with the on-chain-computed amount,
    `open_challenge_count == 1`.
  - **`settle_challenge` END-TO-END, both arms, REAL swap-driven v0.4 AMM TWAP
    (CS2).** After opening, the test builds the **real** pass/fail v0.4 AMM pools
    on the fork (`ammV04.createAmm` + `addLiquidity` on this market's conditional
    KASS/USDC mint pairs), then drives a **genuine TWAP** (no seeded/forced
    aggregator) and settles:
    - **DISQUALIFY (challenge succeeds):** the PASS pool is left neutral; a real
      `ammV04.swap` BUY pushes the FAIL pool's price up, and two
      `ammV04.crankThatTwap` cranks ≥150 slots apart fold the post-swap price
      into the slot-weighted TWAP — decoded over RPC, `fail_twap (≈2.4e9) × DEN >
      pass_twap (1.0e9) × (DEN+NUM)`, clearing the 10% margin. `settleChallenge`
      then resolves the question FAIL-side `[0,1]`, carves `kass_fee = bond/100`
      to the challenger, redeems `bond − kass_fee` into `stake_vault`, returns the
      full USDC escrow to the challenger, and records the slash
      (`slashed_amount == bond − kass_fee`, `bond_pool += that`, `surviving_count
      − 1`) — all asserted from on-chain accounts.
    - **SURVIVE (challenge fails):** both pools are cranked neutral (both TWAPs
      real + non-zero, the margin holds). `settleChallenge` resolves PASS-side
      `[1,0]`, redeems the full bond back into `stake_vault` (un-slashed,
      `bond_pool` unchanged), routes `usdc_fee = escrow/100` to the proposer and
      the remainder to the challenger.

    The slot-based v0.4 AMM crank needs the on-chain **execution** slot to advance
    (`surfnet_timeTravel` moves only `getSlot`/`unix_timestamp`, not the slot the
    program reads during execution — unlike G3's *timestamp*-based futarchy
    oracle), so this suite boots surfpool in **`clock` block-production mode**
    (fast slot-time) and waits real slots between cranks.

    *Honesty note (fabricated inputs, real outcomes).* The TWAP, the swap, the
    crank, the resolution, the redeem, and every directional-fee transfer run
    through the **real** deployed AMM / conditional-vault + the real Kassandra
    `settle_challenge`. What is fabricated is SPL plumbing only: the pools'
    conditional-token liquidity (canonical SPL balances at the payer's ATAs) and
    the escrow-price `Dao` blob the challenge arm sizes its USDC escrow from
    (same `surfnet_setAccount` input-materialization pattern as T4/G3). The
    swap-driven TWAP → disqualify/survive decision and the settled economics are
    not seeded.

- **Meteora DAMM v2 spot path on FORKED mainnet (M2), offsets verified vs the
  DEPLOYED binary.** `meteora-spot-e2e.test.ts` boots surfpool forking mainnet so
  the REAL cp-amm `cpamdpZC…` executes over RPC, clones a REAL public + static
  mainnet `Config` (index 0, `8CNy9goNQNLM4wtgRw528tUQGMKD3vSuFRZY2gLGLLvF`,
  `pool_creator_authority == default`), fabricates two SPL mints + funded payer
  token accounts, then DRIVES the M1 builders through the real program
  (`skipPreflight:false`, confirm-throws): `initializePool` (creates the pool +
  first Token-2022-NFT position, funded `liquidity`+`sqrt_price`) → `addLiquidity`
  → `swap` (A→B) → `createPosition` (a second, empty position).

  **Offset verification (the point).** `decodePool`/`decodePosition` read the
  freshly-driven on-chain accounts and assert: `sqrt_price` (abs 456) ==
  `SQRT_PRICE_INIT`; `liquidity` (360), `sqrt_min/max` (424/440), mints (168/200),
  vaults (232/264), `token_a_amount`/`token_b_amount` (680/688) all == the driven
  values (reserves computed with cp-amm's exact deposit math AND matching the live
  vault balances); `unlocked_liquidity` (152) == the deposited liquidity, rising by
  the `addLiquidity` delta; after the A→B swap `sqrt_price` MOVED DOWN, the token-A
  reserve rose by exactly `amount_in`, and token-B fell (each vault holds
  reserve + accrued LP fee). Independently, a genuine mainnet pool is fetched from
  mainnet and `decodePool`d, asserting `(sqrt_price/2^64)² ≈ reserve_b/reserve_a`
  within 1%. If any computed offset were wrong these reads would be garbage — so
  passing proves the offsets against the deployed layout. Runs in ~1–2s once
  surfpool is up (transaction block mode; no slot crank needed — cp-amm price is
  instantaneous). Runtime deps: network (fork + a direct mainnet `getAccountInfo`).

  **F1 — `claimPositionFee` + `removeLiquidity` driven LIVE (all 6 builders now
  covered).** After the swap, the same arm drives the last two cp-amm builders
  through the real program (`skipPreflight:false`, confirm-throws):
  - **`claimPositionFee` (NONZERO, real transfer).** A couple more A→B swaps grow
    the accrued LP trading fee, then a tiny `addLiquidity` CHECKPOINTS it onto the
    position (cp-amm updates position fees lazily). On this cloned public Config
    the `collect_fee_mode` collects fees in **token B** for both swap directions,
    so `fee_b_pending` is nonzero (empirically ~8.5e5 raw; `fee_a_pending` stays
    0). The claim is asserted to transfer EXACTLY `fee_b_pending` to the owner's
    token-B account (`> 0`) and to clear the position's pending fees — a genuine,
    nonzero fee sweep, not a no-op.
  - **`removeLiquidity` (full withdrawal).** Removes ALL `unlocked_liquidity` from
    the first position and asserts: the position's `unlocked_liquidity` → 0, the
    pool `liquidity` dropped by exactly the removed delta, both tracked reserves
    (`token_a_amount`/`token_b_amount`) fell, and the owner's token accounts rose
    by exactly the reserve deltas (the withdrawn amounts, `> 0` on both sides).

- **DAO-OWNED, ADMIN-FREE Meteora treasury-fee claim on FORKED mainnet (D1) —
  the FIX for the F2a/F2b MetaDAO-admin dependency.** `dao-meteora-treasury-e2e.test.ts`
  proves a futarchy DAO collects its OWN Meteora cp-amm LP fees WITHOUT any
  MetaDAO admin, governance-authorized, end to end over RPC (real futarchy
  v0.6.1 + Squads v4 + cp-amm):
  - **The position is genuinely DAO-owned.** `initialize_pool` is called with
    `creator == the DAO's Squads vault`, so cp-amm mints the FUNDED first
    position's NFT straight to the vault (cp-amm `creator` is an unchecked
    non-signer with `token::authority = creator`; the payer funds the liquidity
    but the NFT authority is the vault) — verified by decoding the position NFT
    account's authority (owner @32) == the Squads vault. No NFT transfer needed.
  - **The fee is real + nonzero.** A→B swaps accrue a token-B (quote) LP fee; a
    payer-owned probe position is checkpointed to DECODE `fee_b_pending > 0`
    (proof the pool accrues real quote-side fees; the vault position, with larger
    liquidity, accrues more).
  - **The claim is authorized by the DAO's own governance (not an admin, not a
    plain keypair).** The cp-amm `claim_position_fee` (owner == the vault,
    recipients == the DAO's OWN vault-owned ATAs) is compiled into a Squads
    compact `TransactionMessage` and staged via `vault_transaction_create` +
    `proposal_create`. A REAL futarchy proposal is then driven to a PASS TWAP
    verdict (G3's swap-driven machinery), whose `finalize_proposal` CPI-approves
    the Squads proposal (threshold 1; the sole Vote member is the Dao PDA — so a
    passing proposal is the ONLY way the vault acts). Finally
    `vault_transaction_execute` (member = the public permissionless member)
    `invoke_signed`s the Meteora claim AS THE VAULT.
  - **Asserted:** the DAO's OWN ATA balance rose by a NONZERO fee; the vault
    position's `fee_b_pending`/`fee_a_pending` cleared to 0 (a genuine, non-no-op
    sweep); and NO MetaDAO admin (`tSTp6B6k…`) or MetaDAO vault (`6awyHMsh…`)
    appears in ANY account of the inner claim, the staged Squads message, or the
    `vault_transaction_execute` remaining-accounts (asserted absent). This is the
    correct/supported treasury path: DAO-owned, governance-authorized, admin-free.
    Runs in ~14s once surfpool is up. *(Honesty note: as in G3, the TWAP pass
    margin is thin — deterministic-pass-on-a-fork, not economic width — and the
    SPL token balances / LP liquidity are `surfnet_setAccount` fabrications; the
    OWNERSHIP, the governance approval, the vault-signed CPI, and the swept fee
    are all REAL through the deployed programs.)*

### Deferred (NOT asserted — documented honestly)

- **Meteora dynamic-fee / reward-emission mechanics** beyond the spot lifecycle.
- **Dead-end ECONOMIC settlement.** G3 proves `resolve_deadend` is
  governance-driven and STAMPS the outcome (`Phase::Resolved` + `resolved_option`);
  the token movement / payout for a governance-resolved dead-end belongs to the
  settlement milestone and is NOT exercised here.
- **Program-driven DAO creation.** The bootstrap is off-chain by decision
  (`bootstrapGovernance` calls the real `initialize_dao` + `set_governance`); the
  on-chain `initialize_dao` Borsh stub stays unused.
- **Live-cluster / mainnet deployment with real funds.** No devnet/mainnet
  submission of the real KASS DAO with real funds; no real (non-mock) Anthropic
  call (the runner's live test already exists, `#[ignore]`).
- **Making the suite part of the default `pnpm test`** — it is intentionally
  gated (heavier + network for the forks).

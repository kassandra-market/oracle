# Bootstrapping via an activity-scaled stake floor

**Date:** 2026-07-08
**Status:** Design (validated), ready for implementation planning

## Problem

The oracle is meant to launch with **no premined KASS** — the KASS mint's
authority is a program PDA (`[b"mint_authority"]`), and at genesis the supply is
0. But every way to *use* the oracle currently requires holding KASS:

- `create_oracle` burns a creation fee (0 at genesis, since `fee_ema == 0`) and
  mints an *emission* — but into the oracle's **stake vault** (the reward pool for
  winners), never to a participant, and it is disabled at genesis anyway
  (`total_supply_cap == 0`).
- `propose`, `submit_fact`, and `vote_fact` each transfer a KASS **bond/stake**
  from the caller's own account (and reject a zero stake with `ZeroStake`).

So the first oracles are unplayable: nobody holds KASS to stake, and the only KASS
(emission) lands in reward pools you can only claim by having already participated.
Chicken-and-egg.

## Goal

Let **anyone create and participate in the first oracles with no pre-existing
KASS**, without minting any bootstrap tokens — and restore Sybil resistance
smoothly as the network matures and KASS circulates.

## Approach: scale the *cost* from zero, not mint tokens

Rather than mint bootstrap KASS (faucet / protocol-funded bonds / emission to the
creator — all considered and rejected), make the **minimum stake itself start at 0
and grow with network activity**. At genesis, activity ≈ 0, so the floor is 0 and
participation is free. As oracle-creation activity rises (and emission-funded
rewards have distributed KASS), the floor ramps up, re-introducing skin-in-the-game.

### Decisions (from design dialogue)

1. **Mechanism:** an activity-scaled *minimum* stake, not minted bootstrap tokens.
2. **Scope:** applies to **all three stakes** — proposer bond, fact-submission
   stake, fact-vote stake. (The creation fee keeps its existing fee-EMA formula,
   unchanged.)
3. **Floor, not fixed:** the scaled value is a **minimum floor**; a participant may
   still stake *more* for a larger reward share, preserving the current
   stake-proportional reward model.
4. **Activity metric:** the **existing fee-EMA** (`Protocol.fee_ema`) — a
   fixed-point, 1-day-half-life decaying accumulator of creation activity. No new
   state; smooth; no midnight-reset edge.

## The curve

A pure, piecewise-linear function of the (decayed) fee-EMA `e`:

```
stake_floor(e, threshold, cap, max):
    if max == 0 or cap <= threshold:  return 0        # disabled / degenerate
    if e <= threshold:                return 0        # free (bootstrap)
    if e >= cap:                      return max      # capped
    return max * (e - threshold) / (cap - threshold)  # linear ramp (u128 math)
```

### Calibration (fee-EMA ↔ oracles/day)

The fee-EMA is bumped by `FEE_EMA_INCREMENT = 1e9` per creation and decays with a
1-day half-life. At a steady rate of `n` oracles/day the EMA settles at

```
E(n) = 1e9 / (1 − 2^(−1/n))
```

| rate (oracles/day) | steady-state fee-EMA |
|---|---|
| 1   | 2.00e9  |
| 10  | 1.49e10 |
| 100 | 1.45e11 |
| 1000| 1.44e12 |

**Default params** (governance-tunable via `set_config`):
- `stake_floor_ema_threshold` = **1.49e10** (≈ **10 oracles/day**) — free below this.
- `stake_floor_ema_cap` = **1.44e12** (≈ **1000 oracles/day**) — floor maxes here.
- `stake_floor_max` = governance-set to the token's value (a placeholder default;
  see "Open item" below).

Genesis is free automatically: `fee_ema` starts at 0 ≤ threshold → floor 0.

## Where it lives

**`Protocol`** (governable, config-as-state) — three new `u64` fields **appended**
after the existing monetary knobs, set in `set_config`, defaulted in
`init_protocol` from new `config.rs` consts:
`stake_floor_ema_threshold`, `stake_floor_ema_cap`, `stake_floor_max`.

**`Oracle`** — one new `u64` field, `min_stake`, **appended after
`reward_emission`** so every existing field offset is unchanged (the market
indexer's oracle offsets 160/161/197 and its length check keep working; only the
tail grows by 8 bytes). Snapshotted at `create_oracle`.

**`create_oracle`** already computes `decayed_fee_ema` for the fee. Reuse that exact
value:

```
oracle.min_stake = stake_floor(decayed_fee_ema, protocol.stake_floor_ema_threshold,
                               protocol.stake_floor_ema_cap, protocol.stake_floor_max)
```

Snapshotting freezes each oracle's floor at creation — stable for its whole life,
predictable for participants, and consistent with how every other governable param
is frozen onto the oracle (Task F2).

## Enforcement

`propose`, `submit_fact`, `vote_fact` all already carry the oracle account, so no
new accounts are needed. Replace each `stake == 0 → ZeroStake` guard with:

```
if stake < oracle.min_stake:  return Err(BelowMinStake)   # new error = 36
```

When `min_stake == 0` (genesis) this permits `stake == 0` — free, weightless
participation. When `min_stake > 0`, the stake must clear the floor.

## Why zero stakes are safe (no resolution breakage)

- **Outcome selection** is plurality by **proposer count** (`counts[opt] += 1`),
  not bond weight, so a zero-bond oracle still resolves to a clear winner — never a
  forced tie/dead-end.
- **Reward distribution** already has explicit **zero-stake fallback branches**
  (`reward_buckets`: empty-cohort roll-in when `total_correct_proposer_stake == 0`
  or `total_approved_fact_stake == 0`), so a zero-stake cohort degrades gracefully.
- **Slashing** with zero stakes slashes zero — a no-op.
- At pure genesis (stakes 0, emission disabled) an oracle runs "free, weightless,
  no-reward," resolving on facts/AI/challenge signals. As activity + emission turn
  on together, it ramps to "staked, weighted, rewarded." The degradation is
  monotone and continuous.

The accepted trade-off: at genesis, weightless participation is Sybil-cheap. That
is *intended* — the floor rising with activity is exactly what re-prices Sybil
attacks once the network (and token) is worth attacking.

## Cross-stack impact

Appending fields changes the on-chain `LEN` of `Oracle` (+8) and `Protocol` (+24),
so the decoders and length checks must follow:

- **program:** `state.rs` (2 structs), a new `stake_floor` module (pure + tested),
  `create_oracle` (snapshot), `propose`/`submit_fact`/`vote_fact` (floor guard),
  `set_config` (+3 params, payload + validation `threshold ≤ cap`),
  `init_protocol` (+3 defaults), `config.rs` (+3 consts), `error.rs`
  (`BelowMinStake = 36`).
- **sdk-rs / sdk-rs-market:** Oracle/Protocol layouts + `set_config` builder.
- **TS SDK (`sdk/`):** Oracle/Protocol decoders + the **oracle length check**
  (currently 360 → 368) + `setConfig`/`createOracle` builders.
- **indexer:** oracle-account decode offsets are all before the appended field, so
  unaffected; only bump any hard-coded expected length.
- **tests:** LiteSVM/surfpool oracle harness fabricated-account byte builders
  (append the new fields, zero-filled = disabled = current behavior).

## Testing

- **Unit (`stake_floor`):** below/at threshold → 0; at/above cap → max; linear
  midpoint; `max == 0` and `cap ≤ threshold` → 0; no overflow at `u64::MAX` inputs.
- **Unit (calibration):** `E(n)` table values map to the documented rates.
- **Program (LiteSVM):**
  - Genesis (`fee_ema == 0`): `create_oracle` snapshots `min_stake == 0`; `propose`
    / `submit_fact` / `vote_fact` with `stake == 0` **succeed**; the oracle resolves
    by plurality with zero bonds.
  - High activity (fee-EMA driven above cap): `min_stake == stake_floor_max`; a
    below-floor stake is rejected `BelowMinStake`; an at/above-floor stake succeeds.
  - Frozen snapshot: retuning `set_config` after creation does **not** change an
    existing oracle's `min_stake`.
  - Reward parity: a fully-floored oracle distributes rewards identically to today
    (floor acts only as a minimum).
- **e2e/SDK:** the surfpool seed no longer needs to pre-fund KASS to create + drive
  a genesis oracle end-to-end.

## Open item for governance

`stake_floor_max` is an economic magnitude that depends on KASS's value, which
doesn't exist yet. Ship a conservative documented default and leave it for
governance (`set_config`) to tune once the token has a market — exactly like
`total_supply_cap` / `emission_num`, which genesis also leaves for governance.

## YAGNI / non-goals

- No new faucet, no protocol-funded bonds, no emission-to-creator — the floor alone
  meets the goal.
- No change to the emission mechanism, the creation-fee formula, plurality, or the
  reward math (only new zero-stake paths, which already existed).
- No per-day counter / new activity accumulator — reuse the fee-EMA.

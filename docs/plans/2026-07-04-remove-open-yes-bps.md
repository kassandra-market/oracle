# Remove `open_yes_bps` (abandon uneven opening prior)

✅ **DONE** — addressed by `9371a7e` (`chore: remove open_yes_bps — abandon uneven opening prior`). All 3 steps green (89 Rust + 55 SDK + 69 app); Market layout stable (LEN 400, `open_yes_bps@152` → `_reserved_152`); `InvalidSplit` kept (unused). Pushed to origin/master.


Abandon the uneven opening prior. `open_yes_bps` is recorded-but-unused (pools always seed 50/50), and the "Opening YES %" create-form input is misleading. Remove the feature from every surface — payload, validation, SDK arg, app input, exposed field.

**Layout decision:** `Market.open_yes_bps: u16` sits mid-struct at offset **152** (before `status@154`). Rather than delete it and shift ~15 downstream offsets (risky), convert it to **reserved padding** (`_reserved_152: [u8; 2]` @152) — offsets 154+ stay put, Market LEN stays 400. The field is gone from the API/wire/UX; 2 harmless reserved bytes remain (standard).

The create_market payload shrinks: `open_yes_bps[2] ++ seed_amount[8] ++ outcome_index[1]` (11) → `seed_amount[8] ++ outcome_index[1]` (9). `InvalidSplit` error becomes unused (keep the variant, no renumber). Pool seeding is unchanged (already 50/50).

## Step 1: Program + sdk-rs (wire format — must move together)
- `state.rs`: rename `Market.open_yes_bps: u16` @152 → `_reserved_152: [u8; 2]` (offset/LEN unchanged). Update the field doc/struct comment.
- `create_market.rs`: PAYLOAD_LEN 11→9; parse `seed_amount = payload[0..8]`, `outcome_index = payload[8]` (no open_yes_bps); drop the `1..=9999 → InvalidSplit` check; drop `market.open_yes_bps = ...`. Update the payload doc comment.
- `activate.rs`: drop the "open_yes_bps is recorded-but-unused" comment (now moot).
- `sdk-rs/src/ix.rs`: `create_market` builder drops the `open_yes_bps` arg + payload byte (payload = seed_amount ++ outcome_index).
- Tests/harness: `tests/common/mod.rs` `create_market`/`create_market_full` helpers drop the open_yes_bps arg (thread through all callers); `tests/create_market.rs` drop the `InvalidSplit`/open_yes_bps assertions (and any "opening split" test); `state_layout.rs` rename the `open_yes_bps@152` assertion to `_reserved_152@152` (offset unchanged). 
- Verify: `just build && just test` green; `cargo clippy` clean.

## Step 2: TS SDK
- `instructions/market.ts`: `createMarket` drops `openYesBps` (payload = seedAmount ++ outcomeIndex, 9 bytes).
- `accounts/market.ts`: `decodeMarket` drops the `openYesBps` field (don't read/expose the reserved bytes). Update the `Market` type.
- `flows/createAll.ts`: drop `openYesBps` from `createAllOutcomeMarkets`.
- `constants.ts` / docs: drop any openYesBps references; `MarketError.InvalidSplit` stays (unused).
- Tests: `builders.test.ts`, `flows.test.ts`, `categorical.e2e.test.ts`, `lifecycle*.e2e.test.ts`, `surfpool/lifecycle-e2e.test.ts`, the litesvm/surfpool harnesses — drop openYesBps from every createMarket call + any decode assertion. parity unaffected (Market LEN 400).
- Verify: `pnpm --filter @kassandra-market/sdk test` + typecheck green (rebuild SDK dist for the app).

## Step 3: App
- `CreateMarketForm.tsx`: remove the "Opening YES %" input from BOTH single-outcome and batch ("all N") modes; default the seeding narration to 50/50 (or just drop the mention). 
- `data/actions/{create,createAll}.ts`: drop `openYesBps` from `buildCreateMarketIxs`/`buildCreateAllSteps` → the SDK builders.
- `components/markets/MarketCard.tsx`, `pages/MarketDetail.tsx`: remove any "opening YES %" display.
- `app/e2e/global-setup.ts`: drop openYesBps from seeding createMarket calls.
- Unit tests (`app/test/{actions,actions-active}.test.ts`): drop openYesBps from create assertions.
- Verify: `pnpm --filter @kassandra-market/app typecheck && lint && build && test` green.

## Commit
One commit at the end (established workflow): `chore: remove open_yes_bps — abandon uneven opening prior`, then push. (Each step must independently build+test green before moving on, per the removal discipline.)

## Not touched
- 50/50 pool seeding (unchanged — it was always the behavior). `InvalidSplit` variant kept (no renumber).

# litesvm program + account fixtures (D2)

These are the DEPLOYED mainnet programs + accounts that
`sdks/oracles/ts/test/meteora-collect-litesvm.test.ts` (D2) loads into litesvm to full-drive
MetaDAO's `collect_meteora_damm_fees` past the admin gate.

## Programs (committed `.so`)

Dumped from mainnet-beta with the Solana CLI:

```sh
solana program dump -u m FUTARELBfJfQ8RDGhg1wdhddq1odMAJUePHFuBYfUxKq futarchy.so   # ~1.24 MB
solana program dump -u m cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG cp_amm.so     # ~2.17 MB
solana program dump -u m SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf squads_v4.so  # ~1.47 MB
```

(SPL Token, Token-2022 and the ATA program come from litesvm's default builtins.)

### Pinned checksums (`sha256`)

The exact bytes committed here, so a future re-dump of an *upgraded* on-chain
program can't silently change the D2 proof. Verify with
`shasum -a 256 -c` against this list (or `shasum -a 256 *.so`):

```
753daac67ed0393dc6b3678420ead88814205780eae13cacb5dbafdb179bf8d6  futarchy.so
16eeb0c2f116a0b43849f8de2422c915fea2e35d47fbe3bf705cb95570f1ebfb  cp_amm.so
dec8d3e0fae58c7c8f2416e5f67c25e673f047afd6dd2bba4a47e0b29a01d34c  squads_v4.so
```

A mismatch means the deployed program was upgraded since these were dumped —
re-verify the D2 wire format against the new binary before updating them.

## Account fixtures (committed JSON: `{ pubkey, dataB64, owner, lamports, space }`)

- `squads-program-config.json` — Squads v4 `ProgramConfig` PDA
  (`BSTq9w3kZwNwpBXJEvTZz2G9ZTNyKBvoSeXMvwb4cNZr`). Read by `initialize_dao`
  (its `treasury` field @ offset 48 receives the multisig creation fee — 0 here).
- `cp-amm-config.json` — a REAL public/static cp-amm `Config`
  (`8CNy9goNQNLM4wtgRw528tUQGMKD3vSuFRZY2gLGLLvF`, `pool_creator_authority ==
  default`) so an arbitrary payer can create the pool.

Re-dump:

```sh
solana account -u m <pubkey> --output json   # then map account.data[0]->dataB64, account.owner->owner, etc.
```

## Why gated (`KASSANDRA_LITESVM_PROGRAMS=1`)

The `.so`s total ~4.9 MB. They are committed (so the D2 proof is reproducible
offline), but the test is gated behind `KASSANDRA_LITESVM_PROGRAMS=1` so the
default `pnpm test` stays fast (it skips loading ~4.9 MB of BPF each run). Run it
with:

```sh
KASSANDRA_LITESVM_PROGRAMS=1 pnpm exec vitest run test/meteora-collect-litesvm.test.ts
```

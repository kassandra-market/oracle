---
name: wire-format-single-source-of-truth
description: "Use when a SECOND consumer needs to build the same on-chain instruction, protocol message, or binary/wire payload (e.g. an integration test hand-rolls an instruction and a keeper/bot, frontend, or other service also sends it). Symptoms — writing account metas + a discriminant byte + amount.to_le_bytes() in one place then again in another; typing `const IX_DEPOSIT = 1 // must match the program`; planning that the frontend will build the same instruction later; two files hand-encoding the same struct, packet, or protobuf."
---

# Wire-format single source of truth

## Overview

When two or more consumers must agree on a wire format — instruction account order, discriminants, payload byte layout, protocol message fields — **define it once in a client library derived from the schema, and route every consumer through that library.** Never hand-roll the same encoding in more than one place, and never hardcode a copy of a value the schema owns (a discriminant, a PDA seed, a field offset).

The failure this prevents: the encoding gets copy-pasted across a test, a keeper, and a frontend; a schema-side change (a renumbered discriminant, a reordered account) silently desyncs the copies, and the bug surfaces only as a failed transaction at runtime — not at compile time.

## When to use

Trigger the moment a **second** consumer needs the same encoding. Symptoms:

- You're about to hand-write `AccountMeta`/account order + a discriminant byte + `to_le_bytes()` in a test, and a keeper/bot/app also sends that instruction.
- You typed `const IX_DEPOSIT: u8 = 1; // must match the program`.
- "The React app will build the exact same instruction later."
- Two files construct the same protobuf/struct/packet/header by hand.

Skip only for a genuinely single-consumer, throwaway encoding (but those rarely stay single-consumer — re-apply the moment a second appears).

## The pattern

**Wrong — each consumer re-encodes, and the copies drift:**
```
tests/common.rs:  deposit_ix(){ AccountMeta::new(vault,false); data.push(1); ... }  // copy 1
keeper/main.rs:   const IX_DEPOSIT = 1;  AccountMeta::new(vault,false); ...          // copy 2 (drifts)
app/deposit.ts:   [{pubkey: vault, ...}], Buffer.from([1, ...amount])               // copy 3 (drifts)
```

**Right — one client library, derived from the schema; everyone imports it:**
```
client-sdk/   (depends on the program/schema for the discriminant enum + layouts)
    fn deposit(vault, depositor, vault_token, amount) -> Instruction   // ONE definition
tests  -> client_sdk::deposit(...)
keeper -> client_sdk::deposit(...)
app    -> its language's SDK, kept in parity by a byte-equality test
```

Rules:

1. **Derive, don't redeclare.** The client library depends on the schema/program for discriminants, seeds, and layouts. A schema change then **breaks the build**, not production.
2. **Tests are consumers too.** A test that hand-rolls the instruction is just another drift source — put the builder in the SDK and have the harness delegate to it.
3. **Cross-language:** a TS app can't share the Rust code. Mirror the encoding in exactly one place per language and add a parity test asserting identical bytes.
4. **Caller footprint shrinks to the call:** no account metas, no discriminants, no `to_le_bytes()` at the call site.

## Common mistakes

- **Hardcoding the discriminant** (`const IX = 1`) instead of importing the schema's enum — the #1 drift source; a comment "must match the program" is an admission it can drift.
- **Exempting tests** — "it's just a test helper." Centralize it; delegate from the harness.
- **Waiting for the third copy** — centralize when the *second* consumer appears, not after the runtime bug.

## Real-world impact

Routing an integration-test harness plus an off-chain runner through one Rust SDK (deriving discriminants + account layouts from the program) collapsed a 2,300-line hand-rolled harness's builders into thin delegations and let the runner drop its direct dependency on the program crate entirely — one definition per instruction, every drift caught at compile time.

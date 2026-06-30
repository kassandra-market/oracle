# Kassandra prompt assembly — the off-chain protocol contract

This document is **the spec** for how the runner assembles the model input
(`system` / `user`) from an oracle's inputs. Because `io_hash` (see
`runner/HASHING.md`) commits to the **exact** `system` / `user` bytes, a
challenger re-running the oracle MUST assemble byte-identical strings or their
`io_hash` will not match. The Rust reference implementation is
`runner/src/prompt.rs`.

This is **prompt-assembly version 1**, pinned by `PROMPT_ASSEMBLY_VERSION` (in
`runner/src/hashing.rs`), which is folded into `params_hash`. **Any change to the
assembled bytes — preamble, headers, separators, fact rendering, option
enumeration, or the answer instruction — MUST bump `PROMPT_ASSEMBLY_VERSION`**,
so claims produced by different assembly versions never collide. The regression
anchor test (`assembly_regression_anchor`) pins the exact output of a fixed
input.

## Determinism rules

- **Fact ordering is canonical**: facts are sorted by their 32-byte
  `content_hash` **ascending (lexicographic)**, independent of input order.
- **Fixed separators**: blocks are joined by exactly `"\n\n"`; option lines by
  exactly `"\n"`. There is **no trailing whitespace and no trailing newline**.
- **Verbatim content**: verified fact content + the oracle interpretation are
  rendered **verbatim** (no trimming/normalization) — the content is what
  `content_hash` / `prompt_hash` commit to.
- **No nondeterminism**: no map iteration, no floats, no locale, no timestamps.
  Integers are rendered base-10.

## Inputs

- `interpretation` — the oracle's resolution-rule text (on-chain `prompt_hash`).
- `facts` — already-verified `(content_hash[32], content)` pairs (Task R3 fetches
  + verifies content against `content_hash`; assembly accepts the verified pair).
- `options` — `{ count, labels? }`; `labels`, when present, carries
  `{ index, label? }` entries matched to indices `0..count`.

## `system` layout

```
{SYSTEM_PREAMBLE}

# Resolution rules

{interpretation}
```

`SYSTEM_PREAMBLE` (fixed, version 1):

```
You are an impartial oracle resolver for a categorical prediction market. Your task is to determine the single correct outcome by applying the resolution rules to the provided facts. Decide based ONLY on the resolution rules and the facts given to you; do not use outside knowledge, assumptions, or information not present below. You must choose exactly one option by its integer index.
```

(Single line; the `\` line-continuations in the Rust source join with no inserted
whitespace beyond the single spaces shown.)

## `user` layout

Three blocks joined by `"\n\n"`, no trailing newline:

```
# Facts

## Fact 1 (sha256: {hex64})
{content}

## Fact 2 (sha256: {hex64})
{content}

# Options

You must choose exactly one of the following options by its integer index:

[0] {label or "(no label)"}
[1] {label or "(no label)"}

# Answer

Respond with the structured JSON output { "option_index": <index> }, where <index> is the integer index (0 to {count-1} inclusive) of the single correct option. Base your choice ONLY on the resolution rules and the facts above.
```

- **Facts**: sorted by `content_hash` ascending, numbered `1..=N` in that order,
  each tagged with its lowercase `content_hash` hex (64 chars) so two distinct
  fact sets cannot render to identical bytes. If there are no facts, the Facts
  body is the literal `(no facts provided)`.
- **Options**: enumerated `[i]` for `i` in `0..count`. With a label: `[i] {label}`.
  Without (no `labels`, or that index has no label): `[i] (no label)`. Labels are
  matched by their explicit `index` field, not vec position.
- **Answer**: `{count-1}` uses saturating subtraction (degenerate `count == 0`
  yields `0`); on-chain `options_count` is always `>= 2`.

## Structured-output schema

`output_schema(options_count)` returns the JSON Schema forcing the answer shape.
Its stable identity is `OUTPUT_SCHEMA_ID = "kassandra.categorical_option_index"`
/ `OUTPUT_SCHEMA_VERSION = 1` (in `runner/src/hashing.rs`, folded into
`params_hash`). The only input-dependent value is `maximum` (= `options_count -
1`):

```json
{
  "type": "object",
  "properties": {
    "option_index": { "type": "integer", "minimum": 0, "maximum": <count-1> }
  },
  "required": ["option_index"],
  "additionalProperties": false
}
```

## Parsing

`parse_option_index(raw_response, options_count) -> Result<u8, ParseError>`:

- Parses the verbatim response JSON; reads `option_index`.
- **Lenient about extra fields**: any field other than `option_index` is ignored
  (a schema-compliant provider sends none, but a stray field never breaks
  parsing).
- **Strict about the value**: rejects malformed JSON (`InvalidJson`), non-object
  (`NotAnObject`), missing field (`MissingField`), non-negative-integer values —
  floats incl. `1.0`, strings, booleans, null, negatives (`NotAnUnsignedInteger`)
  — and `index >= options_count` (`OutOfRange`).

## Regression anchor

For `interpretation = "Resolve YES if BTC closed above $100k on the date;
otherwise NO."`, two facts (`content_hash 0x22… "BTC closed at $98,000."` and
`0x11… "The date in question is 2025-12-31."`, supplied out of order), and options
`{count: 2, labels: [Yes, No]}`, the assembly produces (facts reordered to `0x11`
then `0x22`):

`user`:

```
# Facts

## Fact 1 (sha256: 1111111111111111111111111111111111111111111111111111111111111111)
The date in question is 2025-12-31.

## Fact 2 (sha256: 2222222222222222222222222222222222222222222222222222222222222222)
BTC closed at $98,000.

# Options

You must choose exactly one of the following options by its integer index:

[0] Yes
[1] No

# Answer

Respond with the structured JSON output { "option_index": <index> }, where <index> is the integer index (0 to 1 inclusive) of the single correct option. Base your choice ONLY on the resolution rules and the facts above.
```

The exact `system` + `user` bytes are pinned in the `assembly_regression_anchor`
test in `runner/src/prompt.rs`.

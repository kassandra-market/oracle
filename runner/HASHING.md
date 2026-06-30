# Kassandra claim-metadata hashing — the off-chain protocol contract

This document is **the spec**. The on-chain program stores three opaque 32-byte
commitments in `AiClaim` — `model_id`, `params_hash`, `io_hash` — plus a 1-byte
categorical `option`, and submits them via the `submit_ai_claim` instruction
payload:

```
model_id[32] ++ params_hash[32] ++ io_hash[32] ++ option[1]   = 97 bytes
offset:  0            32             64            96
```

The program does **not** compute the three hashes. The runner defines the
canonical, byte-exact scheme below, and **both** a proposer's runner and a
challenger's independent re-run must follow it byte-for-byte to produce/reproduce
identical commitments. A third party with only this document plus the same
inputs can reproduce all three hashes in any language. The Rust reference
implementation is `runner/src/hashing.rs`.

`option` is **not** hashed — it is a separate plaintext byte in the payload.

## Determinism rules (apply to every preimage)

- **Integers** are fixed-width **big-endian** (`u32` → 4 bytes). No
  platform-dependent widths, no endianness ambiguity.
- **Strings** are their **verbatim UTF-8 bytes**, prefixed by a **4-byte
  big-endian length** (`u32be(byte_len) ++ utf8_bytes`). Call this `str(s)`.
  Strings ≥ 4 GiB are unsupported (out of protocol scope).
- **Optional strings** are a **1-byte presence tag** — `0x00` = none, `0x01` =
  some — followed, when present, by `str(s)`. Call this `opt(s)`.
- No maps / no iteration order, no floating point, no locale, no timestamps.
- Hash function is **SHA-256**; output is 32 bytes.

The length prefixing is what makes adjacent fields collision-free: `str("a") ++
str("bc")` differs from `str("ab") ++ str("c")` even though the naive
concatenations are equal.

## 1. `model_id`

```
model_id = sha256( utf8(model_id_string) )
```

- Preimage: the **verbatim UTF-8 bytes** of the resolved model identifier
  string — no length prefix, no separators.
- Source of the string: `ModelConfig.model_id`, as echoed back in
  `CompletionResponse.model_id` (the model that actually answered). Default
  pinned model: `"claude-opus-4-8"`.

Known answer: `sha256("claude-opus-4-8")` =
`47a46a22f0c9fb105db3f0d8bda83ad51bd59369ab8c8c30cc32ba6356ac5a4a`
(cross-checked with `printf 'claude-opus-4-8' | shasum -a 256`).

## 2. `params_hash`

```
params_hash = sha256( canonical_params_bytes )
```

`canonical_params_bytes` is the concatenation of the following fields **in this
exact order** (the order IS the spec — never reorder):

| # | field                    | encoding                       |
|---|--------------------------|--------------------------------|
| 1 | `prompt_assembly_version`| `u32be`                        |
| 2 | `provider`               | `str(provider)`                |
| 3 | `model_id`               | `str(model_id)`                |
| 4 | `thinking`               | `opt(thinking)`                |
| 5 | `output_schema_id`       | `str(output_schema_id)`        |
| 6 | `output_schema_version`  | `u32be`                        |
| 7 | `max_tokens`             | `u32be`                        |

Field sources / runner-owned constants (see `runner/src/hashing.rs`):

- `prompt_assembly_version` = **`PROMPT_ASSEMBLY_VERSION`** (currently `1`). This
  is a constant the runner owns; **bump it whenever R2's prompt assembly changes
  the model input**, so claims from different assembly versions never collide.
- `provider` = `ModelConfig.provider` (e.g. `"anthropic"`).
- `model_id` = `ModelConfig.model_id` (e.g. `"claude-opus-4-8"`).
- `thinking` = `ModelConfig.thinking` (e.g. `Some("adaptive")`, or `None`).
- `output_schema_id` = **`OUTPUT_SCHEMA_ID`** = `"kassandra.categorical_option_index"`.
- `output_schema_version` = **`OUTPUT_SCHEMA_VERSION`** (currently `1`).
- `max_tokens` = `ModelConfig.max_tokens`.

Known answer for `provider="anthropic"`, `model_id="claude-opus-4-8"`,
`thinking=Some("adaptive")`, `output_schema_id="kassandra.categorical_option_index"`,
`output_schema_version=1`, `max_tokens=1024`, `prompt_assembly_version=1`:
`a08e048d8f780ebcc8122268ee6f2e796e8176632b817f3874d8dc4fc405f9c4`.

## 3. `io_hash`

```
io_hash = sha256( str(system) ++ str(user) ++ utf8(raw_response) )
```

- `system` and `user` are the **exact assembled model input strings** the
  submitter sent (R2 builds these; R1 hashes them as-is). They are
  length-prefixed (`str(...)`), so the system/user boundary is unambiguous.
- `raw_response` is the model's **verbatim response text** — the
  structured-output JSON string, byte-for-byte as returned. It is appended
  **verbatim** (no length prefix) and consumes the remainder of the preimage, so
  there is no boundary ambiguity (everything after `str(system) ++ str(user)` is
  the response).
- This commits to the **exact (input, output) pair** the submitter used. It is a
  commitment, not a reproducibility oracle — frontier models are not
  bit-reproducible; the protocol's decision market arbitrates the categorical
  answer, while `io_hash` proves what the submitter actually ran.

Known answer for `system="Decide the outcome per the interpretation."`,
`user="Facts: ...\nOptions:\n0) yes\n1) no\nChoose exactly one."`,
`raw_response={"option_index":1}`:
`e24990bd43a9d570ea938da194cb7323cb9b1df388211a48f7abaf37479d87c7`.

## Reference reproduction (Python)

```python
import hashlib, struct

def s(b, x):           # str(x): u32be length-prefixed UTF-8
    e = x.encode()
    b += struct.pack(">I", len(e)); b += e

def opt(b, x):         # opt(x): presence tag + optional str
    if x is None: b += b"\x00"
    else: b += b"\x01"; s(b, x)

def model_id(model):
    return hashlib.sha256(model.encode()).hexdigest()

def params_hash(provider, model, thinking, schema_id, schema_ver, max_tokens, asm_ver):
    b = bytearray()
    b += struct.pack(">I", asm_ver)
    s(b, provider); s(b, model); opt(b, thinking)
    s(b, schema_id); b += struct.pack(">I", schema_ver); b += struct.pack(">I", max_tokens)
    return hashlib.sha256(bytes(b)).hexdigest()

def io_hash(system, user, raw_response):
    b = bytearray()
    s(b, system); s(b, user); b += raw_response.encode()
    return hashlib.sha256(bytes(b)).hexdigest()
```

These reproduce the three known-answer values above exactly.

## Payload assembly

`ClaimMetadata::to_payload(option)` writes the 97-byte `submit_ai_claim` payload
with the hashes at offsets 0 / 32 / 64 and `option` at byte 96. The widths and
total length are tied to the R0 constants (`MODEL_ID_LEN`, `PARAMS_HASH_LEN`,
`IO_HASH_LEN`, `OPTION_LEN`, `SUBMIT_AI_CLAIM_PAYLOAD_LEN`), which are in turn
pinned to the on-chain `AiClaim` field layout via compile-time assertions in
`runner/src/constants.rs`.

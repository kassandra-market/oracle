# Runner R0 recon notes

## Decision: DEPEND on `kassandra-oracles-program` (not mirror)

The runner depends on the on-chain program crate as a path dependency with the
`no-entrypoint` feature:

```toml
kassandra-oracles-program = { path = "../programs/oracles", features = ["no-entrypoint"] }
```

### Why it's clean

- The program crate declares `crate-type = ["cdylib", "lib"]`, so a host `lib`
  target already exists.
- Its runtime dependencies are light: `pinocchio` (+ `pinocchio-pubkey`,
  `-system`, `-token`), `bytemuck`, `five8`. `solana-sdk` / `litesvm` /
  `spl-token` are **dev-dependencies only**, so they do NOT enter the runner's
  build graph. No heavy or bpf-only deps leak in.
- `cargo build -p kassandra-oracles-program --lib --features no-entrypoint` builds
  cleanly on the host (aarch64-apple-darwin), in ~2.5s from cold.
- `lib.rs` gates the pinocchio `entrypoint!` (which installs a global allocator
  + panic handler — would conflict in a host binary) behind
  `#[cfg(not(feature = "no-entrypoint"))]`. Enabling `no-entrypoint` is what
  makes the crate safe to link into a normal host binary.

### What the runner reuses (`runner/src/constants.rs`)

- `CLAIM_OPTION_NONE` (0xFF) — re-exported directly from
  `kassandra_oracles_program::state`.
- The `submit_ai_claim` payload widths, pinned to the actual
  `kassandra_oracles_program::state::AiClaim` field layout via `offset_of!`
  compile-time assertions. If the program ever reorders or resizes
  `model_id` / `params_hash` / `io_hash` / `option`, **this crate fails to
  build** — drift is caught at compile time, not runtime.

No constants are mirrored. There is therefore no parity-drift risk for the
pieces R0 needs; the program crate remains the single source of truth and is
left completely unmodified.

## Verified `submit_ai_claim` payload layout

Verified against `programs/oracles/src/processor/submit_ai_claim.rs`
(`Args::parse`, `PAYLOAD_LEN`) and the `AiClaim` struct in
`programs/oracles/src/state.rs`:

```
offset  field        width
0..32   model_id     [u8; 32]
32..64  params_hash  [u8; 32]
64..96  io_hash      [u8; 32]
96      option       u8
                     ---------
                     97 bytes (exact; trailing bytes rejected)
```

(This is the payload AFTER the 1-byte instruction discriminant.) The processor
requires `payload.len() == 97` exactly. `option` must satisfy
`option < oracle.options_count`. AiClaim PDA seeds:
`[b"claim", oracle_pubkey, proposer_pubkey]`.

## Canonical claim-hash module (R1) — recommendation

**Keep the canonical `model_id` / `params_hash` / `io_hash` hashing module in
the runner crate, as THE reference implementation.** Rationale:

1. The on-chain program does NOT compute these hashes — it only stores the three
   32-byte commitments verbatim. So the hashing scheme is a purely off-chain
   contract; it has no on-chain caller that would justify living in the program.
2. The program crate is READ-ONLY for this work (the task forbids modifying its
   source or `Cargo.toml`), so we cannot add a host-only hashing module there
   even if we wanted to.
3. The hashing pulls in `sha2` + the prompt-assembly logic, which are runner
   concerns; the program crate should stay minimal and bpf-targeted.

The runner already shares the on-chain *encoding* facts (payload widths,
`CLAIM_OPTION_NONE`) from the program, so there is no encoding drift. R1 will
document the byte layout of each hash (a `HASHING.md` / rustdoc) so a
third-party challenger can reproduce it independently. The runner's `verify`
path (R4) consumes the same module, guaranteeing proposer/challenger parity.

## Status

Builds, tests, clippy clean (see commit). Program crate untouched.

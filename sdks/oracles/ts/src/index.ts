/**
 * `@kassandra-market/oracles` — the public entry point.
 *
 * Hand-written TypeScript SDK for the Kassandra dispute-core Solana program. It
 * lets clients build every instruction, derive every PDA, and decode every
 * on-chain account, with NO IDL — the wire formats are mirrored from the Rust
 * program (the source of truth) and guarded by a parity test.
 *
 * Re-exports, by area:
 *
 *  - **constants** — {@link KASSANDRA_PROGRAM_ID}, {@link Ix}, {@link AccountType},
 *    {@link Phase}, {@link KassandraError} + {@link decodeError},
 *    {@link EXTERNAL_PROGRAM_IDS}, {@link CONFIG}, {@link ACCOUNT_SIZES},
 *    {@link SYSTEM_PROGRAM_ID} / {@link TOKEN_PROGRAM_ID}, the sentinels
 *    ({@link CLAIM_OPTION_NONE}, {@link VOTE_APPROVE}, {@link VOTE_DUPLICATE}).
 *  - **PDAs** — all derivation fns (`oracle(nonce)`, `proposer`, `fact`, …); each
 *    is also reachable under the {@link pda} namespace (e.g. `pda.oracle(1n)`).
 *  - **account decoders** — `decodeProtocol` / `decodeOracle` / … and their types.
 *  - **instruction builders** — all 23 builders + their `*Args` param types, plus
 *    the low-level payload helpers.
 *  - **litesvm interop** — {@link toLiteSvmTransaction}, the web3.js-v3 → litesvm
 *    bridge (types-only import of litesvm; safe to import without it installed —
 *    handy for local/integration testing).
 */
export * from "./constants.js";

// PDA derivation: both flat (`oracle(nonce)`) and namespaced (`pda.oracle(nonce)`).
export * from "./pda.js";
export * as pda from "./pda.js";

// The 7 Pod account decoders + their decoded types.
export * from "./accounts/index.js";

// The 23 instruction builders, their `*Args` param types, + payload helpers.
export * from "./instructions/index.js";

// web3.js v3 ↔ litesvm transaction bridge (for local testing).
export * from "./litesvm-interop.js";

// v0-tx + Address Lookup Table path for near-cap finalizes (live-cluster only).
export * from "./v0.js";

// Runner → SDK bridge: submitAiClaimFromRunner + the runner-payload parity guard.
export * from "./runner-bridge.js";

// Futarchy v0.6 + Squads v4 builders + governance bootstrap (under `futarchy.*`).
export * as futarchy from "./futarchy/index.js";

// MetaDAO v0.4 standalone AMM builders + PDA derivers (under `ammV04.*`).
export * as ammV04 from "./amm-v04/index.js";

// Meteora DAMM v2 (cp-amm) spot-path builders + Pool/Position decoders (under `meteora.*`).
export * as meteora from "./meteora/index.js";

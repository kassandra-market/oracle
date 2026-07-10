//! Kassandra off-chain AI runner (library).
//!
//! Takes an oracle's fixed prompt + interpretation, the agreed fact set, and
//! the categorical options; calls a pinned model behind a generic
//! [`provider::AiProvider`]; and emits the on-chain claim metadata
//! (`model_id` / `params_hash` / `io_hash` + the chosen `option`) that
//! `submit_ai_claim` records and a challenger can independently reproduce.
//!
//! The on-chain program (via `kassandra_oracles_sdk`, which re-exports it) is the source of truth for the
//! claim encoding; see [`constants`] for what the runner reuses from it.
//!
//! Task R0 scaffolds the crate, the provider trait + a deterministic mock, and
//! the constants recon; R1–R3 fill in [`hashing`], [`prompt`], and [`fetch`];
//! R4 adds the default [`anthropic`] provider and the [`cli`] (`run`/`verify`).

pub mod anthropic;
pub mod cli;
pub mod constants;
pub mod fetch;
pub mod hashing;
pub mod prompt;
pub mod provider;
pub mod rpc;
pub mod submit;

//! Kassandra off-chain AI runner (library).
//!
//! Takes an oracle's fixed prompt + interpretation, the agreed fact set, and
//! the categorical options; calls a pinned model behind a generic
//! [`provider::AiProvider`]; and emits the on-chain claim metadata
//! (`model_id` / `params_hash` / `io_hash` + the chosen `option`) that
//! `submit_ai_claim` records and a challenger can independently reproduce.
//!
//! The on-chain program ([`kassandra_program`]) is the source of truth for the
//! claim encoding; see [`constants`] for what the runner reuses from it.
//!
//! Task R0 scaffolds the crate, the provider trait + a deterministic mock, and
//! the constants recon. Later tasks fill in the stub modules below.

pub mod constants;
pub mod hashing;
pub mod prompt;
pub mod provider;

// --- stubs for later tasks --------------------------------------------------

/// R3: agreed-fact fetching + `content_hash` verification.
pub mod fetch {}

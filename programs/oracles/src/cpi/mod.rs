//! Hand-built cross-program-invocation interfaces to external programs.
//!
//! Kassandra's decision-market challenge (design §6) reuses MetaDAO's deployed
//! `conditional_vault` (and, later, `amm`) programs. Those are Anchor programs;
//! we are a Pinocchio program with no Anchor dependency, so we drive them by
//! reconstructing their wire format by hand: the 8-byte Anchor sighash
//! discriminator, the exact ordered account metas, and Borsh-encoded args.
//!
//! See [`metadao`] for the resolved program IDs, discriminators, and the
//! account orderings verified against the real on-chain binaries.

pub mod metadao;
pub mod metadao_v06;

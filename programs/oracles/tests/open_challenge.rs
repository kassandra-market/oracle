//! `open_challenge` (Task 10): verify a decomposed MetaDAO decision market,
//! program-sign the proposer-KASS split, and record a [`Market`] PDA.
//!
//! The challenger composes the MetaDAO accounts (binary question with
//! resolver == the Kassandra oracle PDA, a KASS conditional vault, a USDC
//! conditional vault, and pass/fail AMMs) by driving the REAL deployed
//! conditional_vault binary in-test (same wire format as `metadao_cpi.rs`).
//! `open_challenge` then verifies + records them and splits the proposer's
//! escrowed KASS into pass/fail conditional KASS, all program-signed.

mod common;
use common::*;

const VAULT_SO: &[u8] = include_bytes!("fixtures/metadao_conditional_vault.so");
const AMM_SO: &[u8] = include_bytes!("fixtures/metadao_amm.so");

#[path = "open_challenge/support.rs"]
mod support;
#[path = "open_challenge/fixture.rs"]
mod fixture;
#[path = "open_challenge/escrow.rs"]
mod escrow;
#[path = "open_challenge/amm_binding.rs"]
mod amm_binding;
#[path = "open_challenge/guards.rs"]
mod guards;

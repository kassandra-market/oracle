//! `settle_challenge` (Task 11): read the decision-market TWAP from the REAL
//! deployed MetaDAO `amm` v0.4 binary, apply the slash trigger, and resolve the
//! conditional-vault question — all driven against the real programs in LiteSVM.
//!
//! Each test composes the MetaDAO market exactly like `open_challenge.rs` (a
//! binary question whose resolver is the Kassandra oracle PDA + KASS/USDC
//! conditional vaults), then builds GENUINE pass/fail AMM pools via the real
//! `create_amm` + `add_liquidity` + `crank_that_twap` instructions so the TWAP
//! `settle_challenge` reads is produced by the real binary — not fabricated.
//! `open_challenge` records the real AMM addresses on the `Market`; `settle`
//! then HARD-binds each AMM to this market's conditional mint pair, reads the
//! TWAP, and slashes / resolves accordingly.
//!
//! Shared fixtures live in `settle_challenge/{support,fixtures}.rs`; the
//! `#[test]` fns are grouped into `settle_challenge/{resolve,guards,fees}.rs`.

mod common;
use common::*;

// The `.so` fixtures are loaded via `include_bytes!`, whose path is resolved
// relative to THIS file, so the two program-blob consts stay in the crate root
// (the child modules reach them through `use super::*`).
const VAULT_SO: &[u8] = include_bytes!("fixtures/metadao_conditional_vault.so");
const AMM_SO: &[u8] = include_bytes!("fixtures/metadao_amm.so");

#[path = "settle_challenge/support.rs"]
mod support;
#[path = "settle_challenge/fixtures.rs"]
mod fixtures;
#[path = "settle_challenge/resolve.rs"]
mod resolve;
#[path = "settle_challenge/guards.rs"]
mod guards;
#[path = "settle_challenge/fees.rs"]
mod fees;

//! Account-meta golden guard — pins each instruction's ACCOUNT CONTRACT.
//!
//! The `tests/parity.rs` guard locks discriminators, sizes, and payload bytes,
//! but a wrong account ORDER or a flipped `is_signer`/`is_writable` flag would
//! still slip past CI. This file closes that gap: for every `ix::*` builder AND
//! every `metadao::*` account builder, it builds the instruction with DISTINCT
//! placeholder `Pubkey`s (deriving every PDA the builder derives), then asserts
//! the resulting `Vec<(role, is_signer, is_writable)>` equals a HARDCODED literal
//! golden — a hand-written frozen snapshot, NOT computed from the builder.
//!
//! Cross-checked against the program processors' `let [a, b, ..]` destructures
//! (`programs/markets/src/processor/*.rs`) and the CPI metas
//! (`activate.rs` split_metas, `collect_fee.rs` redeem_metas). Where the two SDKs
//! disagreed the PROGRAM wins; the labels + order here are IDENTICAL to the TS
//! golden in `sdks/oracles/ts/test/account-metas.test.ts`, so both SDKs encode ONE contract.
//! Any future account-order/flag drift in either SDK fails these tests.
//!
//! The `#[test]`s are split across sibling modules (`kassandra`, `metadao`); this
//! file keeps the shared placeholder/labeling helpers they build on.

#[path = "account_metas/kassandra.rs"]
mod kassandra;
#[path = "account_metas/metadao.rs"]
mod metadao;

use kassandra_markets_sdk::metadao as md;
use solana_sdk::{instruction::Instruction, pubkey::Pubkey};
use std::collections::HashMap;

/// A single golden row: role name + signer/writable flags.
pub(crate) type Meta = (&'static str, bool, bool);

/// Deterministic distinct placeholder pubkey (32 bytes of `n`).
pub(crate) fn pk(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}

/// Label each account meta by looking its pubkey up in `entries`. Panics on any
/// unmapped account so a builder that grew/moved a slot fails loudly.
pub(crate) fn labeled(ix: &Instruction, entries: Vec<(Pubkey, &'static str)>) -> Vec<Meta> {
    let map: HashMap<Pubkey, &'static str> = entries.into_iter().collect();
    ix.accounts
        .iter()
        .map(|m| {
            let name = map
                .get(&m.pubkey)
                .unwrap_or_else(|| panic!("unmapped account {} in {:?}", m.pubkey, ix.program_id));
            (*name, m.is_signer, m.is_writable)
        })
        .collect()
}

/// The fixed program-id accounts, by role. Spread into per-instruction maps.
fn programs() -> Vec<(Pubkey, &'static str)> {
    vec![
        (solana_sdk::system_program::id(), "systemProgram"),
        (spl_token::id(), "tokenProgram"),
        (md::ASSOCIATED_TOKEN_PROGRAM_ID, "ataProgram"),
        (md::CONDITIONAL_VAULT_ID, "cvProgram"),
        (md::AMM_ID, "ammProgram"),
    ]
}

pub(crate) fn with_programs(
    mut entries: Vec<(Pubkey, &'static str)>,
) -> Vec<(Pubkey, &'static str)> {
    entries.extend(programs());
    entries
}

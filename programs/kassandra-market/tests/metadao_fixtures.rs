//! Sanity check that the vendored MetaDAO `.so` fixtures load into LiteSVM and
//! are deployed as executable programs at their canonical program IDs.

mod common;
use common::*;

use solana_sdk::pubkey::Pubkey;

const VAULT_ID: Pubkey = solana_sdk::pubkey!("VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg");
const AMM_ID: Pubkey = solana_sdk::pubkey!("AMMyu265tkBpRW21iGQxKGLaves3gKm2JcMUqfXNSpqD");

#[test]
fn metadao_programs_load_executable() {
    let mut ctx = TestCtx::new();
    ctx.load_metadao();

    let vault = ctx
        .svm
        .get_account(&VAULT_ID)
        .expect("conditional_vault program not deployed");
    assert!(vault.executable, "conditional_vault must be executable");

    let amm = ctx
        .svm
        .get_account(&AMM_ID)
        .expect("amm program not deployed");
    assert!(amm.executable, "amm must be executable");
}

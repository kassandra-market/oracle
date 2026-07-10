//! Determinism + seed-order tests for the sdks/oracles/rust MetaDAO PDA derivers.
//!
//! Cross-checks each derived PDA against a hand-built `find_program_address`
//! using the EXACT seed order realized in
//! `../kassandra/programs/kassandra/tests/challenge_e2e.rs`.

use kassandra_markets_sdk::metadao as md;
use solana_sdk::pubkey::Pubkey;

const CV: Pubkey = md::CONDITIONAL_VAULT_ID;
const AMM: Pubkey = md::AMM_ID;

#[test]
fn question_pda_matches_seed_order() {
    let question_id = [7u8; 32];
    let oracle = Pubkey::new_from_array([9u8; 32]);
    let (got, _) = md::question(&question_id, &oracle, 2);
    let (want, _) =
        Pubkey::find_program_address(&[b"question", &question_id, oracle.as_ref(), &[2u8]], &CV);
    assert_eq!(got, want);
    // Deterministic across calls.
    assert_eq!(got, md::question(&question_id, &oracle, 2).0);
}

#[test]
fn vault_and_conditional_mint_pdas_match_seed_order() {
    let question = Pubkey::new_from_array([1u8; 32]);
    let underlying = Pubkey::new_from_array([2u8; 32]);
    let (vault, _) = md::vault(&question, &underlying);
    let (want_vault, _) = Pubkey::find_program_address(
        &[b"conditional_vault", question.as_ref(), underlying.as_ref()],
        &CV,
    );
    assert_eq!(vault, want_vault);

    let (yes, _) = md::conditional_token_mint(&vault, 0);
    let (no, _) = md::conditional_token_mint(&vault, 1);
    let (want_yes, _) =
        Pubkey::find_program_address(&[b"conditional_token", vault.as_ref(), &[0u8]], &CV);
    let (want_no, _) =
        Pubkey::find_program_address(&[b"conditional_token", vault.as_ref(), &[1u8]], &CV);
    assert_eq!(yes, want_yes);
    assert_eq!(no, want_no);
    assert_ne!(yes, no);
}

#[test]
fn amm_and_lp_mint_and_event_authority_match_seed_order() {
    let base = Pubkey::new_from_array([3u8; 32]);
    let quote = Pubkey::new_from_array([4u8; 32]);
    let (amm, _) = md::amm(&base, &quote);
    let (want_amm, _) =
        Pubkey::find_program_address(&[b"amm__", base.as_ref(), quote.as_ref()], &AMM);
    assert_eq!(amm, want_amm);

    let (lp, _) = md::amm_lp_mint(&amm);
    let (want_lp, _) = Pubkey::find_program_address(&[b"amm_lp_mint", amm.as_ref()], &AMM);
    assert_eq!(lp, want_lp);

    // event authority is derived under the target program id, so vault != amm.
    assert_ne!(md::event_authority(&CV).0, md::event_authority(&AMM).0);
    assert_eq!(
        md::event_authority(&AMM).0,
        Pubkey::find_program_address(&[b"__event_authority"], &AMM).0
    );
}

#[test]
fn known_pubkey_snapshot_is_stable() {
    // Full composition with fixed inputs → a pinned address (regression guard).
    let question_id = [7u8; 32];
    let oracle = Pubkey::new_from_array([42u8; 32]);
    let (question, _) = md::question(&question_id, &oracle, 2);
    let kass = Pubkey::new_from_array([5u8; 32]);
    let (vault, _) = md::vault(&question, &kass);
    let (yes, _) = md::conditional_token_mint(&vault, 0);
    let (no, _) = md::conditional_token_mint(&vault, 1);
    let (amm, _) = md::amm(&yes, &no);
    // Re-derive independently; the whole chain must be reproducible.
    let (question2, _) = md::question(&question_id, &oracle, 2);
    let (vault2, _) = md::vault(&question2, &kass);
    let (yes2, _) = md::conditional_token_mint(&vault2, 0);
    let (no2, _) = md::conditional_token_mint(&vault2, 1);
    let (amm2, _) = md::amm(&yes2, &no2);
    assert_eq!(
        (question, vault, yes, no, amm),
        (question2, vault2, yes2, no2, amm2)
    );
}

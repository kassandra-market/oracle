use kassandra_markets_program::cpi::metadao as prog_md;
use kassandra_markets_program::instruction::Ix;
use kassandra_markets_sdk as sdk;
use kassandra_markets_sdk::metadao as sdk_md;

#[test]
fn ix_discriminants_match_sdk() {
    assert_eq!(Ix::InitConfig as u8, sdk::IX_INIT_CONFIG);
    assert_eq!(Ix::UpdateConfig as u8, sdk::IX_UPDATE_CONFIG);
    assert_eq!(Ix::CreateMarket as u8, sdk::IX_CREATE_MARKET);
    assert_eq!(Ix::Contribute as u8, sdk::IX_CONTRIBUTE);
    assert_eq!(Ix::Cancel as u8, sdk::IX_CANCEL);
    assert_eq!(Ix::Refund as u8, sdk::IX_REFUND);
    assert_eq!(Ix::Activate as u8, sdk::IX_ACTIVATE);
    assert_eq!(Ix::ClaimLp as u8, sdk::IX_CLAIM_LP);
    assert_eq!(Ix::ResolveMarket as u8, sdk::IX_RESOLVE_MARKET);
    assert_eq!(Ix::CollectFee as u8, sdk::IX_COLLECT_FEE);
    assert_eq!(Ix::CloseMarket as u8, sdk::IX_CLOSE_MARKET);
}

#[test]
fn program_id_matches_sdk() {
    assert_eq!(
        kassandra_markets_program::ID.to_bytes(),
        kassandra_markets_sdk::PROGRAM_ID.to_bytes()
    );
}

/// The MetaDAO wire-format constants live in BOTH crates (program invokes
/// split/add_liquidity; sdk composes the rest). They must never drift.
#[test]
fn metadao_discriminators_match_across_crates() {
    assert_eq!(prog_md::SPLIT_TOKENS_DISC, sdk_md::SPLIT_TOKENS_DISC);
    assert_eq!(
        prog_md::RESOLVE_QUESTION_DISC,
        sdk_md::RESOLVE_QUESTION_DISC
    );
    assert_eq!(prog_md::REDEEM_TOKENS_DISC, sdk_md::REDEEM_TOKENS_DISC);
    assert_eq!(prog_md::ADD_LIQUIDITY_DISC, sdk_md::ADD_LIQUIDITY_DISC);
    assert_eq!(
        prog_md::AMM_ACCOUNT_DISCRIMINATOR,
        sdk_md::AMM_ACCOUNT_DISCRIMINATOR
    );
}

#[test]
fn metadao_program_ids_match_across_crates() {
    assert_eq!(
        prog_md::CONDITIONAL_VAULT_ID.to_bytes(),
        sdk_md::CONDITIONAL_VAULT_ID.to_bytes()
    );
    assert_eq!(prog_md::AMM_ID.to_bytes(), sdk_md::AMM_ID.to_bytes());
}

/// The program's arg encoders must byte-match the sdk composer's.
#[test]
fn metadao_arg_encoders_match_across_crates() {
    assert_eq!(
        prog_md::split_tokens_data(12_345).as_slice(),
        sdk_md::split_tokens_data(12_345).as_slice()
    );
    assert_eq!(
        prog_md::add_liquidity_data(7, 8, 9).as_slice(),
        sdk_md::add_liquidity_data(7, 8, 9).as_slice()
    );
    assert_eq!(
        prog_md::redeem_tokens_data().as_slice(),
        sdk_md::redeem_tokens_data().as_slice()
    );
}

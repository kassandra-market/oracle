use core::mem::{offset_of, size_of};
use kassandra_markets_program::state::*;

#[test]
fn account_sizes_are_stable() {
    assert_eq!(size_of::<Config>(), Config::LEN);
    assert_eq!(size_of::<Market>(), Market::LEN);
    assert_eq!(size_of::<Contribution>(), Contribution::LEN);
    assert_eq!(Config::LEN, 120);
    assert_eq!(Market::LEN, 400);
    assert_eq!(Contribution::LEN, 88);
}

#[test]
fn field_offsets_are_pinned() {
    assert_eq!(offset_of!(Config, account_type), 0);
    assert_eq!(offset_of!(Config, authority), 8);
    assert_eq!(offset_of!(Config, kass_mint), 40);
    assert_eq!(offset_of!(Config, min_liquidity), 72);
    // Fee config appended after `bump` (@80); Phase-1 offsets above unchanged.
    assert_eq!(offset_of!(Config, fee_bps), 82);
    assert_eq!(offset_of!(Config, fee_destination), 84);

    assert_eq!(offset_of!(Market, account_type), 0);
    assert_eq!(offset_of!(Market, oracle), 8);
    assert_eq!(offset_of!(Market, creator), 40);
    assert_eq!(offset_of!(Market, kass_mint), 72);
    assert_eq!(offset_of!(Market, escrow_vault), 104);
    assert_eq!(offset_of!(Market, min_liquidity), 136);
    assert_eq!(offset_of!(Market, total_contributed), 144);
    assert_eq!(offset_of!(Market, open_contributions), 152);
    assert_eq!(offset_of!(Market, status), 154);
    // Phase-2a MetaDAO bindings, appended after the pinned Phase-1 tail.
    assert_eq!(offset_of!(Market, question), 160);
    assert_eq!(offset_of!(Market, vault), 192);
    assert_eq!(offset_of!(Market, yes_mint), 224);
    assert_eq!(offset_of!(Market, no_mint), 256);
    assert_eq!(offset_of!(Market, amm), 288);
    assert_eq!(offset_of!(Market, lp_mint), 320);
    assert_eq!(offset_of!(Market, lp_vault), 352);
    assert_eq!(offset_of!(Market, lp_total), 384);
    assert_eq!(offset_of!(Market, settled), 392);
    // Fee snapshot appended after `settled`, absorbed by the pre-existing tail pad.
    assert_eq!(offset_of!(Market, fee_bps), 394);
    // `fee_collected` flag appended after `fee_bps`, still within the tail pad.
    assert_eq!(offset_of!(Market, fee_collected), 396);
    // `outcome_index` appended after `fee_collected`, absorbed by the tail pad (LEN 400).
    assert_eq!(offset_of!(Market, outcome_index), 397);

    assert_eq!(offset_of!(Contribution, account_type), 0);
    assert_eq!(offset_of!(Contribution, market), 8);
    assert_eq!(offset_of!(Contribution, contributor), 40);
    assert_eq!(offset_of!(Contribution, amount), 72);
    assert_eq!(offset_of!(Contribution, claimed), 80);
}

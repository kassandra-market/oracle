//! Byte-exact wire-format tests for the program-side MetaDAO CPI helpers.

use kassandra_markets_program::cpi::metadao as m;

#[test]
fn split_tokens_data_is_byte_exact() {
    // disc ++ amount(u64 LE)
    let got = m::split_tokens_data(0x0102_0304_0506_0708);
    let mut want = [0u8; 16];
    want[0..8].copy_from_slice(&[0x4f, 0xc3, 0x74, 0x00, 0x8c, 0xb0, 0x49, 0xb3]);
    want[8..16].copy_from_slice(&0x0102_0304_0506_0708u64.to_le_bytes());
    assert_eq!(got, want);
    // spot-check the LE byte order explicitly.
    assert_eq!(
        &got[8..16],
        &[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]
    );
}

#[test]
fn add_liquidity_data_is_byte_exact() {
    // disc ++ quote_amount ++ max_base_amount ++ min_lp_tokens (all u64 LE)
    let got = m::add_liquidity_data(1, 2, 3);
    let mut want = [0u8; 32];
    want[0..8].copy_from_slice(&[0xb5, 0x9d, 0x59, 0x43, 0x8f, 0xb6, 0x34, 0x48]);
    want[8..16].copy_from_slice(&1u64.to_le_bytes());
    want[16..24].copy_from_slice(&2u64.to_le_bytes());
    want[24..32].copy_from_slice(&3u64.to_le_bytes());
    assert_eq!(got, want);
}

#[test]
fn remove_liquidity_data_is_byte_exact() {
    // disc ++ lp_tokens_to_burn ++ min_base_amount ++ min_quote_amount (all u64 LE).
    let got = m::remove_liquidity_data(0x1122_3344_5566_7788, 0, 0);
    let mut want = [0u8; 32];
    // sha256("global:remove_liquidity")[..8].
    want[0..8].copy_from_slice(&[0x50, 0x55, 0xd1, 0x48, 0x18, 0xce, 0xb1, 0x6c]);
    want[8..16].copy_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
    // mins default to 0.
    assert_eq!(got, want);
    assert_eq!(
        m::REMOVE_LIQUIDITY_DISC,
        [0x50, 0x55, 0xd1, 0x48, 0x18, 0xce, 0xb1, 0x6c]
    );
    // Non-zero mins land in the right slots.
    let got = m::remove_liquidity_data(1, 2, 3);
    assert_eq!(&got[8..16], &1u64.to_le_bytes());
    assert_eq!(&got[16..24], &2u64.to_le_bytes());
    assert_eq!(&got[24..32], &3u64.to_le_bytes());
}

#[test]
fn redeem_tokens_data_is_disc_only() {
    // sha256("global:redeem_tokens")[..8], no args.
    let got = m::redeem_tokens_data();
    assert_eq!(got, [0xf6, 0x62, 0x86, 0x29, 0x98, 0x21, 0x78, 0x45]);
    assert_eq!(got, m::REDEEM_TOKENS_DISC);
}

#[test]
fn mint_supply_read_at_offset() {
    // SPL Mint.supply (u64 LE) lives at byte 36 (after mint_authority COption).
    let mut buf = vec![0u8; 82]; // spl_token::state::Mint::LEN
    let supply: u64 = 123_456_789;
    buf[m::MINT_SUPPLY_OFFSET..m::MINT_SUPPLY_OFFSET + 8].copy_from_slice(&supply.to_le_bytes());
    assert_eq!(m::read_u64(&buf, m::MINT_SUPPLY_OFFSET).unwrap(), supply);
    assert_eq!(m::MINT_SUPPLY_OFFSET, 36);
}

#[test]
fn resolve_question_data_binary_is_byte_exact() {
    // disc ++ len(u32 LE == 2) ++ num0(u32 LE) ++ num1(u32 LE)
    let got = m::resolve_question_data_binary([1, 0]);
    let mut want = [0u8; 20];
    want[0..8].copy_from_slice(&[0x34, 0x20, 0xe0, 0xb3, 0xb4, 0x08, 0x00, 0xf6]);
    want[8..12].copy_from_slice(&2u32.to_le_bytes());
    want[12..16].copy_from_slice(&1u32.to_le_bytes());
    want[16..20].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(got, want);
    // spot-check the Vec length prefix + the void encoding.
    assert_eq!(&got[8..12], &[0x02, 0x00, 0x00, 0x00]);
    assert_eq!(
        &m::resolve_question_data_binary([0, 1])[12..20],
        &[0, 0, 0, 0, 1, 0, 0, 0]
    );
    assert_eq!(
        &m::resolve_question_data_binary([1, 1])[12..20],
        &[1, 0, 0, 0, 1, 0, 0, 0]
    );
}

#[test]
fn read_pubkey_at_offset() {
    // Hand-build a buffer with a known pubkey at the Amm.base_mint offset (49).
    let key = [0xAB; 32];
    let mut buf = vec![0u8; 128];
    buf[m::AMM_BASE_MINT_OFFSET..m::AMM_BASE_MINT_OFFSET + 32].copy_from_slice(&key);
    assert_eq!(
        m::read_pubkey(&buf, m::AMM_BASE_MINT_OFFSET)
            .unwrap()
            .to_bytes(),
        key
    );
    // out-of-bounds → error.
    assert!(m::read_pubkey(&buf, 200).is_err());
}

#[test]
fn read_u32_at_offset() {
    // num_outcomes length prefix (u32 LE) at the Question offset (72).
    let mut buf = vec![0u8; 128];
    buf[m::QUESTION_NUM_OUTCOMES_LEN_OFFSET..m::QUESTION_NUM_OUTCOMES_LEN_OFFSET + 4]
        .copy_from_slice(&2u32.to_le_bytes());
    assert_eq!(
        m::read_u32(&buf, m::QUESTION_NUM_OUTCOMES_LEN_OFFSET).unwrap(),
        2
    );
    assert!(m::read_u32(&buf, 200).is_err());
}

#[test]
fn read_u64_at_offset() {
    let mut buf = vec![0u8; 32];
    let val: u64 = 0xDEAD_BEEF_CAFE_1234;
    buf[8..16].copy_from_slice(&val.to_le_bytes());
    assert_eq!(m::read_u64(&buf, 8).unwrap(), val);
    assert!(m::read_u64(&buf, 30).is_err());
}

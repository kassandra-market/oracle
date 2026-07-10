//! Wire-format regression tests for the instruction builders + PDAs. These pin
//! the discriminants, payload lengths, and account counts to the on-chain
//! contract so a program renumber or account-order change is caught here.

use kassandra_oracles_sdk::{ix, pda, ConfigParams, Ix, Pubkey, PROGRAM_ID};

fn pk(b: u8) -> Pubkey {
    Pubkey::new_from_array([b; 32])
}

#[test]
fn program_id_is_canonical() {
    assert_eq!(
        PROGRAM_ID,
        Pubkey::from_str_const("KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY")
    );
}

#[test]
fn create_oracle_payload_is_25_plus_disc_in_field_order() {
    let ix = ix::create_oracle(
        &PROGRAM_ID,
        42,    // nonce
        3,     // options_count
        1_234, // deadline
        600,   // twap_window
        pk(1),
        pk(2),
        pk(3),
        pk(4),
        pk(5),
    );
    assert_eq!(ix.data[0], Ix::CreateOracle as u8);
    assert_eq!(ix.data.len(), 1 + 25);
    // nonce (LE u64) at payload offset 0.
    assert_eq!(&ix.data[1..9], &42u64.to_le_bytes());
    // options_count at offset 8 (prompt_hash removed).
    assert_eq!(ix.data[9], 3);
    // deadline (LE i64) at offset 9.
    assert_eq!(&ix.data[10..18], &1_234i64.to_le_bytes());
    // twap_window (LE i64) at offset 17.
    assert_eq!(&ix.data[18..26], &600i64.to_le_bytes());
    assert_eq!(ix.accounts.len(), 10);
}

#[test]
fn write_oracle_meta_body_is_length_prefixed_in_order() {
    let uri_hash = [9u8; 32];
    let ix = ix::write_oracle_meta(
        &PROGRAM_ID,
        pk(1), // oracle
        pk(2), // creator
        "Q?",
        &["Yes", "No"],
        "u",
        &uri_hash,
    );
    assert_eq!(ix.data[0], Ix::WriteOracleMeta as u8);
    let body = &ix.data[1..];
    let mut off = 0usize;
    // subject
    assert_eq!(&body[off..off + 2], &2u16.to_le_bytes());
    off += 2;
    assert_eq!(&body[off..off + 2], b"Q?");
    off += 2;
    // options
    assert_eq!(body[off], 2);
    off += 1;
    for label in ["Yes", "No"] {
        assert_eq!(&body[off..off + 2], &(label.len() as u16).to_le_bytes());
        off += 2;
        assert_eq!(&body[off..off + label.len()], label.as_bytes());
        off += label.len();
    }
    // uri
    assert_eq!(&body[off..off + 2], &1u16.to_le_bytes());
    off += 2;
    assert_eq!(&body[off..off + 1], b"u");
    off += 1;
    // uri_hash
    assert_eq!(&body[off..off + 32], &uri_hash);
    assert_eq!(off + 32, body.len(), "no trailing bytes");
    // creator (signer) + oracle + meta + system.
    assert_eq!(ix.accounts.len(), 4);
    assert!(ix.accounts[0].is_signer);
}

#[test]
fn propose_payload_is_option_then_bond() {
    let ix = ix::propose(&PROGRAM_ID, pk(1), pk(2), pk(3), pk(4), pk(5), 2, 1_000);
    assert_eq!(ix.data[0], Ix::Propose as u8);
    assert_eq!(ix.data.len(), 1 + 9);
    assert_eq!(ix.data[1], 2); // option
    assert_eq!(&ix.data[2..10], &1_000u64.to_le_bytes());
    assert_eq!(ix.accounts.len(), 7);
    assert!(ix.accounts[2].is_signer); // authority signs
}

#[test]
fn submit_ai_claim_component_equals_raw() {
    let model = [0xaau8; 32];
    let params = [0xbbu8; 32];
    let io = [0xccu8; 32];
    let from_parts = ix::submit_ai_claim(
        &PROGRAM_ID,
        pk(1),
        pk(2),
        pk(3),
        pk(4),
        &model,
        &params,
        &io,
        5,
    );
    let mut payload = [0u8; 97];
    payload[0..32].copy_from_slice(&model);
    payload[32..64].copy_from_slice(&params);
    payload[64..96].copy_from_slice(&io);
    payload[96] = 5;
    let from_raw = ix::submit_ai_claim_raw(&PROGRAM_ID, pk(1), pk(2), pk(3), pk(4), &payload);
    assert_eq!(from_parts.data, from_raw.data);
    assert_eq!(from_parts.data[0], Ix::SubmitAiClaim as u8);
    assert_eq!(from_parts.data.len(), 1 + 97);
    assert_eq!(from_parts.accounts.len(), 5);
}

#[test]
fn set_config_payload_is_200_bytes() {
    let ix = ix::set_config(&PROGRAM_ID, pk(1), pk(2), &ConfigParams::defaults());
    assert_eq!(ix.data[0], Ix::SetConfig as u8);
    assert_eq!(ix.data.len(), 1 + 200);
}

#[test]
fn resolve_deadend_payload_is_single_option_byte() {
    let ix = ix::resolve_deadend(&PROGRAM_ID, pk(1), pk(2), pk(3), 4);
    assert_eq!(ix.data, vec![Ix::ResolveDeadend as u8, 4]);
}

#[test]
fn close_ai_claim_has_empty_payload() {
    let ix = ix::close_ai_claim(&PROGRAM_ID, pk(1), pk(2), pk(3));
    assert_eq!(ix.data, vec![Ix::CloseAiClaim as u8]);
    assert_eq!(ix.accounts.len(), 3);
}

#[test]
fn mega_instruction_account_counts() {
    let oc = ix::OpenChallengeAccounts {
        oracle: pk(1),
        ai_claim: pk(2),
        proposer: pk(3),
        market: pk(4),
        challenger: pk(5),
        question: pk(6),
        kass_vault: pk(7),
        usdc_vault: pk(8),
        pass_amm: pk(9),
        fail_amm: pk(10),
        stake_vault: pk(11),
        kass_vault_underlying: pk(12),
        pass_kass_mint: pk(13),
        fail_kass_mint: pk(14),
        oracle_pass_kass: pk(15),
        oracle_fail_kass: pk(16),
        cv_program: pk(17),
        cv_event_authority: pk(18),
        protocol: pk(19),
        kass_dao: pk(20),
        usdc_mint: pk(21),
        challenger_usdc_src: pk(22),
        challenger_usdc_vault: pk(23),
    };
    let ix = ix::open_challenge(&PROGRAM_ID, &oc, 7);
    assert_eq!(ix.accounts.len(), 25);
    assert_eq!(ix.data[0], Ix::OpenChallenge as u8);
    assert_eq!(&ix.data[1..9], &7u64.to_le_bytes());

    let sc = ix::SettleChallengeAccounts {
        oracle: pk(1),
        market: pk(2),
        ai_claim: pk(3),
        proposer: pk(4),
        question: pk(5),
        pass_amm: pk(6),
        fail_amm: pk(7),
        cv_program: pk(8),
        cv_event_authority: pk(9),
        stake_vault: pk(10),
        kass_vault: pk(11),
        kass_vault_underlying: pk(12),
        pass_kass_mint: pk(13),
        fail_kass_mint: pk(14),
        oracle_pass_kass: pk(15),
        oracle_fail_kass: pk(16),
        challenger_usdc_vault: pk(17),
        proposer_usdc: pk(18),
        challenger_usdc_dest: pk(19),
        challenger_kass: pk(20),
    };
    let ix = ix::settle_challenge(&PROGRAM_ID, &sc, 7);
    assert_eq!(ix.accounts.len(), 21);
}

#[test]
fn pdas_match_documented_seeds() {
    let oracle = pk(1);
    let authority = pk(2);
    assert_eq!(
        pda::oracle(&PROGRAM_ID, 42).0,
        Pubkey::find_program_address(&[b"oracle", &42u64.to_le_bytes()], &PROGRAM_ID).0
    );
    assert_eq!(
        pda::proposer(&PROGRAM_ID, &oracle, &authority).0,
        Pubkey::find_program_address(
            &[b"proposer", oracle.as_ref(), authority.as_ref()],
            &PROGRAM_ID
        )
        .0
    );
    assert_eq!(
        pda::ai_claim(&PROGRAM_ID, &oracle, &authority).0,
        Pubkey::find_program_address(
            &[b"claim", oracle.as_ref(), authority.as_ref()],
            &PROGRAM_ID
        )
        .0
    );
}

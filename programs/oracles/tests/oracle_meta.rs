//! Tests for `write_oracle_meta` (Ix 23): the companion `[b"oracle_meta", oracle]`
//! PDA holding the plaintext subject + option labels + uri/uri_hash. Write-once,
//! sized to fit, gated to the oracle's creator.

mod common;
use common::*;

use kassandra_oracles_program::error::KassandraError;
use solana_pubkey::Pubkey;

/// Decode a LiteSVM transaction error into its `Custom(u32)` code, if any.
fn custom_code(res: &litesvm::types::TransactionResult) -> Option<u32> {
    use solana_instruction_error::InstructionError;
    use solana_transaction_error::TransactionError;
    match res {
        Err(meta) => match &meta.err {
            TransactionError::InstructionError(_, InstructionError::Custom(code)) => Some(*code),
            _ => None,
        },
        Ok(_) => None,
    }
}

/// Parse the `oracle_meta` account bytes → (oracle, subject, options, uri, uri_hash).
fn parse_meta(data: &[u8]) -> (Pubkey, String, Vec<String>, String, [u8; 32]) {
    assert_eq!(data[0], 8, "AccountType::OracleMeta tag");
    let oracle = Pubkey::new_from_array(data[2..34].try_into().unwrap());
    let mut off = 34usize;
    let read_u16 = |d: &[u8], off: &mut usize| -> usize {
        let v = u16::from_le_bytes(d[*off..*off + 2].try_into().unwrap()) as usize;
        *off += 2;
        v
    };
    let subject_len = read_u16(data, &mut off);
    let subject = String::from_utf8(data[off..off + subject_len].to_vec()).unwrap();
    off += subject_len;
    let options_count = data[off];
    off += 1;
    let mut options = Vec::new();
    for _ in 0..options_count {
        let ol = read_u16(data, &mut off);
        options.push(String::from_utf8(data[off..off + ol].to_vec()).unwrap());
        off += ol;
    }
    let uri_len = read_u16(data, &mut off);
    let uri = String::from_utf8(data[off..off + uri_len].to_vec()).unwrap();
    off += uri_len;
    let uri_hash: [u8; 32] = data[off..off + 32].try_into().unwrap();
    assert_eq!(off + 32, data.len(), "no trailing bytes");
    (oracle, subject, options, uri, uri_hash)
}

fn seed_oracle(ctx: &mut TestCtx, nonce: u64, options_count: u8) -> Pubkey {
    let _ = ctx.init_protocol();
    let deadline = ctx.now() + 1_000;
    let (oracle, res) = ctx.create_oracle(nonce, options_count, deadline, 600);
    assert!(res.is_ok(), "create_oracle should succeed: {res:?}");
    oracle
}

#[test]
fn writes_and_reads_back() {
    let mut ctx = TestCtx::new();
    let oracle = seed_oracle(&mut ctx, 7, 3);

    let subject = "Which team wins the final?";
    let options = ["Red", "Blue", "Draw"];
    let uri = "https://app.example/api/oracle/x/metadata.json";
    let uri_hash = [0xABu8; 32];
    let res = ctx.write_oracle_meta(oracle, subject, &options, uri, uri_hash);
    assert!(res.is_ok(), "write_oracle_meta should succeed: {res:?}");

    let (meta_pda, _) = kassandra_oracles_sdk::pda::oracle_meta(&ctx.program_id, &oracle);
    let data = ctx
        .svm
        .get_account(&meta_pda)
        .expect("oracle_meta account")
        .data;
    let (back_oracle, back_subject, back_options, back_uri, back_hash) = parse_meta(&data);

    assert_eq!(back_oracle, oracle);
    assert_eq!(back_subject, subject);
    assert_eq!(back_options, options);
    assert_eq!(back_uri, uri);
    assert_eq!(back_hash, uri_hash);
}

#[test]
fn write_once() {
    let mut ctx = TestCtx::new();
    let oracle = seed_oracle(&mut ctx, 1, 2);

    let r1 = ctx.write_oracle_meta(oracle, "Q?", &["Yes", "No"], "", [0u8; 32]);
    assert!(r1.is_ok(), "first write should succeed: {r1:?}");

    let r2 = ctx.write_oracle_meta(oracle, "Q2?", &["Yes", "No"], "", [0u8; 32]);
    assert_eq!(
        custom_code(&r2),
        Some(KassandraError::AlreadyInitialized as u32),
        "second write must fail AlreadyInitialized: {r2:?}"
    );
}

#[test]
fn options_count_must_match_oracle() {
    let mut ctx = TestCtx::new();
    let oracle = seed_oracle(&mut ctx, 2, 2); // oracle declares 2 options

    // ...but the metadata provides 3 labels.
    let res = ctx.write_oracle_meta(oracle, "Q?", &["A", "B", "C"], "", [0u8; 32]);
    assert_eq!(
        custom_code(&res),
        Some(KassandraError::InvalidOptionsCount as u32),
        "label count != oracle.options_count must fail InvalidOptionsCount: {res:?}"
    );
}

#[test]
fn empty_uri_is_allowed() {
    let mut ctx = TestCtx::new();
    let oracle = seed_oracle(&mut ctx, 3, 2);
    let res = ctx.write_oracle_meta(oracle, "No-uri question", &["Yes", "No"], "", [0u8; 32]);
    assert!(res.is_ok(), "empty uri should be allowed: {res:?}");
    let (meta_pda, _) = kassandra_oracles_sdk::pda::oracle_meta(&ctx.program_id, &oracle);
    let data = ctx.svm.get_account(&meta_pda).unwrap().data;
    let (_, _, _, uri, _) = parse_meta(&data);
    assert_eq!(uri, "");
}

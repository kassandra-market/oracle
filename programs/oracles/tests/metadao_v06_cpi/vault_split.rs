use super::*;

use solana_instruction::AccountMeta;

// ─────────────────────────────────────────────────────────────────────────────
// 3. Full real-binary CPI: conditional_vault split against the v0.6 vault binary.
//    (The v0.6 vault is the same deployed program as v0.4; this re-proves the
//    wire format against the freshly-dumped v0.6 fixture.)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn v06_conditional_vault_split() {
    use kassandra_oracles_program::cpi::metadao as md4; // shared vault discriminators/seeds

    let mut svm = LiteSVM::new();
    load_all(&mut svm);

    let payer = Keypair::new();
    svm.airdrop(&payer.pubkey(), 10_000_000_000).unwrap();

    let kass = fabricate_mint(&mut svm, 9, payer.pubkey());

    let underlying_amount: u64 = 5_000_000_000;
    let user_underlying = ata(&payer.pubkey(), &kass);
    fabricate_token_account(
        &mut svm,
        user_underlying,
        kass,
        payer.pubkey(),
        underlying_amount,
    );

    let num_outcomes: u8 = 2;
    let question_id = [7u8; 32];
    let resolver = Pubkey::new_unique();
    let resolver_pk = resolver.to_bytes();

    let kass_arr = kass.to_bytes();
    let (question, _) = Pubkey::find_program_address(
        &md4::question_seeds(&question_id, &resolver_pk.into(), &[num_outcomes]),
        &vault_id(),
    );
    let question_arr = question.to_bytes();
    let (vault, _) = Pubkey::find_program_address(
        &md4::vault_seeds(&question_arr.into(), &kass_arr.into()),
        &vault_id(),
    );
    let vault_arr = vault.to_bytes();
    let (cond0, _) = Pubkey::find_program_address(
        &md4::conditional_token_mint_seeds(&vault_arr.into(), &[0u8]),
        &vault_id(),
    );
    let (cond1, _) = Pubkey::find_program_address(
        &md4::conditional_token_mint_seeds(&vault_arr.into(), &[1u8]),
        &vault_id(),
    );
    let (event_authority, _) =
        Pubkey::find_program_address(&md4::event_authority_seeds(), &vault_id());

    let vault_underlying = ata(&vault, &kass);

    let ix_q = Instruction {
        program_id: vault_id(),
        accounts: vec![
            AccountMeta::new(question, false),
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new_readonly(solana_sdk_ids::system_program::ID, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(vault_id(), false),
        ],
        data: md4::initialize_question_data(&question_id, &resolver_pk.into(), num_outcomes)
            .to_vec(),
    };
    send(&mut svm, &payer, &[ix_q]).expect("initialize_question failed");

    let ix_v = Instruction {
        program_id: vault_id(),
        accounts: vec![
            AccountMeta::new(vault, false),
            AccountMeta::new_readonly(question, false),
            AccountMeta::new_readonly(kass, false),
            AccountMeta::new(vault_underlying, false),
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(ATA_PROGRAM_ID, false),
            AccountMeta::new_readonly(solana_sdk_ids::system_program::ID, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new(cond0, false),
            AccountMeta::new(cond1, false),
        ],
        data: md4::initialize_conditional_vault_data().to_vec(),
    };
    send(&mut svm, &payer, &[ix_v]).expect("initialize_conditional_vault failed");

    let user_cond0 = ata(&payer.pubkey(), &cond0);
    let user_cond1 = ata(&payer.pubkey(), &cond1);
    fabricate_token_account(&mut svm, user_cond0, cond0, payer.pubkey(), 0);
    fabricate_token_account(&mut svm, user_cond1, cond1, payer.pubkey(), 0);

    let split_amount: u64 = 2_000_000_000;
    let ix_s = Instruction {
        program_id: vault_id(),
        accounts: vec![
            AccountMeta::new_readonly(question, false),
            AccountMeta::new(vault, false),
            AccountMeta::new(vault_underlying, false),
            AccountMeta::new_readonly(payer.pubkey(), true),
            AccountMeta::new(user_underlying, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new(cond0, false),
            AccountMeta::new(cond1, false),
            AccountMeta::new(user_cond0, false),
            AccountMeta::new(user_cond1, false),
        ],
        data: md4::split_tokens_data(split_amount).to_vec(),
    };
    send(&mut svm, &payer, &[ix_s]).expect("split_tokens failed");

    assert_eq!(token_balance(&svm, user_cond0), split_amount);
    assert_eq!(token_balance(&svm, user_cond1), split_amount);
    assert_eq!(token_balance(&svm, vault_underlying), split_amount);
    assert_eq!(
        token_balance(&svm, user_underlying),
        underlying_amount - split_amount
    );
}

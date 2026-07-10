//! Account-meta goldens for the 8 MetaDAO builders sdks/oracles/rust exposes.

use super::{labeled, pk, with_programs, Meta};
use kassandra_markets_sdk::metadao as md;
use solana_sdk::pubkey::Pubkey;

#[test]
fn golden_md_create_amm() {
    let (payer, base_mint, quote_mint) = (pk(1), pk(40), pk(41));
    let ix = md::create_amm(&payer, &base_mint, &quote_mint, 1, 2, 0);
    let (amm, _) = md::amm(&base_mint, &quote_mint);
    let (lp_mint, _) = md::amm_lp_mint(&amm);
    let (event_auth, _) = md::event_authority(&md::AMM_ID);
    assert_eq!(
        labeled(
            &ix,
            with_programs(vec![
                (payer, "payer"),
                (amm, "amm"),
                (lp_mint, "lpMint"),
                (base_mint, "baseMint"),
                (quote_mint, "quoteMint"),
                (md::ata(&amm, &base_mint), "ammVaultBase"),
                (md::ata(&amm, &quote_mint), "ammVaultQuote"),
                (event_auth, "ammEventAuthority"),
            ]),
        ),
        vec![
            ("payer", true, true),
            ("amm", false, true),
            ("lpMint", false, true),
            ("baseMint", false, false),
            ("quoteMint", false, false),
            ("ammVaultBase", false, true),
            ("ammVaultQuote", false, true),
            ("ataProgram", false, false),
            ("tokenProgram", false, false),
            ("systemProgram", false, false),
            ("ammEventAuthority", false, false),
            ("ammProgram", false, false),
        ],
    );
}

#[test]
fn golden_md_add_liquidity() {
    let (payer, base_mint, quote_mint) = (pk(1), pk(40), pk(41));
    let (user_lp, user_base, user_quote) = (pk(43), pk(48), pk(49));
    let ix = md::add_liquidity(
        &payer,
        &base_mint,
        &quote_mint,
        &user_lp,
        &user_base,
        &user_quote,
        1,
        2,
        3,
    );
    let (amm, _) = md::amm(&base_mint, &quote_mint);
    let (lp_mint, _) = md::amm_lp_mint(&amm);
    let (event_auth, _) = md::event_authority(&md::AMM_ID);
    assert_eq!(
        labeled(
            &ix,
            with_programs(vec![
                (payer, "payer"),
                (amm, "amm"),
                (lp_mint, "lpMint"),
                (user_lp, "userLp"),
                (user_base, "userBase"),
                (user_quote, "userQuote"),
                (md::ata(&amm, &base_mint), "ammVaultBase"),
                (md::ata(&amm, &quote_mint), "ammVaultQuote"),
                (event_auth, "ammEventAuthority"),
            ]),
        ),
        vec![
            ("payer", true, true),
            ("amm", false, true),
            ("lpMint", false, true),
            ("userLp", false, true),
            ("userBase", false, true),
            ("userQuote", false, true),
            ("ammVaultBase", false, true),
            ("ammVaultQuote", false, true),
            ("tokenProgram", false, false),
            ("ammEventAuthority", false, false),
            ("ammProgram", false, false),
        ],
    );
}

#[test]
fn golden_md_swap() {
    let (payer, base_mint, quote_mint) = (pk(1), pk(40), pk(41));
    let (user_base, user_quote) = (pk(48), pk(49));
    let ix = md::swap(
        &payer,
        &base_mint,
        &quote_mint,
        &user_base,
        &user_quote,
        md::SwapType::Buy,
        10,
        0,
    );
    let (amm, _) = md::amm(&base_mint, &quote_mint);
    let (event_auth, _) = md::event_authority(&md::AMM_ID);
    assert_eq!(
        labeled(
            &ix,
            with_programs(vec![
                (payer, "payer"),
                (amm, "amm"),
                (user_base, "userBase"),
                (user_quote, "userQuote"),
                (md::ata(&amm, &base_mint), "ammVaultBase"),
                (md::ata(&amm, &quote_mint), "ammVaultQuote"),
                (event_auth, "ammEventAuthority"),
            ]),
        ),
        vec![
            ("payer", true, true),
            ("amm", false, true),
            ("userBase", false, true),
            ("userQuote", false, true),
            ("ammVaultBase", false, true),
            ("ammVaultQuote", false, true),
            ("tokenProgram", false, false),
            ("ammEventAuthority", false, false),
            ("ammProgram", false, false),
        ],
    );
}

#[test]
fn golden_md_initialize_question() {
    let (payer, oracle) = (pk(1), pk(4));
    let question_id = [47u8; 32];
    let ix = md::initialize_question(&payer, &oracle, &question_id, 2);
    let (question, _) = md::question(&question_id, &oracle, 2);
    let (event_auth, _) = md::event_authority(&md::CONDITIONAL_VAULT_ID);
    assert_eq!(
        labeled(
            &ix,
            with_programs(vec![
                (question, "question"),
                (payer, "payer"),
                (event_auth, "cvEventAuthority"),
            ]),
        ),
        vec![
            ("question", false, true),
            ("payer", true, true),
            ("systemProgram", false, false),
            ("cvEventAuthority", false, false),
            ("cvProgram", false, false),
        ],
    );
}

#[test]
fn golden_md_initialize_conditional_vault() {
    let (payer, question, underlying_mint) = (pk(1), pk(10), pk(42));
    let ix = md::initialize_conditional_vault(&payer, &question, &underlying_mint, 2);
    let (vault, _) = md::vault(&question, &underlying_mint);
    let vault_underlying_ata = md::ata(&vault, &underlying_mint);
    let (event_auth, _) = md::event_authority(&md::CONDITIONAL_VAULT_ID);
    let (cond_mint0, _) = md::conditional_token_mint(&vault, 0);
    let (cond_mint1, _) = md::conditional_token_mint(&vault, 1);
    assert_eq!(
        labeled(
            &ix,
            with_programs(vec![
                (vault, "vault"),
                (question, "question"),
                (underlying_mint, "underlyingMint"),
                (vault_underlying_ata, "vaultUnderlyingAta"),
                (payer, "payer"),
                (event_auth, "cvEventAuthority"),
                (cond_mint0, "condMint0"),
                (cond_mint1, "condMint1"),
            ]),
        ),
        vec![
            ("vault", false, true),
            ("question", false, false),
            ("underlyingMint", false, false),
            ("vaultUnderlyingAta", false, true),
            ("payer", true, true),
            ("tokenProgram", false, false),
            ("ataProgram", false, false),
            ("systemProgram", false, false),
            ("cvEventAuthority", false, false),
            ("cvProgram", false, false),
            ("condMint0", false, true),
            ("condMint1", false, true),
        ],
    );
}

/// The shared `InteractWithVault` golden (split/merge/redeem share this list).
/// `authority` is a READONLY signer (matches the program's split/redeem CPI metas
/// and the TS SDK — the fixed drift).
fn interact_golden() -> Vec<Meta> {
    vec![
        ("question", false, false),
        ("vault", false, true),
        ("vaultUnderlyingAta", false, true),
        ("authority", true, false),
        ("userUnderlyingAta", false, true),
        ("tokenProgram", false, false),
        ("cvEventAuthority", false, false),
        ("cvProgram", false, false),
        ("condMint0", false, true),
        ("condMint1", false, true),
        ("userCond0", false, true),
        ("userCond1", false, true),
    ]
}

#[allow(clippy::too_many_arguments)]
fn interact_entries(
    question: Pubkey,
    vault: Pubkey,
    vault_underlying_ata: Pubkey,
    authority: Pubkey,
    user_underlying_ata: Pubkey,
    yes_mint: Pubkey,
    no_mint: Pubkey,
    user_yes: Pubkey,
    user_no: Pubkey,
) -> Vec<(Pubkey, &'static str)> {
    let (event_auth, _) = md::event_authority(&md::CONDITIONAL_VAULT_ID);
    with_programs(vec![
        (question, "question"),
        (vault, "vault"),
        (vault_underlying_ata, "vaultUnderlyingAta"),
        (authority, "authority"),
        (user_underlying_ata, "userUnderlyingAta"),
        (event_auth, "cvEventAuthority"),
        (yes_mint, "condMint0"),
        (no_mint, "condMint1"),
        (user_yes, "userCond0"),
        (user_no, "userCond1"),
    ])
}

#[test]
fn golden_md_split_tokens() {
    let (auth, q, v, vua, uua) = (pk(3), pk(10), pk(12), pk(13), pk(44));
    let (ym, nm, uy, un) = (pk(14), pk(15), pk(45), pk(46));
    let ix = md::split_tokens(&auth, &q, &v, &vua, &uua, &ym, &nm, &uy, &un, 1);
    assert_eq!(
        labeled(&ix, interact_entries(q, v, vua, auth, uua, ym, nm, uy, un)),
        interact_golden(),
    );
}

#[test]
fn golden_md_merge_tokens() {
    let (auth, q, v, vua, uua) = (pk(3), pk(10), pk(12), pk(13), pk(44));
    let (ym, nm, uy, un) = (pk(14), pk(15), pk(45), pk(46));
    let ix = md::merge_tokens(&auth, &q, &v, &vua, &uua, &ym, &nm, &uy, &un, 1);
    assert_eq!(
        labeled(&ix, interact_entries(q, v, vua, auth, uua, ym, nm, uy, un)),
        interact_golden(),
    );
}

#[test]
fn golden_md_redeem_tokens() {
    let (auth, q, v, vua, uua) = (pk(3), pk(10), pk(12), pk(13), pk(44));
    let (ym, nm, uy, un) = (pk(14), pk(15), pk(45), pk(46));
    let ix = md::redeem_tokens(&auth, &q, &v, &vua, &uua, &ym, &nm, &uy, &un);
    assert_eq!(
        labeled(&ix, interact_entries(q, v, vua, auth, uua, ym, nm, uy, un)),
        interact_golden(),
    );
}

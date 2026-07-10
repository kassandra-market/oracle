//! Account-meta goldens for the 11 kassandra-market instructions.

use super::{labeled, pk, with_programs};
use kassandra_markets_sdk::{ix, metadao as md, pda, PROGRAM_ID};

#[test]
fn golden_init_config() {
    let (payer, kass_mint, authority, fee_dest) = (pk(1), pk(2), pk(3), pk(11));
    let ix = ix::init_config(&payer, &kass_mint, &authority, 5, 250, &fee_dest);
    let (config, _) = pda::config();
    let (program_data, _) = pda::program_data(&PROGRAM_ID);
    assert_eq!(
        labeled(
            &ix,
            with_programs(vec![
                (config, "config"),
                (payer, "payer"),
                (kass_mint, "kassMint"),
                (fee_dest, "feeDestination"),
                (program_data, "programData"),
            ]),
        ),
        vec![
            ("config", false, true),
            ("payer", true, true),
            ("kassMint", false, false),
            ("feeDestination", false, false),
            ("systemProgram", false, false),
            ("programData", false, false),
        ],
    );
}

#[test]
fn golden_update_config() {
    let (authority, fee_dest) = (pk(3), pk(11));
    let ix = ix::update_config(&authority, 42, 300, &fee_dest);
    let (config, _) = pda::config();
    assert_eq!(
        labeled(
            &ix,
            vec![
                (config, "config"),
                (authority, "authority"),
                (fee_dest, "feeDestination"),
            ],
        ),
        vec![
            ("config", false, true),
            ("authority", true, false),
            ("feeDestination", false, false),
        ],
    );
}

#[test]
fn golden_create_market() {
    let (creator, oracle, kass_mint, creator_ata) = (pk(5), pk(4), pk(2), pk(6));
    let ix = ix::create_market(&creator, &oracle, &kass_mint, &creator_ata, 1000, 0);
    let (config, _) = pda::config();
    let (market, _) = pda::market(&oracle, 0);
    let (escrow, _) = pda::escrow(&market);
    let (contribution, _) = pda::contribution(&market, &creator);
    assert_eq!(
        labeled(
            &ix,
            with_programs(vec![
                (config, "config"),
                (oracle, "oracle"),
                (market, "market"),
                (escrow, "escrow"),
                (kass_mint, "kassMint"),
                (creator, "creator"),
                (creator_ata, "creatorKassAta"),
                (contribution, "contribution"),
            ]),
        ),
        vec![
            ("config", false, false),
            ("oracle", false, false),
            ("market", false, true),
            ("escrow", false, true),
            ("kassMint", false, false),
            ("creator", true, true),
            ("creatorKassAta", false, true),
            ("contribution", false, true),
            ("tokenProgram", false, false),
            ("systemProgram", false, false),
        ],
    );
}

#[test]
fn golden_contribute() {
    let (contributor, contrib_ata) = (pk(7), pk(8));
    let (market, _) = pda::market(&pk(4), 0);
    let (escrow, _) = pda::escrow(&market);
    let ix = ix::contribute(&contributor, &market, &escrow, &contrib_ata, 777);
    let (contribution, _) = pda::contribution(&market, &contributor);
    assert_eq!(
        labeled(
            &ix,
            with_programs(vec![
                (market, "market"),
                (escrow, "escrow"),
                (contributor, "contributor"),
                (contrib_ata, "contributorKassAta"),
                (contribution, "contribution"),
            ]),
        ),
        vec![
            ("market", false, true),
            ("escrow", false, true),
            ("contributor", true, true),
            ("contributorKassAta", false, true),
            ("contribution", false, true),
            ("tokenProgram", false, false),
            ("systemProgram", false, false),
        ],
    );
}

#[test]
fn golden_cancel() {
    let (market, _) = pda::market(&pk(4), 0);
    let oracle = pk(4);
    let ix = ix::cancel(&market, &oracle);
    assert_eq!(
        labeled(&ix, vec![(market, "market"), (oracle, "oracle")]),
        vec![("market", false, true), ("oracle", false, false)],
    );
}

#[test]
fn golden_refund() {
    let (market, _) = pda::market(&pk(4), 0);
    let (escrow, _) = pda::escrow(&market);
    let (contribution, _) = pda::contribution(&market, &pk(7));
    let (contrib_ata, contributor) = (pk(8), pk(7));
    let ix = ix::refund(&market, &escrow, &contribution, &contrib_ata, &contributor);
    assert_eq!(
        labeled(
            &ix,
            with_programs(vec![
                (market, "market"),
                (escrow, "escrow"),
                (contribution, "contribution"),
                (contrib_ata, "contributorKassAta"),
                (contributor, "contributor"),
            ]),
        ),
        vec![
            ("market", false, true),
            ("escrow", false, true),
            ("contribution", false, true),
            ("contributorKassAta", false, true),
            ("contributor", false, true),
            ("tokenProgram", false, false),
        ],
    );
}

#[test]
fn golden_activate() {
    let (payer, oracle, kass_mint) = (pk(1), pk(4), pk(2));
    let ix = ix::activate(&payer, &oracle, &kass_mint, 0);
    let (market, _) = pda::market(&oracle, 0);
    let (escrow, _) = pda::escrow(&market);
    let (question, _) = md::question(&oracle.to_bytes(), &market, 2);
    let (vault, _) = md::vault(&question, &kass_mint);
    let vault_underlying_ata = md::ata(&vault, &kass_mint);
    let (yes_mint, _) = md::conditional_token_mint(&vault, 0);
    let (no_mint, _) = md::conditional_token_mint(&vault, 1);
    let (market_cyes, _) = pda::market_cyes(&market);
    let (market_cno, _) = pda::market_cno(&market);
    let (amm, _) = md::amm(&yes_mint, &no_mint);
    let (lp_mint, _) = md::amm_lp_mint(&amm);
    let (lp_vault, _) = pda::lp_vault(&market);
    let amm_vault_base = md::ata(&amm, &yes_mint);
    let amm_vault_quote = md::ata(&amm, &no_mint);
    let (cv_event_auth, _) = md::event_authority(&md::CONDITIONAL_VAULT_ID);
    let (amm_event_auth, _) = md::event_authority(&md::AMM_ID);
    assert_eq!(
        labeled(
            &ix,
            with_programs(vec![
                (market, "market"),
                (oracle, "oracle"),
                (payer, "payer"),
                (question, "question"),
                (vault, "vault"),
                (vault_underlying_ata, "vaultUnderlyingAta"),
                (escrow, "escrow"),
                (yes_mint, "yesMint"),
                (no_mint, "noMint"),
                (market_cyes, "marketCyes"),
                (market_cno, "marketCno"),
                (amm, "amm"),
                (lp_mint, "lpMint"),
                (lp_vault, "lpVault"),
                (amm_vault_base, "ammVaultBase"),
                (amm_vault_quote, "ammVaultQuote"),
                (cv_event_auth, "cvEventAuthority"),
                (amm_event_auth, "ammEventAuthority"),
            ]),
        ),
        vec![
            ("market", false, true),
            ("oracle", false, false),
            ("payer", true, true),
            ("question", false, false),
            ("vault", false, true),
            ("vaultUnderlyingAta", false, true),
            ("escrow", false, true),
            ("yesMint", false, true),
            ("noMint", false, true),
            ("marketCyes", false, true),
            ("marketCno", false, true),
            ("amm", false, true),
            ("lpMint", false, true),
            ("lpVault", false, true),
            ("ammVaultBase", false, true),
            ("ammVaultQuote", false, true),
            ("cvEventAuthority", false, false),
            ("cvProgram", false, false),
            ("ammEventAuthority", false, false),
            ("ammProgram", false, false),
            ("tokenProgram", false, false),
            ("systemProgram", false, false),
        ],
    );
}

#[test]
fn golden_claim_lp() {
    let (market, _) = pda::market(&pk(4), 0);
    let (lp_vault, _) = pda::lp_vault(&market);
    let (contribution, _) = pda::contribution(&market, &pk(7));
    let (lp_ata, contributor) = (pk(9), pk(7));
    let ix = ix::claim_lp(&market, &lp_vault, &contribution, &lp_ata, &contributor);
    assert_eq!(
        labeled(
            &ix,
            with_programs(vec![
                (market, "market"),
                (lp_vault, "lpVault"),
                (contribution, "contribution"),
                (lp_ata, "contributorLpAta"),
                (contributor, "contributor"),
            ]),
        ),
        vec![
            ("market", false, true),
            ("lpVault", false, true),
            ("contribution", false, true),
            ("contributorLpAta", false, true),
            ("contributor", false, true),
            ("tokenProgram", false, false),
        ],
    );
}

#[test]
fn golden_resolve_market() {
    let (market, _) = pda::market(&pk(4), 0);
    let (oracle, question, cv_event_auth) = (pk(4), pk(10), pk(23));
    let ix = ix::resolve_market(&market, &oracle, &question, &cv_event_auth);
    assert_eq!(
        labeled(
            &ix,
            with_programs(vec![
                (market, "market"),
                (oracle, "oracle"),
                (question, "question"),
                (cv_event_auth, "cvEventAuthority"),
            ]),
        ),
        vec![
            ("market", false, true),
            ("oracle", false, false),
            ("question", false, true),
            ("cvEventAuthority", false, false),
            ("cvProgram", false, false),
        ],
    );
}

#[test]
fn golden_collect_fee() {
    let (oracle, kass_mint, fee_dest) = (pk(4), pk(2), pk(11));
    let ix = ix::collect_fee(&oracle, &kass_mint, &fee_dest, 0);
    let (config, _) = pda::config();
    let (market, _) = pda::market(&oracle, 0);
    let (escrow, _) = pda::escrow(&market);
    let (question, _) = md::question(&oracle.to_bytes(), &market, 2);
    let (vault, _) = md::vault(&question, &kass_mint);
    let vault_underlying_ata = md::ata(&vault, &kass_mint);
    let (yes_mint, _) = md::conditional_token_mint(&vault, 0);
    let (no_mint, _) = md::conditional_token_mint(&vault, 1);
    let (market_cyes, _) = pda::market_cyes(&market);
    let (market_cno, _) = pda::market_cno(&market);
    let (amm, _) = md::amm(&yes_mint, &no_mint);
    let (lp_mint, _) = md::amm_lp_mint(&amm);
    let (lp_vault, _) = pda::lp_vault(&market);
    let amm_vault_base = md::ata(&amm, &yes_mint);
    let amm_vault_quote = md::ata(&amm, &no_mint);
    let (cv_event_auth, _) = md::event_authority(&md::CONDITIONAL_VAULT_ID);
    let (amm_event_auth, _) = md::event_authority(&md::AMM_ID);
    assert_eq!(
        labeled(
            &ix,
            with_programs(vec![
                (market, "market"),
                (config, "config"),
                (fee_dest, "feeDestination"),
                (question, "question"),
                (vault, "vault"),
                (vault_underlying_ata, "vaultUnderlyingAta"),
                (escrow, "escrow"),
                (yes_mint, "yesMint"),
                (no_mint, "noMint"),
                (market_cyes, "marketCyes"),
                (market_cno, "marketCno"),
                (amm, "amm"),
                (lp_mint, "lpMint"),
                (lp_vault, "lpVault"),
                (amm_vault_base, "ammVaultBase"),
                (amm_vault_quote, "ammVaultQuote"),
                (cv_event_auth, "cvEventAuthority"),
                (amm_event_auth, "ammEventAuthority"),
            ]),
        ),
        vec![
            ("market", false, true),
            ("config", false, false),
            ("feeDestination", false, true),
            ("question", false, false),
            ("vault", false, true),
            ("vaultUnderlyingAta", false, true),
            ("escrow", false, true),
            ("yesMint", false, true),
            ("noMint", false, true),
            ("marketCyes", false, true),
            ("marketCno", false, true),
            ("amm", false, true),
            ("lpMint", false, true),
            ("lpVault", false, true),
            ("ammVaultBase", false, true),
            ("ammVaultQuote", false, true),
            ("cvEventAuthority", false, false),
            ("cvProgram", false, false),
            ("ammEventAuthority", false, false),
            ("ammProgram", false, false),
            ("tokenProgram", false, false),
        ],
    );
}

#[test]
fn golden_close_market() {
    let (oracle, creator) = (pk(4), pk(5));
    let ix = ix::close_market(&oracle, &creator, 0);
    let (market, _) = pda::market(&oracle, 0);
    let (escrow, _) = pda::escrow(&market);
    let (cyes, _) = pda::market_cyes(&market);
    let (cno, _) = pda::market_cno(&market);
    let (lp_vault, _) = pda::lp_vault(&market);
    assert_eq!(
        labeled(
            &ix,
            with_programs(vec![
                (market, "market"),
                (creator, "creator"),
                (escrow, "escrow"),
                (cyes, "cyes"),
                (cno, "cno"),
                (lp_vault, "lpVault"),
            ]),
        ),
        vec![
            ("market", false, true),
            ("creator", false, true),
            ("escrow", false, true),
            ("cyes", false, true),
            ("cno", false, true),
            ("lpVault", false, true),
            ("tokenProgram", false, false),
        ],
    );
}

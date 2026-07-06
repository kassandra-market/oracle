//! Account-meta golden guard — pins each instruction's ACCOUNT CONTRACT.
//!
//! The `tests/parity.rs` guard locks discriminators, sizes, and payload bytes,
//! but a wrong account ORDER or a flipped `is_signer`/`is_writable` flag would
//! still slip past CI. This file closes that gap: for every `ix::*` builder AND
//! every `metadao::*` account builder, it builds the instruction with DISTINCT
//! placeholder `Pubkey`s (deriving every PDA the builder derives), then asserts
//! the resulting `Vec<(role, is_signer, is_writable)>` equals a HARDCODED literal
//! golden — a hand-written frozen snapshot, NOT computed from the builder.
//!
//! Cross-checked against the program processors' `let [a, b, ..]` destructures
//! (`programs/kassandra-market/src/processor/*.rs`) and the CPI metas
//! (`activate.rs` split_metas, `collect_fee.rs` redeem_metas). Where the two SDKs
//! disagreed the PROGRAM wins; the labels + order here are IDENTICAL to the TS
//! golden in `sdk/test/account-metas.test.ts`, so both SDKs encode ONE contract.
//! Any future account-order/flag drift in either SDK fails these tests.

use kassandra_market_sdk::{ix, metadao as md, pda, PROGRAM_ID};
use solana_sdk::{instruction::Instruction, pubkey::Pubkey};
use std::collections::HashMap;

/// A single golden row: role name + signer/writable flags.
type Meta = (&'static str, bool, bool);

/// Deterministic distinct placeholder pubkey (32 bytes of `n`).
fn pk(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}

/// Label each account meta by looking its pubkey up in `entries`. Panics on any
/// unmapped account so a builder that grew/moved a slot fails loudly.
fn labeled(ix: &Instruction, entries: Vec<(Pubkey, &'static str)>) -> Vec<Meta> {
    let map: HashMap<Pubkey, &'static str> = entries.into_iter().collect();
    ix.accounts
        .iter()
        .map(|m| {
            let name = map
                .get(&m.pubkey)
                .unwrap_or_else(|| panic!("unmapped account {} in {:?}", m.pubkey, ix.program_id));
            (*name, m.is_signer, m.is_writable)
        })
        .collect()
}

/// The fixed program-id accounts, by role. Spread into per-instruction maps.
fn programs() -> Vec<(Pubkey, &'static str)> {
    vec![
        (solana_sdk::system_program::id(), "systemProgram"),
        (spl_token::id(), "tokenProgram"),
        (md::ASSOCIATED_TOKEN_PROGRAM_ID, "ataProgram"),
        (md::CONDITIONAL_VAULT_ID, "cvProgram"),
        (md::AMM_ID, "ammProgram"),
    ]
}

fn with_programs(mut entries: Vec<(Pubkey, &'static str)>) -> Vec<(Pubkey, &'static str)> {
    entries.extend(programs());
    entries
}

// ── kassandra-market (11 instructions) ────────────────────────────────────────

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

// ── MetaDAO builders (8 — the ones sdk-rs exposes) ────────────────────────────

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

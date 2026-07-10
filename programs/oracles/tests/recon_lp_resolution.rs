//! RECON SPIKE (Task C0) — exploratory, NOT production behavior.
//!
//! Drives the ONE piece of the conditional-AMM LP lifecycle that the existing
//! `settle_challenge.rs` tests do NOT yet exercise against the real deployed
//! MetaDAO `amm` v0.4 binary: `remove_liquidity`. It empirically confirms the
//! IMPERMANENT-LOSS crux from the v0.4 source
//! (`amm/src/state/amm.rs::get_base_and_quote_withdrawable`): an LP who removes
//! after trading has shifted the reserves gets back a PRICE-DEPENDENT mix of
//! (base, quote) — pro-rata the pool's reserves AT REMOVAL, not what was
//! deposited. This is the mechanic that collides with a bond being a clean
//! slashable KASS quantity.
//!
//! `create_amm` + `add_liquidity` + `swap` + `crank_that_twap` are already
//! proven against the same binary in `settle_challenge.rs`; `redeem_tokens`
//! (winning side 1:1, losing side -> 0) is fully explicit in the conditional
//! vault source and reasoned about in the recon doc. This file pins the
//! remove-liquidity half so the doc's net-flow trace rests on observed binary
//! behavior, not just source reading.
//!
//! See `docs/plans/2026-06-29-challenge-rework-recon.md`.
//!
//! KEPT after C3 (deliberately, not folded in): this is the ONLY coverage of
//! `remove_liquidity` against the real AMM binary, and it pins the IMPERMANENT-
//! LOSS finding that MOTIVATED the escrow/idle-bond design — the bond is split
//! into idle conditional KASS and redeemed (winning side 1:1) rather than LP'd,
//! so it round-trips cleanly. That idle-bond path is now driven end-to-end by
//! `challenge_e2e.rs` (real open → AMM → settle, both outcomes) +
//! `settle_challenge.rs`; this file remains the empirical "why not LP the bond"
//! record and the lone real-binary `remove_liquidity` check.

mod common;
use common::*;

use kassandra_oracles_program::cpi::metadao;
use solana_account::Account;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use spl_token::{
    solana_program::{program_option::COption, program_pack::Pack},
    state::{Account as TokenAccount, AccountState},
    ID as TOKEN_PROGRAM_ID,
};

const AMM_SO: &[u8] = include_bytes!("fixtures/metadao_amm.so");
const ATA_PROGRAM_ID: Pubkey =
    solana_pubkey::pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
const MAX_PRICE: u128 = (u64::MAX as u128) * 1_000_000_000_000;

fn amm_id() -> Pubkey {
    Pubkey::new_from_array(metadao::AMM_ID.to_bytes())
}

fn ata(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[owner.as_ref(), TOKEN_PROGRAM_ID.as_ref(), mint.as_ref()],
        &ATA_PROGRAM_ID,
    )
    .0
}

fn cu(ix: Instruction) -> [Instruction; 2] {
    [
        ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
        ix,
    ]
}

fn fabricate_token_account(
    ctx: &mut TestCtx,
    addr: Pubkey,
    mint: Pubkey,
    owner: Pubkey,
    amount: u64,
) {
    let state = TokenAccount {
        mint,
        owner,
        amount,
        delegate: COption::None,
        state: AccountState::Initialized,
        is_native: COption::None,
        delegated_amount: 0,
        close_authority: COption::None,
    };
    let mut data = vec![0u8; TokenAccount::LEN];
    state.pack_into_slice(&mut data);
    let lamports = ctx
        .svm
        .minimum_balance_for_rent_exemption(TokenAccount::LEN);
    ctx.svm
        .set_account(
            addr,
            Account {
                lamports,
                data,
                owner: TOKEN_PROGRAM_ID,
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();
}

/// Drive create_amm -> add_liquidity (sole LP) -> a price-shifting BUY ->
/// remove_liquidity (burn all LP), all against the REAL amm binary, and assert
/// the withdrawn (base, quote) is the pool's reserves AT REMOVAL (shifted by the
/// swap) — i.e. impermanent loss / a price-dependent mix, NOT the deposited
/// amounts.
#[test]
fn remove_liquidity_returns_price_dependent_mix() {
    let mut ctx = TestCtx::new();
    ctx.svm.add_program(amm_id(), AMM_SO).unwrap();

    let payer = ctx.payer.pubkey();
    let base_mint = ctx.kass_mint; // 9 dp
    let quote_mint = ctx.usdc_mint; // 6 dp

    // Initial deposit: 100 KASS base, 100 USDC quote (price 1.0 -> scaled 1e9).
    let base_reserve: u64 = 100_000_000_000;
    let quote_reserve: u64 = 100_000_000;

    let base_arr = base_mint.to_bytes();
    let quote_arr = quote_mint.to_bytes();
    let (amm, _) = Pubkey::find_program_address(
        &[b"amm__", base_arr.as_ref(), quote_arr.as_ref()],
        &amm_id(),
    );
    let amm_arr = amm.to_bytes();
    let (lp_mint, _) = Pubkey::find_program_address(&[b"amm_lp_mint", amm_arr.as_ref()], &amm_id());
    let vault_ata_base = ata(&amm, &base_mint);
    let vault_ata_quote = ata(&amm, &quote_mint);
    let (amm_event_auth, _) =
        Pubkey::find_program_address(&metadao::event_authority_seeds(), &amm_id());

    let user_base = ata(&payer, &base_mint);
    let user_quote = ata(&payer, &quote_mint);
    // Fund generously: enough base+quote for add and a follow-up buy.
    fabricate_token_account(&mut ctx, user_base, base_mint, payer, base_reserve * 4);
    fabricate_token_account(&mut ctx, user_quote, quote_mint, payer, quote_reserve * 10);

    // --- create_amm (delayed-twap v0.4.x args) ---
    let initial_obs: u128 = (quote_reserve as u128 * 1_000_000_000_000u128) / base_reserve as u128;
    let mut create_data = metadao::CREATE_AMM.to_vec();
    create_data.extend_from_slice(&initial_obs.to_le_bytes());
    create_data.extend_from_slice(&MAX_PRICE.to_le_bytes());
    create_data.extend_from_slice(&0u64.to_le_bytes());
    let ix_create = Instruction {
        program_id: amm_id(),
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(amm, false),
            AccountMeta::new(lp_mint, false),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new(vault_ata_base, false),
            AccountMeta::new(vault_ata_quote, false),
            AccountMeta::new_readonly(ATA_PROGRAM_ID, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(solana_sdk_ids::system_program::ID, false),
            AccountMeta::new_readonly(amm_event_auth, false),
            AccountMeta::new_readonly(amm_id(), false),
        ],
        data: create_data,
    };
    ctx.send_many(&cu(ix_create), &[]).expect("create_amm");

    let user_lp = ata(&payer, &lp_mint);
    fabricate_token_account(&mut ctx, user_lp, lp_mint, payer, 0);

    // --- add_liquidity: quote ++ max_base ++ min_lp ---
    let mut add_data = metadao::ADD_LIQUIDITY.to_vec();
    add_data.extend_from_slice(&quote_reserve.to_le_bytes());
    add_data.extend_from_slice(&base_reserve.to_le_bytes());
    add_data.extend_from_slice(&0u64.to_le_bytes());
    let add_remove_accounts = vec![
        AccountMeta::new(payer, true),
        AccountMeta::new(amm, false),
        AccountMeta::new(lp_mint, false),
        AccountMeta::new(user_lp, false),
        AccountMeta::new(user_base, false),
        AccountMeta::new(user_quote, false),
        AccountMeta::new(vault_ata_base, false),
        AccountMeta::new(vault_ata_quote, false),
        AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        AccountMeta::new_readonly(amm_event_auth, false),
        AccountMeta::new_readonly(amm_id(), false),
    ];
    let ix_add = Instruction {
        program_id: amm_id(),
        accounts: add_remove_accounts.clone(),
        data: add_data,
    };
    ctx.send_many(&cu(ix_add), &[]).expect("add_liquidity");

    // Sole LP: initial LP minted == quote_amount (v0.4 source).
    let lp_balance = ctx.token_balance(user_lp);
    assert_eq!(lp_balance, quote_reserve, "initial LP == quote deposited");
    assert_eq!(ctx.token_balance(vault_ata_base), base_reserve);
    assert_eq!(ctx.token_balance(vault_ata_quote), quote_reserve);

    // --- price-shifting BUY: quote in, base out -> base reserve down, quote up ---
    let buy_in: u64 = 50_000_000; // 50 USDC in
    let mut swap_data = metadao::SWAP.to_vec();
    swap_data.push(0u8); // SwapType::Buy
    swap_data.extend_from_slice(&buy_in.to_le_bytes());
    swap_data.extend_from_slice(&0u64.to_le_bytes());
    let ix_swap = Instruction {
        program_id: amm_id(),
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(amm, false),
            AccountMeta::new(user_base, false),
            AccountMeta::new(user_quote, false),
            AccountMeta::new(vault_ata_base, false),
            AccountMeta::new(vault_ata_quote, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(amm_event_auth, false),
            AccountMeta::new_readonly(amm_id(), false),
        ],
        data: swap_data,
    };
    ctx.warp_slots(0, 5);
    ctx.send_many(&cu(ix_swap), &[]).expect("swap buy");

    let pool_base_after_swap = ctx.token_balance(vault_ata_base);
    let pool_quote_after_swap = ctx.token_balance(vault_ata_quote);
    assert!(
        pool_base_after_swap < base_reserve,
        "buy drained base: {pool_base_after_swap} < {base_reserve}"
    );
    assert!(
        pool_quote_after_swap > quote_reserve,
        "buy added quote: {pool_quote_after_swap} > {quote_reserve}"
    );

    // --- remove_liquidity: burn ALL LP. Args = lp_to_burn ++ min_quote ++ min_base ---
    let mut rem_data = metadao::REMOVE_LIQUIDITY.to_vec();
    rem_data.extend_from_slice(&lp_balance.to_le_bytes());
    rem_data.extend_from_slice(&0u64.to_le_bytes());
    rem_data.extend_from_slice(&0u64.to_le_bytes());
    let ix_rem = Instruction {
        program_id: amm_id(),
        accounts: add_remove_accounts,
        data: rem_data,
    };
    let base_before_rem = ctx.token_balance(user_base);
    let quote_before_rem = ctx.token_balance(user_quote);
    ctx.send_many(&cu(ix_rem), &[]).expect("remove_liquidity");

    let withdrawn_base = ctx.token_balance(user_base) - base_before_rem;
    let withdrawn_quote = ctx.token_balance(user_quote) - quote_before_rem;

    // The sole LP burning 100% gets the pool's reserves AT REMOVAL — the
    // swap-shifted amounts, NOT the deposited (base_reserve, quote_reserve).
    assert_eq!(
        withdrawn_base, pool_base_after_swap,
        "withdrawn base == pool reserve at removal (pro-rata, 100% LP)"
    );
    assert_eq!(
        withdrawn_quote, pool_quote_after_swap,
        "withdrawn quote == pool reserve at removal"
    );

    // THE CRUX: an LP who deposited base_reserve KASS does NOT get base_reserve
    // KASS back. After a price-up swap they get LESS base and MORE quote — a
    // price-dependent mix. A bond deposited as this base liquidity is therefore
    // NOT cleanly recoverable as its original KASS quantity.
    assert!(
        withdrawn_base < base_reserve,
        "IL: got back LESS base than deposited ({withdrawn_base} < {base_reserve})"
    );
    assert!(
        withdrawn_quote > quote_reserve,
        "IL: got back MORE quote than deposited ({withdrawn_quote} > {quote_reserve})"
    );
}

//! Real AMM pool driving (create/add/swap/crank) + the `open_challenge` /
//! `settle_challenge` instruction builders and settle-side payout fixtures.

use super::support::*;
use super::*;

use super::support::ATA_PROGRAM_ID;
use kassandra_oracles_program::{cpi::metadao, instruction::Ix};
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use solana_sdk_ids::system_program;
use solana_signer::Signer;
use spl_token::ID as TOKEN_PROGRAM_ID;

// ---------------------------------------------------------------------------
// Real AMM pool driving (create + add, then swap / crank separately)
// ---------------------------------------------------------------------------

/// `create_amm` + `add_liquidity` (NO crank yet). Returns the AMM PDA; funds the
/// payer's base/quote generously (4× reserve) so later swaps have headroom.
pub(crate) fn build_pool(
    ctx: &mut TestCtx,
    base_mint: Pubkey,
    quote_mint: Pubkey,
    base_reserve: u64,
    quote_reserve: u64,
) -> Pubkey {
    let payer = ctx.payer.pubkey();
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
    fabricate_token_account(
        ctx,
        user_base,
        base_mint,
        payer,
        base_reserve.saturating_mul(4),
    );
    fabricate_token_account(
        ctx,
        user_quote,
        quote_mint,
        payer,
        quote_reserve.saturating_mul(4),
    );

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
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(amm_event_auth, false),
            AccountMeta::new_readonly(amm_id(), false),
        ],
        data: create_data,
    };
    ctx.send_many(&cu(ix_create), &[]).expect("create_amm");

    let user_lp = ata(&payer, &lp_mint);
    fabricate_token_account(ctx, user_lp, lp_mint, payer, 0);

    let mut add_data = metadao::ADD_LIQUIDITY.to_vec();
    add_data.extend_from_slice(&quote_reserve.to_le_bytes());
    add_data.extend_from_slice(&base_reserve.to_le_bytes());
    add_data.extend_from_slice(&0u64.to_le_bytes());
    let ix_add = Instruction {
        program_id: amm_id(),
        accounts: vec![
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
        ],
        data: add_data,
    };
    ctx.send_many(&cu(ix_add), &[]).expect("add_liquidity");
    amm
}

/// A genuine BUY (quote in, base out) that pushes the pool's price UP.
pub(crate) fn swap_buy(
    ctx: &mut TestCtx,
    amm: Pubkey,
    base_mint: Pubkey,
    quote_mint: Pubkey,
    amount_in: u64,
) {
    let payer = ctx.payer.pubkey();
    let user_base = ata(&payer, &base_mint);
    let user_quote = ata(&payer, &quote_mint);
    let vault_ata_base = ata(&amm, &base_mint);
    let vault_ata_quote = ata(&amm, &quote_mint);
    let (amm_event_auth, _) =
        Pubkey::find_program_address(&metadao::event_authority_seeds(), &amm_id());

    let mut swap_data = metadao::SWAP.to_vec();
    swap_data.push(0u8); // SwapType::Buy
    swap_data.extend_from_slice(&amount_in.to_le_bytes());
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
}

/// Advance ≥ ONE_MINUTE_IN_SLOTS (150) slots, then `crank_that_twap` once.
pub(crate) fn crank_pool(ctx: &mut TestCtx, amm: Pubkey) {
    ctx.warp_slots(0, 300);
    let (amm_event_auth, _) =
        Pubkey::find_program_address(&metadao::event_authority_seeds(), &amm_id());
    let ix_crank = Instruction {
        program_id: amm_id(),
        accounts: vec![
            AccountMeta::new(amm, false),
            AccountMeta::new_readonly(amm_event_auth, false),
            AccountMeta::new_readonly(amm_id(), false),
        ],
        data: metadao::CRANK_THAT_TWAP.to_vec(),
    };
    ctx.send_many(&cu(ix_crank), &[]).expect("crank_that_twap");
}

// ---------------------------------------------------------------------------
// open_challenge / settle_challenge instruction builders
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(crate) fn open_challenge_ix(
    ctx: &TestCtx,
    oracle: Pubkey,
    ai_claim: Pubkey,
    proposer: Pubkey,
    market: Pubkey,
    challenger: Pubkey,
    m: &MarketAccounts,
    pass_amm: Pubkey,
    fail_amm: Pubkey,
    stake_vault: Pubkey,
    oracle_pass_kass: Pubkey,
    oracle_fail_kass: Pubkey,
    kass_dao: Pubkey,
    challenger_usdc_src: Pubkey,
    nonce: u64,
) -> Instruction {
    let (cv_event_auth, _) =
        Pubkey::find_program_address(&metadao::event_authority_seeds(), &vault_id());
    let (protocol, _) = TestCtx::protocol_pda(&ctx.program_id);
    let (escrow_vault, _) = TestCtx::challenge_usdc_vault_pda(&ctx.program_id, &market);
    let mut data = vec![Ix::OpenChallenge as u8];
    data.extend_from_slice(&nonce.to_le_bytes());
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(ai_claim, false),
            AccountMeta::new(proposer, false),
            AccountMeta::new(market, false),
            AccountMeta::new(challenger, true),
            AccountMeta::new_readonly(m.question, false),
            AccountMeta::new(m.kass_vault, false),
            AccountMeta::new_readonly(m.usdc_vault, false),
            AccountMeta::new_readonly(pass_amm, false),
            AccountMeta::new_readonly(fail_amm, false),
            AccountMeta::new(stake_vault, false),
            AccountMeta::new(m.kass_vault_underlying, false),
            AccountMeta::new(m.pass_mint, false),
            AccountMeta::new(m.fail_mint, false),
            AccountMeta::new(oracle_pass_kass, false),
            AccountMeta::new(oracle_fail_kass, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(cv_event_auth, false),
            AccountMeta::new_readonly(protocol, false),
            AccountMeta::new_readonly(kass_dao, false),
            AccountMeta::new_readonly(ctx.usdc_mint, false),
            AccountMeta::new(challenger_usdc_src, false),
            AccountMeta::new(escrow_vault, false),
        ],
        data,
    }
}

pub(crate) struct SettleExtras {
    pub(crate) stake_vault: Pubkey,
    pub(crate) kass_vault: Pubkey,
    pub(crate) kass_vault_underlying: Pubkey,
    pub(crate) pass_mint: Pubkey,
    pub(crate) fail_mint: Pubkey,
    pub(crate) oracle_pass_kass: Pubkey,
    pub(crate) oracle_fail_kass: Pubkey,
    pub(crate) escrow_vault: Pubkey,
    pub(crate) proposer_usdc: Pubkey,
    pub(crate) challenger_usdc_dest: Pubkey,
    pub(crate) challenger_kass: Pubkey,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn settle_ix(
    ctx: &TestCtx,
    oracle: Pubkey,
    market: Pubkey,
    ai_claim: Pubkey,
    proposer: Pubkey,
    question: Pubkey,
    pass_amm: Pubkey,
    fail_amm: Pubkey,
    x: &SettleExtras,
    nonce: u64,
) -> Instruction {
    let (cv_event_auth, _) =
        Pubkey::find_program_address(&metadao::event_authority_seeds(), &vault_id());
    let mut data = vec![Ix::SettleChallenge as u8];
    data.extend_from_slice(&nonce.to_le_bytes());
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(market, false),
            AccountMeta::new_readonly(ai_claim, false),
            AccountMeta::new(proposer, false),
            AccountMeta::new(question, false),
            AccountMeta::new_readonly(pass_amm, false),
            AccountMeta::new_readonly(fail_amm, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new_readonly(cv_event_auth, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new(x.stake_vault, false),
            AccountMeta::new(x.kass_vault, false),
            AccountMeta::new(x.kass_vault_underlying, false),
            AccountMeta::new(x.pass_mint, false),
            AccountMeta::new(x.fail_mint, false),
            AccountMeta::new(x.oracle_pass_kass, false),
            AccountMeta::new(x.oracle_fail_kass, false),
            AccountMeta::new(x.escrow_vault, false),
            AccountMeta::new(x.proposer_usdc, false),
            AccountMeta::new(x.challenger_usdc_dest, false),
            AccountMeta::new(x.challenger_kass, false),
        ],
        data,
    }
}

fn read_u32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(data[off..off + 4].try_into().unwrap())
}

pub(crate) fn question_resolution(ctx: &TestCtx, question: Pubkey) -> (u32, u32, u32) {
    let acc = ctx.svm.get_account(&question).expect("question missing");
    (
        read_u32(&acc.data, 76),
        read_u32(&acc.data, 80),
        read_u32(&acc.data, 84),
    )
}

/// Settle-side payout destinations + the escrow vault, fabricated empty.
pub(crate) struct Payouts {
    pub(crate) escrow_vault: Pubkey,
    pub(crate) proposer_usdc: Pubkey,
    pub(crate) challenger_usdc_dest: Pubkey,
    pub(crate) challenger_kass: Pubkey,
}

pub(crate) fn fabricate_payouts(
    ctx: &mut TestCtx,
    market: Pubkey,
    proposer_authority: Pubkey,
    challenger: Pubkey,
) -> Payouts {
    let usdc = ctx.usdc_mint;
    let kass = ctx.kass_mint;
    let proposer_usdc = Pubkey::new_unique();
    let challenger_usdc_dest = Pubkey::new_unique();
    let challenger_kass = Pubkey::new_unique();
    fabricate_token_account(ctx, proposer_usdc, usdc, proposer_authority, 0);
    fabricate_token_account(ctx, challenger_usdc_dest, usdc, challenger, 0);
    fabricate_token_account(ctx, challenger_kass, kass, challenger, 0);
    let (escrow_vault, _) = TestCtx::challenge_usdc_vault_pda(&ctx.program_id, &market);
    Payouts {
        escrow_vault,
        proposer_usdc,
        challenger_usdc_dest,
        challenger_kass,
    }
}

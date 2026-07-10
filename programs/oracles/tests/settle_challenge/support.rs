//! Shared low-level fixtures for the `settle_challenge` suite: program ids,
//! ATA/token fabrication, and the real-binary MetaDAO market + AMM builders.
//! (Pure relocation from the crate root; `pub(crate)` is visibility glue only.)

use super::*;

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

const ATA_PROGRAM_ID: Pubkey =
    solana_pubkey::pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

/// Largest observation change per update — lets the recorded observation jump
/// straight to the pool's current price in a single crank, so a one-crank TWAP
/// equals the reserve-implied price (clean, deterministic test prices).
const MAX_PRICE: u128 = (u64::MAX as u128) * 1_000_000_000_000;

pub(crate) fn vault_id() -> Pubkey {
    Pubkey::new_from_array(metadao::CONDITIONAL_VAULT_ID.to_bytes())
}
pub(crate) fn amm_id() -> Pubkey {
    Pubkey::new_from_array(metadao::AMM_ID.to_bytes())
}

pub(crate) fn ata(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[owner.as_ref(), TOKEN_PROGRAM_ID.as_ref(), mint.as_ref()],
        &ATA_PROGRAM_ID,
    )
    .0
}

fn cond_mint(vault: &Pubkey, index: u8) -> Pubkey {
    Pubkey::find_program_address(
        &metadao::conditional_token_mint_seeds(&vault.to_bytes().into(), &[index]),
        &vault_id(),
    )
    .0
}

pub(crate) fn cu(ix: Instruction) -> [Instruction; 2] {
    [
        ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
        ix,
    ]
}

/// Fabricate a token account at `addr` holding `amount` of `mint`, owned by `owner`.
pub(crate) fn fabricate_token_account(
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

pub(crate) struct MarketAccounts {
    pub(crate) question: Pubkey,
    pub(crate) kass_vault: Pubkey,
    pub(crate) kass_vault_underlying: Pubkey,
    pub(crate) usdc_vault: Pubkey,
    pub(crate) pass_mint: Pubkey, // pass-KASS  (cond(kass_vault, 0))
    pub(crate) fail_mint: Pubkey, // fail-KASS  (cond(kass_vault, 1))
    pub(crate) pass_usdc: Pubkey, // pass-USDC  (cond(usdc_vault, 0))
    pub(crate) fail_usdc: Pubkey, // fail-USDC  (cond(usdc_vault, 1))
}

/// Compose the MetaDAO market (binary question + KASS/USDC conditional vaults)
/// for `resolver` and return the bound mints/vaults plus the oracle-PDA-owned
/// conditional-KASS destinations for the proposer's split bond.
pub(crate) fn setup_market(ctx: &mut TestCtx, resolver: Pubkey) -> (MarketAccounts, Pubkey, Pubkey) {
    let kass = ctx.kass_mint;
    let usdc = ctx.usdc_mint;
    let resolver_arr = resolver.to_bytes();
    let kass_arr = kass.to_bytes();
    let usdc_arr = usdc.to_bytes();
    let num_outcomes: u8 = 2;
    let question_id = [7u8; 32];

    let (question, _) = Pubkey::find_program_address(
        &metadao::question_seeds(&question_id, &resolver_arr.into(), &[num_outcomes]),
        &vault_id(),
    );
    let question_arr = question.to_bytes();
    let (kass_vault, _) = Pubkey::find_program_address(
        &metadao::vault_seeds(&question_arr.into(), &kass_arr.into()),
        &vault_id(),
    );
    let (usdc_vault, _) = Pubkey::find_program_address(
        &metadao::vault_seeds(&question_arr.into(), &usdc_arr.into()),
        &vault_id(),
    );

    let pass_mint = cond_mint(&kass_vault, 0);
    let fail_mint = cond_mint(&kass_vault, 1);
    let pass_usdc = cond_mint(&usdc_vault, 0);
    let fail_usdc = cond_mint(&usdc_vault, 1);
    let (event_authority, _) =
        Pubkey::find_program_address(&metadao::event_authority_seeds(), &vault_id());

    let kass_vault_underlying = ata(&kass_vault, &kass);
    let usdc_vault_underlying = ata(&usdc_vault, &usdc);

    let payer = ctx.payer.pubkey();
    let ix_q = Instruction {
        program_id: vault_id(),
        accounts: vec![
            AccountMeta::new(question, false),
            AccountMeta::new(payer, true),
            AccountMeta::new_readonly(solana_sdk_ids::system_program::ID, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(vault_id(), false),
        ],
        data: metadao::initialize_question_data(&question_id, &resolver_arr.into(), num_outcomes)
            .to_vec(),
    };
    ctx.send_many(&cu(ix_q), &[])
        .expect("initialize_question failed");

    let ix_kv = Instruction {
        program_id: vault_id(),
        accounts: vec![
            AccountMeta::new(kass_vault, false),
            AccountMeta::new_readonly(question, false),
            AccountMeta::new_readonly(kass, false),
            AccountMeta::new(kass_vault_underlying, false),
            AccountMeta::new(payer, true),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(ATA_PROGRAM_ID, false),
            AccountMeta::new_readonly(solana_sdk_ids::system_program::ID, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new(pass_mint, false),
            AccountMeta::new(fail_mint, false),
        ],
        data: metadao::initialize_conditional_vault_data().to_vec(),
    };
    ctx.send_many(&cu(ix_kv), &[])
        .expect("init KASS vault failed");

    let ix_uv = Instruction {
        program_id: vault_id(),
        accounts: vec![
            AccountMeta::new(usdc_vault, false),
            AccountMeta::new_readonly(question, false),
            AccountMeta::new_readonly(usdc, false),
            AccountMeta::new(usdc_vault_underlying, false),
            AccountMeta::new(payer, true),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(ATA_PROGRAM_ID, false),
            AccountMeta::new_readonly(solana_sdk_ids::system_program::ID, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new(pass_usdc, false),
            AccountMeta::new(fail_usdc, false),
        ],
        data: metadao::initialize_conditional_vault_data().to_vec(),
    };
    ctx.send_many(&cu(ix_uv), &[])
        .expect("init USDC vault failed");

    let oracle_pass_kass = Pubkey::new_unique();
    let oracle_fail_kass = Pubkey::new_unique();
    fabricate_token_account(ctx, oracle_pass_kass, pass_mint, resolver, 0);
    fabricate_token_account(ctx, oracle_fail_kass, fail_mint, resolver, 0);

    (
        MarketAccounts {
            question,
            kass_vault,
            kass_vault_underlying,
            usdc_vault,
            pass_mint,
            fail_mint,
            pass_usdc,
            fail_usdc,
        },
        oracle_pass_kass,
        oracle_fail_kass,
    )
}

/// Build a GENUINE AMM pool via the real `amm` binary: `create_amm` (base/quote
/// = the given conditional mints) → `add_liquidity` (reserves) → warp ≥ 150
/// slots → `crank_that_twap`. After one crank with `MAX_PRICE` allowed change,
/// the recorded slot-weighted TWAP equals the reserve price
/// `quote_reserve * 1e12 / base_reserve`. Returns the AMM PDA.
pub(crate) fn build_amm(
    ctx: &mut TestCtx,
    base_mint: Pubkey,
    quote_mint: Pubkey,
    base_reserve: u64,
    quote_reserve: u64,
    crank: bool,
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

    // Fund the user's base/quote with plenty for add_liquidity.
    let user_base = ata(&payer, &base_mint);
    let user_quote = ata(&payer, &quote_mint);
    fabricate_token_account(
        ctx,
        user_base,
        base_mint,
        payer,
        base_reserve.saturating_mul(4).max(1),
    );
    fabricate_token_account(
        ctx,
        user_quote,
        quote_mint,
        payer,
        quote_reserve.saturating_mul(4).max(1),
    );

    // --- create_amm (delayed-twap v0.4.1+): args =
    //     initial_observation:u128 ++ max_change:u128 ++ start_delay_slots:u64 ---
    let initial_obs: u128 = (quote_reserve as u128 * 1_000_000_000_000u128) / base_reserve as u128;
    let mut create_data = metadao::CREATE_AMM.to_vec();
    create_data.extend_from_slice(&initial_obs.to_le_bytes());
    create_data.extend_from_slice(&MAX_PRICE.to_le_bytes());
    create_data.extend_from_slice(&0u64.to_le_bytes()); // twap_start_delay_slots = 0
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
    ctx.send_many(&cu(ix_create), &[])
        .expect("create_amm failed");

    // user LP account (created after create_amm so lp_mint exists).
    let user_lp = ata(&payer, &lp_mint);
    fabricate_token_account(ctx, user_lp, lp_mint, payer, 0);

    // --- add_liquidity: args = quote:u64 ++ max_base:u64 ++ min_lp:u64 ---
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
    ctx.send_many(&cu(ix_add), &[])
        .expect("add_liquidity failed");

    // When `crank == false` the pool keeps reserves but NEVER records a TWAP
    // observation (aggregator stays 0 → settle reads its TWAP as 0). Used by the
    // pass_twap==0 survive test.
    if crank {
        // Advance > ONE_MINUTE_IN_SLOTS (150) so the crank records an observation.
        ctx.warp_slots(0, 300);

        let ix_crank = Instruction {
            program_id: amm_id(),
            accounts: vec![
                AccountMeta::new(amm, false),
                AccountMeta::new_readonly(amm_event_auth, false),
                AccountMeta::new_readonly(amm_id(), false),
            ],
            data: metadao::CRANK_THAT_TWAP.to_vec(),
        };
        ctx.send_many(&cu(ix_crank), &[])
            .expect("crank_that_twap failed");
    }

    amm
}

//! Shared fixtures + instruction builders for the `open_challenge` test groups.
//! Pure relocation of the file-local helpers; visibility widened to
//! `pub(crate)` so the `escrow` / `amm_binding` / `guards` submodules can reach
//! them via `use super::support::*`.

use super::*;

use kassandra_oracles_program::{cpi::metadao, state::AiClaim};
use solana_account::Account;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_instruction::{AccountMeta, Instruction};
use spl_token::{
    solana_program::{program_option::COption, program_pack::Pack},
    state::{Account as TokenAccount, AccountState},
    ID as TOKEN_PROGRAM_ID,
};

pub(crate) use kassandra_oracles_program::state::{AccountType, Phase};
pub(crate) use solana_keypair::Keypair;
pub(crate) use solana_pubkey::Pubkey;
pub(crate) use solana_signer::Signer;

const ATA_PROGRAM_ID: Pubkey =
    solana_pubkey::pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

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

/// Fabricate an `Amm`-program-owned account carrying the `Amm` account
/// discriminator and the given `base`/`quote` conditional mints at the layout
/// offsets `open_challenge` (and `settle_challenge`) bind against. The TWAP
/// fields are left zero — irrelevant to open_challenge's mint-pair binding.
pub(crate) fn fabricate_amm_account(ctx: &mut TestCtx, base: Pubkey, quote: Pubkey) -> Pubkey {
    let addr = Pubkey::new_unique();
    let mut data = vec![0u8; metadao::AMM_MIN_LEN];
    data[..8].copy_from_slice(&metadao::AMM_ACCOUNT_DISCRIMINATOR);
    data[metadao::AMM_BASE_MINT_OFFSET..metadao::AMM_BASE_MINT_OFFSET + 32]
        .copy_from_slice(&base.to_bytes());
    data[metadao::AMM_QUOTE_MINT_OFFSET..metadao::AMM_QUOTE_MINT_OFFSET + 32]
        .copy_from_slice(&quote.to_bytes());
    let lamports = ctx.svm.minimum_balance_for_rent_exemption(data.len());
    ctx.svm
        .set_account(
            addr,
            Account {
                lamports,
                data,
                owner: amm_id(),
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();
    addr
}

/// Every MetaDAO account a challenge market binds to.
pub(crate) struct MarketAccounts {
    pub(crate) question: Pubkey,
    pub(crate) kass_vault: Pubkey,
    pub(crate) kass_vault_underlying: Pubkey,
    pub(crate) pass_mint: Pubkey,
    pub(crate) fail_mint: Pubkey,
    pub(crate) usdc_vault: Pubkey,
    pub(crate) pass_amm: Pubkey,
    pub(crate) fail_amm: Pubkey,
}

pub(crate) fn cu(ix: Instruction) -> [Instruction; 2] {
    [
        ComputeBudgetInstruction::set_compute_unit_limit(600_000),
        ix,
    ]
}

/// Compose the MetaDAO market for `resolver` (the question's oracle/resolver):
/// initialize_question(num_outcomes=2) → KASS conditional vault → USDC
/// conditional vault → pass/fail AMM stubs. Returns the bound accounts plus the
/// oracle-PDA-owned conditional KASS destinations.
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
    let kass_vault_arr = kass_vault.to_bytes();
    let (pass_mint, _) = Pubkey::find_program_address(
        &metadao::conditional_token_mint_seeds(&kass_vault_arr.into(), &[0u8]),
        &vault_id(),
    );
    let (fail_mint, _) = Pubkey::find_program_address(
        &metadao::conditional_token_mint_seeds(&kass_vault_arr.into(), &[1u8]),
        &vault_id(),
    );
    let usdc_vault_arr = usdc_vault.to_bytes();
    let (usdc_pass, _) = Pubkey::find_program_address(
        &metadao::conditional_token_mint_seeds(&usdc_vault_arr.into(), &[0u8]),
        &vault_id(),
    );
    let (usdc_fail, _) = Pubkey::find_program_address(
        &metadao::conditional_token_mint_seeds(&usdc_vault_arr.into(), &[1u8]),
        &vault_id(),
    );
    let (event_authority, _) =
        Pubkey::find_program_address(&metadao::event_authority_seeds(), &vault_id());

    let kass_vault_underlying = ata(&kass_vault, &kass);
    let usdc_vault_underlying = ata(&usdc_vault, &usdc);

    // --- initialize_question ---
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

    // --- KASS conditional vault ---
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

    // --- USDC conditional vault ---
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
            AccountMeta::new(usdc_pass, false),
            AccountMeta::new(usdc_fail, false),
        ],
        data: metadao::initialize_conditional_vault_data().to_vec(),
    };
    ctx.send_many(&cu(ix_uv), &[])
        .expect("init USDC vault failed");

    let pass_amm = fabricate_amm_account(ctx, pass_mint, usdc_pass);
    let fail_amm = fabricate_amm_account(ctx, fail_mint, usdc_fail);

    // Oracle-PDA-owned destinations for the minted pass/fail conditional KASS.
    let oracle_pass_kass = Pubkey::new_unique();
    let oracle_fail_kass = Pubkey::new_unique();
    fabricate_token_account(ctx, oracle_pass_kass, pass_mint, resolver, 0);
    fabricate_token_account(ctx, oracle_fail_kass, fail_mint, resolver, 0);

    (
        MarketAccounts {
            question,
            kass_vault,
            kass_vault_underlying,
            pass_mint,
            fail_mint,
            usdc_vault,
            pass_amm,
            fail_amm,
        },
        oracle_pass_kass,
        oracle_fail_kass,
    )
}

/// Fabricate an `AiClaim` at its `[b"claim", oracle, proposer]` PDA, bound to
/// the given oracle/proposer, with `option` and `challenged == 0`.
pub(crate) fn seed_ai_claim(ctx: &mut TestCtx, oracle: Pubkey, proposer: Pubkey, option: u8) -> Pubkey {
    let (claim, bump) = Pubkey::find_program_address(
        &[b"claim", oracle.as_ref(), proposer.as_ref()],
        &ctx.program_id,
    );
    let mut c: AiClaim = bytemuck::Zeroable::zeroed();
    c.account_type = AccountType::AiClaim.as_u8();
    c.oracle = oracle.to_bytes().into();
    c.proposer = proposer.to_bytes().into();
    c.option = option;
    c.challenged = 0;
    c.bump = bump;
    ctx.seed_program_account_at(claim, bytemuck::bytes_of(&c).to_vec());
    claim
}

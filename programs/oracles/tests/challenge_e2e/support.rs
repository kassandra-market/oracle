//! Shared foundation (consts, PDA + token helpers, the independent
//! [`ConservationModel`]) + MetaDAO market composition for the CHALLENGE e2e.

use super::*;

use kassandra_oracles_program::{
    config::{MARKET_THRESHOLD_DEN, MARKET_THRESHOLD_NUM},
    cpi::metadao,
};
use solana_account::Account;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use solana_sdk_ids::system_program;
use solana_signer::Signer;
use spl_token::{
    solana_program::{program_option::COption, program_pack::Pack},
    state::{Account as TokenAccount, AccountState},
    ID as TOKEN_PROGRAM_ID,
};

pub(crate) const ATA_PROGRAM_ID: Pubkey =
    solana_pubkey::pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

/// Largest observation change per crank — lets a single crank fold the pool's
/// current price straight into the TWAP (no per-update clamp), so test TWAPs are
/// deterministic.
pub(crate) const MAX_PRICE: u128 = (u64::MAX as u128) * 1_000_000_000_000;

pub(crate) const BOND: u64 = 1_000_000_000; // 1 KASS bond on the challenged proposer.
/// Base reserve: 100 KASS (9 dp).
pub(crate) const BASE_RESERVE: u64 = 100_000_000_000;
/// Quote reserve: 100 USDC (6 dp) → seeded price 1e9 (scaled). add_liquidity
/// needs the quote ≥ 1e8.
pub(crate) const QUOTE_NEUTRAL: u64 = 100_000_000;

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

// ---------------------------------------------------------------------------
// Independent conservation reference (never reads the program's accounting)
// ---------------------------------------------------------------------------

/// Predicts, from `bond` + escrow + the governable fee config alone, every
/// post-settle token delta the two conservation equations rest on.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ConservationModel {
    disqualify: bool,
    bond: u64,
    pub(crate) escrow: u64,
    /// KASS fee → challenger on a successful challenge (`bond × succ_num/den`),
    /// capped at the proposer's remaining un-slashed bond. The e2e/fuzz proposer
    /// is challenged UN-slashed, so the cap is a no-op (`prior_slash == 0`).
    pub(crate) kass_fee: u64,
    /// USDC fee → proposer on a failed challenge (`escrow × fail_num/den`).
    usdc_fee: u64,
}

impl ConservationModel {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn compute(
        disqualify: bool,
        bond: u64,
        escrow: u64,
        prior_slash: u64,
        succ_num: u64,
        succ_den: u64,
        fail_num: u64,
        fail_den: u64,
    ) -> Self {
        let raw_kass_fee = (bond as u128 * succ_num as u128 / succ_den as u128) as u64;
        let kass_fee = raw_kass_fee.min(bond - prior_slash);
        let usdc_fee = (escrow as u128 * fail_num as u128 / fail_den as u128) as u64;
        Self {
            disqualify,
            bond,
            escrow,
            kass_fee,
            usdc_fee,
        }
    }

    /// Expected `challenger_kass` balance after settle.
    pub(crate) fn challenger_kass(&self) -> u64 {
        if self.disqualify {
            self.kass_fee
        } else {
            0
        }
    }
    /// Expected stake_vault DELTA across settle (redeem in, fee out).
    pub(crate) fn stake_vault_delta(&self) -> u64 {
        if self.disqualify {
            self.bond - self.kass_fee
        } else {
            self.bond
        }
    }
    /// Expected `proposer_usdc` balance after settle.
    pub(crate) fn proposer_usdc(&self) -> u64 {
        if self.disqualify {
            0
        } else {
            self.usdc_fee
        }
    }
    /// Expected `challenger_usdc_dest` balance after settle.
    pub(crate) fn challenger_usdc(&self) -> u64 {
        if self.disqualify {
            self.escrow
        } else {
            self.escrow - self.usdc_fee
        }
    }
}

/// Independent slash decision (a fresh copy of the on-chain rule): disqualify iff
/// `pass_twap > 0` AND `fail_twap * DEN > pass_twap * (DEN + NUM)`.
pub(crate) fn ref_disqualify(pass_twap: u128, fail_twap: u128) -> bool {
    if pass_twap == 0 {
        return false;
    }
    fail_twap * MARKET_THRESHOLD_DEN > pass_twap * (MARKET_THRESHOLD_DEN + MARKET_THRESHOLD_NUM)
}

// ---------------------------------------------------------------------------
// MetaDAO market composition (mirror settle_challenge.rs)
// ---------------------------------------------------------------------------

pub(crate) struct MarketAccounts {
    pub(crate) question: Pubkey,
    pub(crate) kass_vault: Pubkey,
    pub(crate) kass_vault_underlying: Pubkey,
    pub(crate) usdc_vault: Pubkey,
    pub(crate) pass_mint: Pubkey,
    pub(crate) fail_mint: Pubkey,
    pub(crate) pass_usdc: Pubkey,
    pub(crate) fail_usdc: Pubkey,
}

/// Compose the binary question + KASS/USDC conditional vaults for `resolver`,
/// plus the oracle-PDA-owned pass/fail conditional-KASS holders.
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
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(vault_id(), false),
        ],
        data: metadao::initialize_question_data(&question_id, &resolver_arr.into(), num_outcomes)
            .to_vec(),
    };
    ctx.send_many(&cu(ix_q), &[]).expect("initialize_question");

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
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new(pass_mint, false),
            AccountMeta::new(fail_mint, false),
        ],
        data: metadao::initialize_conditional_vault_data().to_vec(),
    };
    ctx.send_many(&cu(ix_kv), &[]).expect("init KASS vault");

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
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new(pass_usdc, false),
            AccountMeta::new(fail_usdc, false),
        ],
        data: metadao::initialize_conditional_vault_data().to_vec(),
    };
    ctx.send_many(&cu(ix_uv), &[]).expect("init USDC vault");

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

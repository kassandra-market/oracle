#![allow(unused_imports)]
use crate::*;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

/// Governance guardrail mirror of `state::MAX_FEE_BPS` (10% = 1000 bps).
pub const MAX_FEE_BPS: u16 = 1000;

/// `InitConfig` (Ix 0) — create the `Config` singleton at PDA `[b"config"]`.
/// Payload = `authority` (32) ++ `min_liquidity` (u64 LE) ++ `fee_bps` (u16 LE)
/// ++ `fee_destination` (32). Accounts:
/// `[0] config(pda,w) [1] payer(signer,w) [2] kass_mint(ro) [3] fee_destination(ro)
///  [4] system program [5] program_data(ro)`.
///
/// `program_data` is this program's BPF-Upgradeable-Loader `ProgramData` account
/// (derived from `PROGRAM_ID`): the processor reads its stored `upgrade_authority`
/// and REQUIRES it equals `payer` (the bootstrap front-run defense). Passed
/// read-only.
#[allow(clippy::too_many_arguments)]
pub fn init_config(
    payer: &Pubkey,
    kass_mint: &Pubkey,
    authority: &Pubkey,
    min_liquidity: u64,
    fee_bps: u16,
    fee_destination: &Pubkey,
) -> Instruction {
    let (config, _) = crate::pda::config();
    let (program_data, _) = crate::pda::program_data(&PROGRAM_ID);
    let mut data = vec![IX_INIT_CONFIG];
    data.extend_from_slice(authority.as_ref());
    data.extend_from_slice(&min_liquidity.to_le_bytes());
    data.extend_from_slice(&fee_bps.to_le_bytes());
    data.extend_from_slice(fee_destination.as_ref());
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(config, false),
            AccountMeta::new(*payer, true),
            AccountMeta::new_readonly(*kass_mint, false),
            AccountMeta::new_readonly(*fee_destination, false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(program_data, false),
        ],
        data,
    }
}

/// `CreateMarket` (Ix 2) — create the `outcome_index` binary sub-market for
/// `oracle`, its KASS escrow, and the creator's `Contribution`, transferring
/// `seed_amount` KASS in.
/// Payload = `seed_amount` (u64 LE) ++ `outcome_index` (u8). Accounts:
/// `[0] config(ro) [1] oracle(ro) [2] market(pda,w) [3] escrow(pda,w)
///  [4] kass_mint(ro) [5] creator(signer,w) [6] creator_kass_ata(w)
///  [7] contribution(pda,w) [8] token program [9] system program`.
#[allow(clippy::too_many_arguments)]
pub fn create_market(
    creator: &Pubkey,
    oracle: &Pubkey,
    kass_mint: &Pubkey,
    creator_kass_ata: &Pubkey,
    seed_amount: u64,
    outcome_index: u8,
) -> Instruction {
    let (config, _) = crate::pda::config();
    let (market, _) = crate::pda::market(oracle, outcome_index);
    let (escrow, _) = crate::pda::escrow(&market);
    let (contribution, _) = crate::pda::contribution(&market, creator);
    let mut data = vec![IX_CREATE_MARKET];
    data.extend_from_slice(&seed_amount.to_le_bytes());
    data.push(outcome_index);
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new_readonly(config, false),
            AccountMeta::new_readonly(*oracle, false),
            AccountMeta::new(market, false),
            AccountMeta::new(escrow, false),
            AccountMeta::new_readonly(*kass_mint, false),
            AccountMeta::new(*creator, true),
            AccountMeta::new(*creator_kass_ata, false),
            AccountMeta::new(contribution, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
        ],
        data,
    }
}

/// `Contribute` (Ix 3) — add `amount` KASS to a `Funding` market's escrow and
/// create-or-increment the contributor's `Contribution`.
/// Payload = `amount` (u64 LE). Accounts:
/// `[0] market(w) [1] escrow(w) [2] contributor(signer,w) [3] contributor_kass_ata(w)
///  [4] contribution(pda,w) [5] token program [6] system program`.
///
/// The processor reads only the first six accounts (its slice pattern tolerates
/// trailing accounts). The system program is appended so it is loaded into the
/// transaction: the first-ever contribution from a given contributor creates
/// their `Contribution` PDA via a CPI to the system program's `CreateAccount`,
/// which requires that program to be present in the tx account set (same as
/// `create_market`, which passes it explicitly).
pub fn contribute(
    contributor: &Pubkey,
    market: &Pubkey,
    escrow: &Pubkey,
    contributor_ata: &Pubkey,
    amount: u64,
) -> Instruction {
    let (contribution, _) = crate::pda::contribution(market, contributor);
    let mut data = vec![IX_CONTRIBUTE];
    data.extend_from_slice(&amount.to_le_bytes());
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(*market, false),
            AccountMeta::new(*escrow, false),
            AccountMeta::new(*contributor, true),
            AccountMeta::new(*contributor_ata, false),
            AccountMeta::new(contribution, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
        ],
        data,
    }
}

/// `Cancel` (Ix 4) — mark an under-funded `Funding` market `Cancelled` once its
/// underlying Kassandra oracle is terminal. Permissionless (no required signer
/// beyond the tx fee payer). Payload = empty. Accounts:
/// `[0] market(w) [1] oracle(ro)`.
pub fn cancel(market: &Pubkey, oracle: &Pubkey) -> Instruction {
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(*market, false),
            AccountMeta::new_readonly(*oracle, false),
        ],
        data: vec![IX_CANCEL],
    }
}

/// `Refund` (Ix 5) — permissionless per-contributor refund from a `Cancelled`
/// market. Program-signed transfer of the recorded stake out of escrow back to
/// the contributor's KASS ata, then the `Contribution` is CLOSED with its rent
/// returned to `contributor`. Payload = empty. Accounts:
/// `[0] market(w) [1] escrow(w) [2] contribution(w) [3] contributor_kass_ata(w)
///  [4] contributor(w) [5] token program`.
///
/// `market` is writable (its `open_contributions` counter is decremented) and
/// `contributor` (== `contribution.contributor`) receives the closed Contribution's
/// rent.
pub fn refund(
    market: &Pubkey,
    escrow: &Pubkey,
    contribution: &Pubkey,
    contributor_ata: &Pubkey,
    contributor: &Pubkey,
) -> Instruction {
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(*market, false),
            AccountMeta::new(*escrow, false),
            AccountMeta::new(*contribution, false),
            AccountMeta::new(*contributor_ata, false),
            AccountMeta::new(*contributor, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: vec![IX_REFUND],
    }
}

/// `Activate` (Ix 6) — turn a fully-funded `Funding` market into a live MetaDAO
/// cYES/cNO AMM market: verify the client-composed MetaDAO market, program-signed
/// split the escrowed KASS into cYES/cNO, and seed the AMM pool 50/50.
///
/// All addresses are derivable from `oracle` + `kass_mint` (the MetaDAO market was
/// composed with `oracle_authority = market PDA`, `question_id = oracle bytes`,
/// underlying `= kass_mint`, `base = cYES`, `quote = cNO`). Payload = empty.
///
/// Account order (MUST match `processor::activate`):
/// ```text
///  0  market                 (w)  — the market PDA, must be `Funding`
///  1  oracle                 (ro) — kassandra oracle, non-terminal
///  2  payer                  (signer,w) — rent for the 3 new market-owned token accts
///  3  question               (ro) — MetaDAO Question (oracle-authority == market)
///  4  vault                  (w)  — KASS conditional vault
///  5  vault_underlying_ata   (w)  — vault's KASS ATA (split destination for underlying)
///  6  escrow_vault           (w)  — market.escrow_vault (split source)
///  7  yes_mint               (w)  — conditional mint idx 0 (cYES)
///  8  no_mint                (w)  — conditional mint idx 1 (cNO)
///  9  market_cyes            (w)  — market-PDA-owned cYES holder (created here)
/// 10  market_cno             (w)  — market-PDA-owned cNO holder (created here)
/// 11  amm                    (w)  — the cYES/cNO pool
/// 12  lp_mint                (w)  — the pool's LP mint
/// 13  lp_vault               (w)  — market-PDA-owned LP holder (created here)
/// 14  amm_vault_base         (w)  — amm's cYES ATA
/// 15  amm_vault_quote        (w)  — amm's cNO ATA
/// 16  cv_event_authority     (ro)
/// 17  cv_program             (ro)
/// 18  amm_event_authority    (ro)
/// 19  amm_program            (ro)
/// 20  token program          (ro)
/// 21  system program         (ro)
/// ```
pub fn activate(
    payer: &Pubkey,
    oracle: &Pubkey,
    kass_mint: &Pubkey,
    outcome_index: u8,
) -> Instruction {
    use crate::metadao as md;
    let (market, _) = crate::pda::market(oracle, outcome_index);
    let (escrow, _) = crate::pda::escrow(&market);
    let (question, _) = md::question(&oracle.to_bytes(), &market, 2);
    let (vault, _) = md::vault(&question, kass_mint);
    let vault_underlying_ata = md::ata(&vault, kass_mint);
    let (yes_mint, _) = md::conditional_token_mint(&vault, 0);
    let (no_mint, _) = md::conditional_token_mint(&vault, 1);
    let (market_cyes, _) = crate::pda::market_cyes(&market);
    let (market_cno, _) = crate::pda::market_cno(&market);
    let (amm, _) = md::amm(&yes_mint, &no_mint);
    let (lp_mint, _) = md::amm_lp_mint(&amm);
    let (lp_vault, _) = crate::pda::lp_vault(&market);
    let amm_vault_base = md::ata(&amm, &yes_mint);
    let amm_vault_quote = md::ata(&amm, &no_mint);
    let (cv_event_auth, _) = md::event_authority(&md::CONDITIONAL_VAULT_ID);
    let (amm_event_auth, _) = md::event_authority(&md::AMM_ID);
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(market, false),
            AccountMeta::new_readonly(*oracle, false),
            AccountMeta::new(*payer, true),
            AccountMeta::new_readonly(question, false),
            AccountMeta::new(vault, false),
            AccountMeta::new(vault_underlying_ata, false),
            AccountMeta::new(escrow, false),
            AccountMeta::new(yes_mint, false),
            AccountMeta::new(no_mint, false),
            AccountMeta::new(market_cyes, false),
            AccountMeta::new(market_cno, false),
            AccountMeta::new(amm, false),
            AccountMeta::new(lp_mint, false),
            AccountMeta::new(lp_vault, false),
            AccountMeta::new(amm_vault_base, false),
            AccountMeta::new(amm_vault_quote, false),
            AccountMeta::new_readonly(cv_event_auth, false),
            AccountMeta::new_readonly(md::CONDITIONAL_VAULT_ID, false),
            AccountMeta::new_readonly(amm_event_auth, false),
            AccountMeta::new_readonly(md::AMM_ID, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
        ],
        data: vec![IX_ACTIVATE],
    }
}

/// `ClaimLp` (Ix 7) — permissionless per-contributor claim of the AMM LP tokens
/// seeded at `activate`, out of the Market-PDA-owned `lp_vault`. Program-signed
/// transfer of the floor pro-rata share (or the ENTIRE remaining `lp_vault` for the
/// LAST claimer) to the recorded contributor's LP token account, then the
/// `Contribution` is CLOSED with its rent returned to `contributor`.
/// Payload = empty. Accounts:
/// `[0] market(w) [1] lp_vault(w) [2] contribution(w) [3] contributor_lp_ata(w)
///  [4] contributor(w) [5] token program`.
///
/// `market` is writable (its `open_contributions` counter is decremented) and
/// `contributor` (== `contribution.contributor`) receives the closed Contribution's
/// rent.
pub fn claim_lp(
    market: &Pubkey,
    lp_vault: &Pubkey,
    contribution: &Pubkey,
    contributor_lp_ata: &Pubkey,
    contributor: &Pubkey,
) -> Instruction {
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(*market, false),
            AccountMeta::new(*lp_vault, false),
            AccountMeta::new(*contribution, false),
            AccountMeta::new(*contributor_lp_ata, false),
            AccountMeta::new(*contributor, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: vec![IX_CLAIM_LP],
    }
}

/// `CloseMarket` (Ix 10) — permissionless rent reclaim for a fully-settled market.
/// SPL-`CloseAccount`s the Market-PDA-owned token accounts (escrow always;
/// cyes/cno/lp_vault iff the market was activated) and closes the `Market` PDA, all
/// rent → `creator`. Payload = empty. Accounts:
/// `[0] market(w) [1] creator(w) [2] escrow(w) [3] cyes(w) [4] cno(w) [5] lp_vault(w)
///  [6] token program`.
///
/// All addresses are derivable from `oracle` + `outcome_index`. The cyes/cno/lp_vault
/// slots are always passed (fixed order); the program only closes them when the
/// market was activated (`market.lp_vault != default`).
pub fn close_market(oracle: &Pubkey, creator: &Pubkey, outcome_index: u8) -> Instruction {
    let (market, _) = crate::pda::market(oracle, outcome_index);
    let (escrow, _) = crate::pda::escrow(&market);
    let (cyes, _) = crate::pda::market_cyes(&market);
    let (cno, _) = crate::pda::market_cno(&market);
    let (lp_vault, _) = crate::pda::lp_vault(&market);
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(market, false),
            AccountMeta::new(*creator, false),
            AccountMeta::new(escrow, false),
            AccountMeta::new(cyes, false),
            AccountMeta::new(cno, false),
            AccountMeta::new(lp_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: vec![IX_CLOSE_MARKET],
    }
}

/// `ResolveMarket` (Ix 8) — permissionless idempotent crank that bridges the
/// terminal Kassandra oracle result into the market's MetaDAO `resolve_question`.
/// The Market PDA is the resolver (it signs the CPI via seeds), so it is passed as
/// the writable `market` account AND doubles as the CPI signer.
/// Payload = empty. Accounts:
/// `[0] market(w) [1] oracle(ro) [2] question(w) [3] cv_event_authority(ro)
///  [4] cv_program(ro)`.
pub fn resolve_market(
    market: &Pubkey,
    oracle: &Pubkey,
    question: &Pubkey,
    cv_event_authority: &Pubkey,
) -> Instruction {
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(*market, false),
            AccountMeta::new_readonly(*oracle, false),
            AccountMeta::new(*question, false),
            AccountMeta::new_readonly(*cv_event_authority, false),
            AccountMeta::new_readonly(crate::metadao::CONDITIONAL_VAULT_ID, false),
        ],
        data: vec![IX_RESOLVE_MARKET],
    }
}

/// `CollectFee` (Ix 9) — permissionless crank that cuts the protocol `fee_bps`
/// share of a resolved market's accrued LP earnings (program-signed
/// `amm::remove_liquidity` → `conditional_vault::redeem_tokens` → SPL `transfer`)
/// into `config.fee_destination`. Payload = empty.
///
/// All addresses are derivable from `oracle` + `kass_mint` (same composition as
/// `activate`) plus the `Config` PDA's `fee_destination`.
///
/// Account order (MUST match `processor::collect_fee`):
/// ```text
///  0  market                 (w)
///  1  config                 (ro)
///  2  fee_destination        (w)
///  3  question               (ro)
///  4  vault                  (w)
///  5  vault_underlying_ata   (w)
///  6  escrow_vault           (w)
///  7  yes_mint               (w)
///  8  no_mint                (w)
///  9  market_cyes            (w)
/// 10  market_cno             (w)
/// 11  amm                    (w)
/// 12  lp_mint                (w)
/// 13  lp_vault               (w)
/// 14  amm_vault_base         (w)
/// 15  amm_vault_quote        (w)
/// 16  cv_event_authority     (ro)
/// 17  cv_program             (ro)
/// 18  amm_event_authority    (ro)
/// 19  amm_program            (ro)
/// 20  token program          (ro)
/// ```
pub fn collect_fee(
    oracle: &Pubkey,
    kass_mint: &Pubkey,
    fee_destination: &Pubkey,
    outcome_index: u8,
) -> Instruction {
    use crate::metadao as md;
    let (config, _) = crate::pda::config();
    let (market, _) = crate::pda::market(oracle, outcome_index);
    let (escrow, _) = crate::pda::escrow(&market);
    let (question, _) = md::question(&oracle.to_bytes(), &market, 2);
    let (vault, _) = md::vault(&question, kass_mint);
    let vault_underlying_ata = md::ata(&vault, kass_mint);
    let (yes_mint, _) = md::conditional_token_mint(&vault, 0);
    let (no_mint, _) = md::conditional_token_mint(&vault, 1);
    let (market_cyes, _) = crate::pda::market_cyes(&market);
    let (market_cno, _) = crate::pda::market_cno(&market);
    let (amm, _) = md::amm(&yes_mint, &no_mint);
    let (lp_mint, _) = md::amm_lp_mint(&amm);
    let (lp_vault, _) = crate::pda::lp_vault(&market);
    let amm_vault_base = md::ata(&amm, &yes_mint);
    let amm_vault_quote = md::ata(&amm, &no_mint);
    let (cv_event_auth, _) = md::event_authority(&md::CONDITIONAL_VAULT_ID);
    let (amm_event_auth, _) = md::event_authority(&md::AMM_ID);
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(market, false),
            AccountMeta::new_readonly(config, false),
            AccountMeta::new(*fee_destination, false),
            AccountMeta::new_readonly(question, false),
            AccountMeta::new(vault, false),
            AccountMeta::new(vault_underlying_ata, false),
            AccountMeta::new(escrow, false),
            AccountMeta::new(yes_mint, false),
            AccountMeta::new(no_mint, false),
            AccountMeta::new(market_cyes, false),
            AccountMeta::new(market_cno, false),
            AccountMeta::new(amm, false),
            AccountMeta::new(lp_mint, false),
            AccountMeta::new(lp_vault, false),
            AccountMeta::new(amm_vault_base, false),
            AccountMeta::new(amm_vault_quote, false),
            AccountMeta::new_readonly(cv_event_auth, false),
            AccountMeta::new_readonly(md::CONDITIONAL_VAULT_ID, false),
            AccountMeta::new_readonly(amm_event_auth, false),
            AccountMeta::new_readonly(md::AMM_ID, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: vec![IX_COLLECT_FEE],
    }
}

/// `UpdateConfig` (Ix 1) — futarchy-gated update of `min_liquidity`, `fee_bps`,
/// and `fee_destination` (all three set together).
/// Payload = `min_liquidity` (u64 LE) ++ `fee_bps` (u16 LE) ++ `fee_destination` (32).
/// Accounts: `[0] config(w) [1] authority(signer) [2] fee_destination(ro)`.
pub fn update_config(
    authority: &Pubkey,
    min_liquidity: u64,
    fee_bps: u16,
    fee_destination: &Pubkey,
) -> Instruction {
    let (config, _) = crate::pda::config();
    let mut data = vec![IX_UPDATE_CONFIG];
    data.extend_from_slice(&min_liquidity.to_le_bytes());
    data.extend_from_slice(&fee_bps.to_le_bytes());
    data.extend_from_slice(fee_destination.as_ref());
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(config, false),
            AccountMeta::new_readonly(*authority, true),
            AccountMeta::new_readonly(*fee_destination, false),
        ],
        data,
    }
}

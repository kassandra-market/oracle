//! The `open_challenge` instruction builder + the disputed-oracle `Fixture` the
//! test groups build on. Pure relocation of the file-local builders; visibility
//! is `pub(crate)` so the test submodules reach them via `use super::fixture::*`.

use super::*;
use super::support::*;

use kassandra_oracles_program::{cpi::metadao, instruction::Ix};
use solana_instruction::{AccountMeta, Instruction};
use spl_token::ID as TOKEN_PROGRAM_ID;

/// Build the full `open_challenge` instruction. The challenger USDC escrow size
/// is computed on-chain (no payload amount); the caller passes the challenger's
/// USDC source account + the blessed `kass_dao`, and the protocol/escrow-vault
/// PDAs are derived here.
#[allow(clippy::too_many_arguments)]
pub(crate) fn open_challenge_ix(
    ctx: &TestCtx,
    oracle: Pubkey,
    ai_claim: Pubkey,
    proposer: Pubkey,
    market: Pubkey,
    challenger: Pubkey,
    m: &MarketAccounts,
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
            AccountMeta::new_readonly(m.pass_amm, false),
            AccountMeta::new_readonly(m.fail_amm, false),
            AccountMeta::new(stake_vault, false),
            AccountMeta::new(m.kass_vault_underlying, false),
            AccountMeta::new(m.pass_mint, false),
            AccountMeta::new(m.fail_mint, false),
            AccountMeta::new(oracle_pass_kass, false),
            AccountMeta::new(oracle_fail_kass, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(solana_sdk_ids::system_program::ID, false),
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

/// Common fixture: a disputed oracle in the `Challenge` phase, with one
/// surviving challenged-proposer's AiClaim and a fully composed MetaDAO market.
pub(crate) struct Fixture {
    pub(crate) oracle: Pubkey,
    pub(crate) nonce: u64,
    pub(crate) stake_vault: Pubkey,
    pub(crate) proposer: Pubkey,
    pub(crate) bond: u64,
    pub(crate) ai_claim: Pubkey,
    pub(crate) market: Pubkey,
    pub(crate) challenger: Keypair,
    pub(crate) m: MarketAccounts,
    pub(crate) oracle_pass_kass: Pubkey,
    pub(crate) oracle_fail_kass: Pubkey,
    pub(crate) kass_dao: Pubkey,
    pub(crate) challenger_usdc_src: Pubkey,
}

pub(crate) fn fixture() -> (TestCtx, Fixture) {
    fixture_with_bond(1_000_000_000)
}

pub(crate) fn fixture_with_bond(bond0: u64) -> (TestCtx, Fixture) {
    let mut ctx = TestCtx::new();
    ctx.svm.add_program(vault_id(), VAULT_SO).unwrap();
    ctx.svm.add_program(amm_id(), AMM_SO).unwrap();

    // Protocol + governance handoff with a deterministic kass_price so the
    // on-chain escrow sizing is computable.
    let kass_dao = ctx.bless_kass_price();

    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: bond0,
        },
        ProposerSpec {
            option: 1,
            bond: 1_000_000_000,
        },
    ]);
    let seeded = ctx.seeded(oracle);
    let nonce = seeded.nonce;
    let stake_vault = seeded.stake_vault;
    let proposer = seeded.proposers[0].pda;
    let bond = seeded.proposers[0].bond;

    ctx.set_phase(oracle, Phase::Challenge);
    let ai_claim = seed_ai_claim(&mut ctx, oracle, proposer, 0);

    let (m, oracle_pass_kass, oracle_fail_kass) = setup_market(&mut ctx, oracle);

    let (market, _) =
        Pubkey::find_program_address(&[b"market", ai_claim.as_ref()], &ctx.program_id);

    let challenger = Keypair::new();
    ctx.svm
        .airdrop(&challenger.pubkey(), 1_000_000_000)
        .unwrap();
    // Fund the challenger's USDC source generously (escrow needs bond×price).
    let challenger_usdc_src = ctx.fund_usdc(&challenger, 5_000_000);

    (
        ctx,
        Fixture {
            oracle,
            nonce,
            stake_vault,
            proposer,
            bond,
            ai_claim,
            market,
            challenger,
            m,
            oracle_pass_kass,
            oracle_fail_kass,
            kass_dao,
            challenger_usdc_src,
        },
    )
}

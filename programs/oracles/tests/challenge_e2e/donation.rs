// ---------------------------------------------------------------------------
// Donation edge (C2 review heads-up): anyone can SPL-transfer extra conditional
// KASS into the oracle-PDA-owned holder before settle; redeem burns the FULL
// balance, pulling the extra underlying into stake_vault. This documents that the
// donation only INFLATES stake_vault (the donor forfeits their own KASS) — it is
// NOT theft (no protocol funds leave to the donor), so production is unchanged.
// ---------------------------------------------------------------------------

use super::ops::*;
use super::support::*;
use super::*;

use kassandra_oracles_program::{
    cpi::metadao,
    state::{AccountType, AiClaim, Phase},
};
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use spl_token::ID as TOKEN_PROGRAM_ID;

#[test]
fn donation_into_holder_inflates_stake_vault_not_theft() {
    let mut ctx = TestCtx::new();
    ctx.svm.add_program(vault_id(), VAULT_SO).unwrap();
    ctx.svm.add_program(amm_id(), AMM_SO).unwrap();
    let kass_dao = ctx.bless_kass_price();

    // Lighter seeded-Challenge setup (the donation mechanic is orthogonal to how
    // we reach Challenge); real open/settle + real conditional vault throughout.
    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: BOND,
        },
        ProposerSpec {
            option: 1,
            bond: BOND,
        },
    ]);
    let seeded = ctx.seeded(oracle);
    let nonce = seeded.nonce;
    let stake_vault = seeded.stake_vault;
    let proposer = seeded.proposers[0].pda;
    let proposer_authority = seeded.proposers[0].authority.pubkey();
    ctx.set_phase(oracle, Phase::Challenge);

    let (claim, bump) = Pubkey::find_program_address(
        &[b"claim", oracle.as_ref(), proposer.as_ref()],
        &ctx.program_id,
    );
    let mut a: AiClaim = bytemuck::Zeroable::zeroed();
    a.account_type = AccountType::AiClaim.as_u8();
    a.oracle = oracle.to_bytes().into();
    a.proposer = proposer.to_bytes().into();
    a.option = 0;
    a.bump = bump;
    ctx.seed_program_account_at(claim, bytemuck::bytes_of(&a).to_vec());

    let (m, oracle_pass_kass, oracle_fail_kass) = setup_market(&mut ctx, oracle);
    // Honest (survive) market: both pools neutral → pass-side resolution.
    let pass_amm = build_pool(
        &mut ctx,
        m.pass_mint,
        m.pass_usdc,
        BASE_RESERVE,
        QUOTE_NEUTRAL,
    );
    let fail_amm = build_pool(
        &mut ctx,
        m.fail_mint,
        m.fail_usdc,
        BASE_RESERVE,
        QUOTE_NEUTRAL,
    );
    crank_pool(&mut ctx, pass_amm);
    crank_pool(&mut ctx, fail_amm);

    let (market, _) = Pubkey::find_program_address(&[b"market", claim.as_ref()], &ctx.program_id);
    let challenger = Keypair::new();
    ctx.svm
        .airdrop(&challenger.pubkey(), 1_000_000_000)
        .unwrap();
    let challenger_usdc_src = ctx.fund_usdc(&challenger, 5_000_000);
    let ix = open_challenge_ix(
        &ctx,
        oracle,
        claim,
        proposer,
        market,
        challenger.pubkey(),
        &m,
        pass_amm,
        fail_amm,
        stake_vault,
        oracle_pass_kass,
        oracle_fail_kass,
        kass_dao,
        challenger_usdc_src,
        nonce,
    );
    ctx.send_many(&cu(ix), &[&challenger])
        .expect("open_challenge");

    // --- the donation: a third party splits D KASS in THIS market's KASS vault
    // (minting D pass-KASS + D fail-KASS to themselves) then SPL-transfers the D
    // pass-KASS into the oracle-PDA-owned pass holder. ----------------------
    let donation: u64 = 250_000_000;
    let donor = Keypair::new();
    ctx.svm.airdrop(&donor.pubkey(), 1_000_000_000).unwrap();
    let donor_kass_src = ctx.fund_kass(&donor, donation);
    let donor_pass = Pubkey::new_unique();
    let donor_fail = Pubkey::new_unique();
    fabricate_token_account(&mut ctx, donor_pass, m.pass_mint, donor.pubkey(), 0);
    fabricate_token_account(&mut ctx, donor_fail, m.fail_mint, donor.pubkey(), 0);

    let (cv_event_auth, _) =
        Pubkey::find_program_address(&metadao::event_authority_seeds(), &vault_id());
    let split_ix = Instruction {
        program_id: vault_id(),
        accounts: vec![
            AccountMeta::new_readonly(m.question, false),
            AccountMeta::new(m.kass_vault, false),
            AccountMeta::new(m.kass_vault_underlying, false),
            AccountMeta::new(donor.pubkey(), true),
            AccountMeta::new(donor_kass_src, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(cv_event_auth, false),
            AccountMeta::new_readonly(vault_id(), false),
            AccountMeta::new(m.pass_mint, false),
            AccountMeta::new(m.fail_mint, false),
            AccountMeta::new(donor_pass, false),
            AccountMeta::new(donor_fail, false),
        ],
        data: metadao::split_tokens_data(donation).to_vec(),
    };
    ctx.send_many(&cu(split_ix), &[&donor])
        .expect("donor split");
    // SPL-transfer the donated pass-KASS into the oracle-PDA-owned holder.
    let xfer = spl_token::instruction::transfer(
        &TOKEN_PROGRAM_ID,
        &donor_pass,
        &oracle_pass_kass,
        &donor.pubkey(),
        &[],
        donation,
    )
    .unwrap();
    ctx.send_many(&cu(xfer), &[&donor])
        .expect("donate transfer");

    assert_eq!(
        ctx.token_balance(oracle_pass_kass),
        BOND + donation,
        "holder now carries bond + donated conditional KASS"
    );

    let payouts = fabricate_payouts(&mut ctx, market, proposer_authority, challenger.pubkey());
    let total_before = ctx.oracle(oracle).total_oracle_stake;
    let stake_before = ctx.token_balance(stake_vault);

    ctx.warp(TWAP_WINDOW + 1);
    let extras = SettleExtras {
        stake_vault,
        kass_vault: m.kass_vault,
        kass_vault_underlying: m.kass_vault_underlying,
        pass_mint: m.pass_mint,
        fail_mint: m.fail_mint,
        oracle_pass_kass,
        oracle_fail_kass,
        escrow_vault: payouts.escrow_vault,
        proposer_usdc: payouts.proposer_usdc,
        challenger_usdc_dest: payouts.challenger_usdc_dest,
        challenger_kass: payouts.challenger_kass,
    };
    let ix = settle_ix(
        &ctx, oracle, market, claim, proposer, m.question, pass_amm, fail_amm, &extras, nonce,
    );
    ctx.send_many(&cu(ix), &[]).expect("settle_challenge");

    // Redeem burned the FULL pass holder (bond + donation) → pulled bond +
    // donation into stake_vault. The proposer is NOT slashed (survive), so the
    // donation is pure inflation of stake_vault.
    assert_eq!(ctx.token_balance(oracle_pass_kass), 0, "holder burned");
    assert_eq!(
        ctx.token_balance(m.kass_vault_underlying),
        0,
        "underlying drained"
    );
    assert_eq!(
        ctx.token_balance(stake_vault),
        stake_before + BOND + donation,
        "stake_vault inflated by bond + DONATION (donor forfeited their KASS)"
    );
    // The naive conservation equation now carries the donation on top of the
    // conserved total: stake_vault + underlying == total + donation. The donor's
    // own KASS was pulled in and is NOT recoverable by them (their fail-KASS is
    // worthless) — external griefing that only ADDS KASS to the protocol, never
    // theft, so production is intentionally NOT guarded against it.
    assert_eq!(
        ctx.token_balance(stake_vault) + ctx.token_balance(m.kass_vault_underlying),
        total_before + donation,
        "donation inflates stake_vault beyond the conserved total (no funds stolen)"
    );
    assert_eq!(
        ctx.proposer(proposer).disqualified,
        0,
        "honest still survives"
    );
}

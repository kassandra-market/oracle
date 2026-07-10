use super::fixtures::*;
use super::support::*;
use super::*;

use kassandra_oracles_program::{cpi::metadao, error::KassandraError, state::Market};
use solana_instruction::{AccountMeta, Instruction};
use solana_instruction_error::InstructionError;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction_error::TransactionError;
use spl_token::ID as TOKEN_PROGRAM_ID;

#[test]
fn settle_before_twap_end_fails() {
    let (mut ctx, f) = fixture(QUOTE_LOW, QUOTE_HIGH);
    // Do NOT warp past twap_end.
    let ix = settle_ix(
        &ctx,
        f.oracle,
        f.market,
        f.ai_claim,
        f.proposer,
        f.m.question,
        f.pass_amm,
        f.fail_amm,
        &f.extras(),
        f.nonce,
    );
    let err = ctx.send_many(&cu(ix), &[]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(KassandraError::TwapWindowOpen as u32),
        ),
    );
}

#[test]
fn settle_twice_is_already_settled() {
    let (mut ctx, f) = fixture(QUOTE_LOW, QUOTE_HIGH);
    ctx.warp(TWAP_WINDOW + 1);

    let ix = settle_ix(
        &ctx,
        f.oracle,
        f.market,
        f.ai_claim,
        f.proposer,
        f.m.question,
        f.pass_amm,
        f.fail_amm,
        &f.extras(),
        f.nonce,
    );
    ctx.send_many(&cu(ix.clone()), &[])
        .expect("first settle ok");

    let err = ctx.send_many(&cu(ix), &[]).unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(KassandraError::AlreadySettled as u32),
        ),
    );
}

#[test]
fn settle_last_block_swap_does_not_flip_outcome() {
    // TWAP manipulation resistance: an honest market (pass == fail == 1e9) is
    // cranked over the window. A single large last-moment BUY on the fail pool
    // swings its INSTANTANEOUS price far above threshold, but the AMM only
    // records a new observation once per ONE_MINUTE_IN_SLOTS, so within the same
    // minute the stored slot-weighted TWAP is unchanged → the claim still
    // SURVIVES. This is exactly the time-weighting the design relies on.
    let (mut ctx, f) = fixture(QUOTE_LOW, QUOTE_LOW);

    // Large BUY on the fail pool (quote in, base out) — drives fail price up.
    // Fund the payer with fail-USDC and a fail-KASS receive account.
    let payer = ctx.payer.pubkey();
    let user_base = ata(&payer, &f.m.fail_mint);
    let user_quote = ata(&payer, &f.m.fail_usdc);
    // (user_base/user_quote already exist from build_amm; top up quote.)
    fabricate_token_account(&mut ctx, user_quote, f.m.fail_usdc, payer, 10_000_000_000);
    fabricate_token_account(&mut ctx, user_base, f.m.fail_mint, payer, 0);
    let vault_ata_base = ata(&f.fail_amm, &f.m.fail_mint);
    let vault_ata_quote = ata(&f.fail_amm, &f.m.fail_usdc);
    let (amm_event_auth, _) =
        Pubkey::find_program_address(&metadao::event_authority_seeds(), &amm_id());

    // SwapArgs { swap_type: Buy(0), input_amount, output_amount_min: 0 }.
    let mut swap_data = metadao::SWAP.to_vec();
    swap_data.push(0u8); // SwapType::Buy
    swap_data.extend_from_slice(&80_000_000u64.to_le_bytes()); // 80 USDC in
    swap_data.extend_from_slice(&0u64.to_le_bytes());
    let ix_swap = Instruction {
        program_id: amm_id(),
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(f.fail_amm, false),
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
    // Only a few slots after the crank (< 150), so update_twap records nothing.
    ctx.warp_slots(0, 5);
    ctx.send_many(&cu(ix_swap), &[])
        .expect("swap should succeed");

    ctx.warp(TWAP_WINDOW + 1);
    let ix = settle_ix(
        &ctx,
        f.oracle,
        f.market,
        f.ai_claim,
        f.proposer,
        f.m.question,
        f.pass_amm,
        f.fail_amm,
        &f.extras(),
        f.nonce,
    );
    ctx.send_many(&cu(ix), &[]).expect("settle should succeed");

    // Despite the huge last-moment swing, the TWAP didn't move → survives.
    assert_eq!(
        ctx.proposer(f.proposer).disqualified,
        0,
        "TWAP resists last-block swap"
    );
    let (n0, n1, _) = question_resolution(&ctx, f.m.question);
    assert_eq!((n0, n1), (1, 0), "pass-side: claim survived manipulation");
}

// NOTE on a stronger "crank-folded spike dilutes over the window" test (review
// item 6): the precise diluted average depends on the exact slot deltas between
// the honest cranks and the post-manipulation crank, plus the v0.4.2
// `max_observation_change_per_update` clamp. Making the pass/fail margin land
// deterministically on the survive side requires choreographing an honest window
// many multiples of `ONE_MINUTE_IN_SLOTS` long against a single 150-slot spike
// window — brittle against LiteSVM's slot accounting. The once-per-minute
// observation gate (the realistic last-block attack) is covered deterministically
// by `settle_last_block_swap_does_not_flip_outcome` above; the longer-window
// dilution is a direct consequence of settle dividing the aggregator by the FULL
// elapsed window in `verify_and_read_twap`, exercised by the honest/fraud tests.

#[test]
fn settle_uncranked_pass_pool_survives() {
    // pass pool is NEVER cranked (pass_twap == 0) while the fail pool is cranked
    // to a high price. A pass_twap of 0 means NO counter-trading on the pass side
    // (design §7 → survive), so even with fail far above threshold the claim must
    // SURVIVE — otherwise a challenger could crank only the fail pool to cheaply
    // disqualify an honest proposer.
    let (mut ctx, f) = fixture_with_attack(QUOTE_LOW, QUOTE_HIGH, AmmAttack::PassUncranked);
    let surviving_before = ctx.oracle(f.oracle).surviving_count;
    ctx.warp(TWAP_WINDOW + 1);

    let ix = settle_ix(
        &ctx,
        f.oracle,
        f.market,
        f.ai_claim,
        f.proposer,
        f.m.question,
        f.pass_amm,
        f.fail_amm,
        &f.extras(),
        f.nonce,
    );
    ctx.send_many(&cu(ix), &[]).expect("settle should succeed");

    let p = ctx.proposer(f.proposer);
    assert_eq!(
        p.disqualified, 0,
        "pass_twap==0 must survive, not disqualify"
    );
    assert_eq!(p.slashed, 0);
    let o = ctx.oracle(f.oracle);
    assert_eq!(o.surviving_count, surviving_before, "no slash");
    assert_eq!(o.open_challenge_count, 0);
    assert_eq!(ctx.read_pod::<Market>(f.market).settled, 1);
    let (n0, n1, _) = question_resolution(&ctx, f.m.question);
    assert_eq!((n0, n1), (1, 0), "pass-side resolution");
}

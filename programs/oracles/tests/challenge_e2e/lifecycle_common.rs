//! Shared plumbing for the two deterministic real-AMM lifecycle tests: the REAL
//! dispute-core front door + the cross-outcome resolution/conservation assertion.

use super::ops::*;
use super::support::*;
use super::*;

use kassandra_oracles_program::{
    config::PHASE_WINDOW,
    state::{Market, Phase, VOTE_APPROVE},
};
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

// ---------------------------------------------------------------------------
// Dispute-core instruction builders (mirror lifecycle_e2e.rs)
// ---------------------------------------------------------------------------

fn finalize_facts_ix(ctx: &TestCtx, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
    ctx.finalize_facts_ix(oracle, tail)
}

fn claim_pda(program_id: &Pubkey, oracle: &Pubkey, proposer: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"claim", oracle.as_ref(), proposer.as_ref()], program_id)
}

/// What the front door hands back: a real oracle sitting in `Challenge` with a
/// real `AiClaim` for an un-slashed, surviving proposer ready to be challenged.
pub(crate) struct Challenged {
    pub(crate) oracle: Pubkey,
    pub(crate) nonce: u64,
    pub(crate) stake_vault: Pubkey,
    pub(crate) proposer: Pubkey,
    pub(crate) proposer_authority: Pubkey,
    pub(crate) ai_claim: Pubkey,
}

/// Drive the REAL dispute core to `Phase::Challenge` (see module header). The
/// returned proposer is the option-0 proposer, who claims option 0 (no flip), so
/// it is surviving with `slashed_amount == 0` — a clean bond to challenge.
pub(crate) fn front_door_to_challenge(ctx: &mut TestCtx) -> Challenged {
    // create_oracle → propose×2 (conflict) → finalize_proposals => FactProposal.
    let oracle = ctx.dispute_via_real_flow(&[
        ProposerSpec {
            option: 0,
            bond: BOND,
        },
        ProposerSpec {
            option: 1,
            bond: BOND,
        },
    ]);
    let (vault, _) = TestCtx::stake_vault_pda(&ctx.program_id, &oracle);
    let nonce = ctx.seeded(oracle).nonce;
    let proposer_pdas: Vec<Pubkey> = ctx.proposers(oracle).iter().map(|p| p.pda).collect();
    let authorities: Vec<Keypair> = ctx
        .proposers(oracle)
        .iter()
        .map(|p| p.authority.insecure_clone())
        .collect();

    // submit_fact (FactProposal still open).
    let submitter = Keypair::new();
    ctx.svm.airdrop(&submitter.pubkey(), 1_000_000_000).unwrap();
    let submitter_kass = ctx.fund_kass(&submitter, 1_000_000);
    let content_hash = [0x07u8; 32];
    let (fact, _) = TestCtx::fact_pda(&ctx.program_id, &oracle, &content_hash);
    ctx.send(
        submit_fact_ix(
            ctx,
            oracle,
            fact,
            submitter.pubkey(),
            submitter_kass,
            vault,
            submit_fact_payload(&content_hash, 100, b"ipfs://fact"),
        ),
        &[&submitter],
    )
    .expect("submit_fact");

    // warp past FactProposal, advance_phase => FactVoting.
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(advance_phase_ix(ctx, oracle), &[])
        .expect("advance_phase");

    // vote approve well past the 2/3 quorum of dispute_bond_total (== 2*BOND).
    let voter = Keypair::new();
    ctx.svm.airdrop(&voter.pubkey(), 1_000_000_000).unwrap();
    let voter_kass = ctx.fund_kass(&voter, 2 * BOND);
    let (fact_vote, _) = TestCtx::vote_pda(&ctx.program_id, &fact, &voter.pubkey());
    ctx.send(
        vote_fact_ix(
            ctx,
            oracle,
            fact,
            fact_vote,
            voter.pubkey(),
            voter_kass,
            vault,
            vote_payload(VOTE_APPROVE, 2 * BOND),
        ),
        &[&voter],
    )
    .expect("vote_fact");

    // warp past voting, finalize_facts => AiClaim.
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(finalize_facts_ix(ctx, oracle, &[fact]), &[])
        .expect("finalize_facts");
    assert_eq!(ctx.oracle(oracle).phase, Phase::AiClaim.as_u8());

    // Both proposers claim option 0: proposer[0] (orig 0) does NOT flip (survives
    // un-slashed); proposer[1] (orig 1) flips (partial slash, still surviving).
    for (auth, pda) in authorities.iter().zip(&proposer_pdas) {
        ctx.svm.airdrop(&auth.pubkey(), 1_000_000_000).unwrap();
        let (claim, _) = claim_pda(&ctx.program_id, &oracle, pda);
        ctx.send(
            submit_ai_claim_ix(
                ctx,
                oracle,
                *pda,
                claim,
                auth.pubkey(),
                submit_ai_payload(0),
            ),
            &[auth],
        )
        .expect("submit_ai_claim");
    }

    // warp past AiClaim, finalize_ai_claims => Challenge.
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(finalize_ai_claims_ix(ctx, oracle, &proposer_pdas), &[])
        .expect("finalize_ai_claims");
    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::Challenge.as_u8());
    assert_eq!(
        o.surviving_count, 2,
        "both proposers survive into Challenge"
    );

    let proposer = proposer_pdas[0];
    let proposer_authority = authorities[0].pubkey();
    assert_eq!(
        ctx.proposer(proposer).slashed_amount,
        0,
        "the challenged (option-0) proposer is un-slashed"
    );
    let (ai_claim, _) = claim_pda(&ctx.program_id, &oracle, &proposer);

    Challenged {
        oracle,
        nonce,
        stake_vault: vault,
        proposer,
        proposer_authority,
        ai_claim,
    }
}

/// The shared cross-outcome assertions: question settled, conditional KASS fully
/// redeemed + holders burned, and BOTH conservation equations against the
/// INDEPENDENT [`ConservationModel`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn assert_resolution_and_conservation(
    ctx: &TestCtx,
    oracle: Pubkey,
    market: Pubkey,
    _proposer: Pubkey,
    _question: Pubkey,
    x: &SettleExtras,
    model: &ConservationModel,
    total_before: u64,
    _bond_pool_before: u64,
    stake_before: u64,
) {
    assert_eq!(ctx.read_pod::<Market>(market).settled, 1, "market settled");
    assert_eq!(
        ctx.oracle(oracle).open_challenge_count,
        0,
        "counter back to 0"
    );

    // Physical redeem drained the conditional KASS vault + burned both holders.
    assert_eq!(
        ctx.token_balance(x.kass_vault_underlying),
        0,
        "underlying drained"
    );
    assert_eq!(ctx.token_balance(x.oracle_pass_kass), 0, "pass-KASS burned");
    assert_eq!(ctx.token_balance(x.oracle_fail_kass), 0, "fail-KASS burned");
    // No donation present in these e2e flows: the holders carried EXACTLY the
    // bond-derived balance (see the dedicated donation test for the griefing edge).

    // KASS routing vs the independent reference.
    assert_eq!(
        ctx.token_balance(x.challenger_kass),
        model.challenger_kass()
    );
    assert_eq!(
        ctx.token_balance(x.stake_vault),
        stake_before + model.stake_vault_delta(),
        "stake_vault delta == redeem − kass_fee carve-out"
    );
    // KASS conservation: stake_vault + underlying + challenger_kass == total
    // (the kass_fee carve-out left the system to the challenger on disqualify; on
    // survive challenger_kass == 0 and it reduces to the idle-bond conservation).
    assert_eq!(
        ctx.token_balance(x.stake_vault)
            + ctx.token_balance(x.kass_vault_underlying)
            + ctx.token_balance(x.challenger_kass),
        total_before,
        "KASS conservation incl. the kass_fee carve-out",
    );

    // USDC routing + conservation vs the independent reference.
    assert_eq!(ctx.token_balance(x.proposer_usdc), model.proposer_usdc());
    assert_eq!(
        ctx.token_balance(x.challenger_usdc_dest),
        model.challenger_usdc()
    );
    assert_eq!(
        ctx.token_balance(x.proposer_usdc) + ctx.token_balance(x.challenger_usdc_dest),
        model.escrow,
        "USDC escrow fully accounted (fee + return == escrow)",
    );
    assert_eq!(ctx.token_balance(x.escrow_vault), 0, "escrow drained");
}

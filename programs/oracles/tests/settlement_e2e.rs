//! Task S5 — end-to-end STAKER-SETTLEMENT lifecycle tests.
//!
//! These drive a full lifecycle to a terminal state and then have EVERY staker
//! claim (the S2 `claim_proposer` / `claim_fact` / `claim_fact_vote`) and every
//! account close (the S4 `close_ai_claim` / `close_market`), asserting the
//! per-actor matrix, that all accounts close, that the stake vault drains to
//! floor dust, and KASS conservation sourced ONLY from the stake vault.
//!
//! # What is REAL vs SEEDED (honest split)
//! * **Tests 1-2** drive the dispute core through the GENUINE front door —
//!   `create_oracle → propose×2 (conflict) → finalize_proposals → submit_fact →
//!   advance_phase → vote_fact → finalize_facts → submit_ai_claim×2 →
//!   finalize_ai_claims → finalize_oracle` (no `set_phase`; only `warp` moves
//!   time) — then run REAL claims + closes. Everything is a real instruction,
//!   including the creation-time emission the real `create_oracle` MINTS into the
//!   vault (ON by default): test 1 (Resolved) folds it into `reward_pool`, test 2
//!   (InvalidDeadend) burns it back.
//! * **Tests 3-5 (emission)** SEED the disputed oracle (the dispute mechanics are
//!   exhaustively covered by `lifecycle_e2e` / `invariants` Arm A) but keep the
//!   emission MOVEMENT + settlement REAL: the creation-time `reward_emission` is
//!   placed in the vault (backed by mint supply), then the REAL `finalize_oracle`
//!   folds it into `reward_pool` (Resolved) or BURNS it back (InvalidDeadend), and
//!   the REAL claims/closes settle it. The mint-AT-CREATION half of emission
//!   (`create_oracle` minting from the reservoir, the fee-burn boost, the
//!   mint-authority guard) is covered by `tests/emissions.rs`.
//!
//! The independent-reference CONSERVATION FUZZ (every matrix combination ×
//! emission × outcome, asserted against a reimplemented reference) lives in
//! `tests/invariants.rs` (Arms D + E).

mod common;
use common::*;

use kassandra_oracles_program::{
    config::{FACT_VOTE_SLASH_DEN, FACT_VOTE_SLASH_NUM, PHASE_WINDOW},
    reward,
    state::{Phase, VOTE_APPROVE},
};
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

#[path = "settlement_e2e/real_dispute.rs"]
mod real_dispute;
#[path = "settlement_e2e/emission_seeded.rs"]
mod emission_seeded;
#[path = "settlement_e2e/fact_vote.rs"]
mod fact_vote;

// ---------------------------------------------------------------------------
// Dispute-core instruction builders (mirror lifecycle_e2e.rs / challenge_e2e.rs)
// ---------------------------------------------------------------------------

fn finalize_facts_ix(ctx: &TestCtx, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
    ctx.finalize_facts_ix(oracle, tail)
}

/// AiClaim PDA seeds `[b"claim", oracle, proposer]`.
fn claim_pda(program_id: &Pubkey, oracle: &Pubkey, proposer: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"claim", oracle.as_ref(), proposer.as_ref()], program_id)
}

/// The proposer-reward bucket for the oracle's current resolution stamps.
fn proposer_bucket_of(o: &kassandra_oracles_program::state::Oracle) -> u64 {
    reward::reward_buckets(
        o.reward_pool,
        o.reward_proposer_weight,
        o.reward_fact_weight,
        o.total_correct_proposer_stake,
        o.total_approved_fact_stake,
    )
    .0
}

/// The fact-reward bucket for the oracle's current resolution stamps.
fn fact_bucket_of(o: &kassandra_oracles_program::state::Oracle) -> u64 {
    reward::reward_buckets(
        o.reward_pool,
        o.reward_proposer_weight,
        o.reward_fact_weight,
        o.total_correct_proposer_stake,
        o.total_approved_fact_stake,
    )
    .1
}

// ---------------------------------------------------------------------------
// Real front-door FACT/VOTE dead-end driver (tests 6-7, in fact_vote.rs)
// ---------------------------------------------------------------------------

/// Bonds / stakes for the driven fact/vote dead-end (chosen so the rejected fact
/// stays below the 2/3 quorum and the rejected approve stake is ODD).
const FVD_BOND: u64 = 1_000;
const FVD_AGREED_SUB: u64 = 400;
const FVD_AGREED_VOTE: u64 = 2_000;
const FVD_REJECTED_SUB: u64 = 300;
const FVD_REJECTED_VOTE: u64 = 501; // odd → exercises the ceil-vs-floor margin

/// Everything a fact/vote dead-end settlement test needs: the oracle handles, the
/// two proposers, both facts (+ their submitter/voter), and the pre-burn snapshots
/// (`supply_before` / `bond_pool` / `emission` / `sum_stakes`) needed to assert
/// the burn delta + full conservation equation.
struct DrivenFactVoteDeadend {
    oracle: Pubkey,
    nonce: u64,
    vault: Pubkey,
    proposers: Vec<(Keypair, Pubkey)>,
    agreed_fact: Pubkey,
    agreed_submitter: Keypair,
    agreed_vote: Pubkey,
    agreed_voter: Keypair,
    rejected_fact: Pubkey,
    rejected_submitter: Keypair,
    rejected_vote: Pubkey,
    rejected_voter: Keypair,
    emission: u64,
    bond_pool: u64,
    supply_before: u64,
    sum_stakes: u64,
}

/// `floor(value · num / den)` — the aggregate bond_pool credit (mirrors
/// `finalize_facts`'s rejected-fact voter slash accumulation).
fn floor_slash(value: u64, num: u64, den: u64) -> u64 {
    (value as u128 * num as u128 / den as u128) as u64
}

/// Drive a REAL dispute to a Tie dead-end carrying one AGREED + one REJECTED fact
/// (the rejected fact has a SLASHED approve-voter). The creation-time emission
/// (minted by the real `create_oracle`, ON by default) sits in the vault and is
/// burned by the terminal `finalize_oracle`. Snapshots supply + bond_pool just
/// before the burn. The oracle ends in InvalidDeadend.
fn drive_real_fact_vote_deadend(ctx: &mut TestCtx) -> DrivenFactVoteDeadend {
    // create_oracle → propose×2 (DISTINCT options 0/1) → finalize_proposals.
    let oracle = ctx.dispute_via_real_flow(&[
        ProposerSpec {
            option: 0,
            bond: FVD_BOND,
        },
        ProposerSpec {
            option: 1,
            bond: FVD_BOND,
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
    // dispute_bond_total == 2*FVD_BOND == 2000 → 2/3 quorum == 1334.

    // ---- submit_fact ×2 (FactProposal window) ------------------------------
    let agreed_submitter = Keypair::new();
    ctx.svm
        .airdrop(&agreed_submitter.pubkey(), 1_000_000_000)
        .unwrap();
    let agreed_sub_kass = ctx.fund_kass(&agreed_submitter, FVD_AGREED_SUB);
    let agreed_hash = [0x07u8; 32];
    let (agreed_fact, _) = TestCtx::fact_pda(&ctx.program_id, &oracle, &agreed_hash);
    ctx.send(
        submit_fact_ix(
            ctx,
            oracle,
            agreed_fact,
            agreed_submitter.pubkey(),
            agreed_sub_kass,
            vault,
            submit_fact_payload(&agreed_hash, FVD_AGREED_SUB, b"ipfs://agreed"),
        ),
        &[&agreed_submitter],
    )
    .expect("submit_fact (agreed)");

    let rejected_submitter = Keypair::new();
    ctx.svm
        .airdrop(&rejected_submitter.pubkey(), 1_000_000_000)
        .unwrap();
    let rejected_sub_kass = ctx.fund_kass(&rejected_submitter, FVD_REJECTED_SUB);
    let rejected_hash = [0x09u8; 32];
    let (rejected_fact, _) = TestCtx::fact_pda(&ctx.program_id, &oracle, &rejected_hash);
    ctx.send(
        submit_fact_ix(
            ctx,
            oracle,
            rejected_fact,
            rejected_submitter.pubkey(),
            rejected_sub_kass,
            vault,
            submit_fact_payload(&rejected_hash, FVD_REJECTED_SUB, b"ipfs://rejected"),
        ),
        &[&rejected_submitter],
    )
    .expect("submit_fact (rejected)");

    // ---- advance to FactVoting ---------------------------------------------
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(advance_phase_ix(ctx, oracle), &[])
        .expect("advance_phase");

    // ---- vote_fact ×2 (FactVoting window) ----------------------------------
    // Agreed fact: approve 2000 (>= 1334 quorum) → agreed.
    let agreed_voter = Keypair::new();
    ctx.svm
        .airdrop(&agreed_voter.pubkey(), 1_000_000_000)
        .unwrap();
    let agreed_voter_kass = ctx.fund_kass(&agreed_voter, FVD_AGREED_VOTE);
    let (agreed_vote, _) = TestCtx::vote_pda(&ctx.program_id, &agreed_fact, &agreed_voter.pubkey());
    ctx.send(
        vote_fact_ix(
            ctx,
            oracle,
            agreed_fact,
            agreed_vote,
            agreed_voter.pubkey(),
            agreed_voter_kass,
            vault,
            vote_payload(VOTE_APPROVE, FVD_AGREED_VOTE),
        ),
        &[&agreed_voter],
    )
    .expect("vote_fact (agreed)");

    // Rejected fact: approve 501 (> duplicate 0 but < 1334 quorum) → rejected;
    // this lone approve-voter is slashed `ceil(501·1/2) = 251` at claim.
    let rejected_voter = Keypair::new();
    ctx.svm
        .airdrop(&rejected_voter.pubkey(), 1_000_000_000)
        .unwrap();
    let rejected_voter_kass = ctx.fund_kass(&rejected_voter, FVD_REJECTED_VOTE);
    let (rejected_vote, _) =
        TestCtx::vote_pda(&ctx.program_id, &rejected_fact, &rejected_voter.pubkey());
    ctx.send(
        vote_fact_ix(
            ctx,
            oracle,
            rejected_fact,
            rejected_vote,
            rejected_voter.pubkey(),
            rejected_voter_kass,
            vault,
            vote_payload(VOTE_APPROVE, FVD_REJECTED_VOTE),
        ),
        &[&rejected_voter],
    )
    .expect("vote_fact (rejected)");

    // ---- finalize_facts → AiClaim ------------------------------------------
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(
        finalize_facts_ix(ctx, oracle, &[agreed_fact, rejected_fact]),
        &[],
    )
    .expect("finalize_facts");
    assert_eq!(
        ctx.fact(agreed_fact).agreed,
        1,
        "agreed fact cleared quorum"
    );
    let rf = ctx.fact(rejected_fact);
    assert_eq!(rf.agreed, 0, "rejected fact not agreed");
    assert_eq!(rf.duplicate, 0, "rejected fact not duplicate-dominant");

    // bond_pool == rejected submitter stake + FLOOR aggregate voter slash.
    let expected_bond_pool =
        FVD_REJECTED_SUB + floor_slash(FVD_REJECTED_VOTE, FACT_VOTE_SLASH_NUM, FACT_VOTE_SLASH_DEN);
    assert_eq!(
        ctx.oracle(oracle).bond_pool,
        expected_bond_pool,
        "bond_pool = rejected submitter stake + floor(approve·1/2)"
    );

    // ---- submit_ai_claim ×2 (DISTINCT options 0/1 → tie) -------------------
    for (i, (auth, pda)) in authorities.iter().zip(&proposer_pdas).enumerate() {
        ctx.svm.airdrop(&auth.pubkey(), 1_000_000_000).unwrap();
        let (claim, _) = claim_pda(&ctx.program_id, &oracle, pda);
        ctx.send(
            submit_ai_claim_ix(
                ctx,
                oracle,
                *pda,
                claim,
                auth.pubkey(),
                submit_ai_payload(i as u8),
            ),
            &[auth],
        )
        .expect("submit_ai_claim");
    }

    // ---- finalize_ai_claims → Challenge ------------------------------------
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(finalize_ai_claims_ix(ctx, oracle, &proposer_pdas), &[])
        .expect("finalize_ai_claims");
    assert_eq!(ctx.oracle(oracle).phase, Phase::Challenge.as_u8());

    // ---- the creation-time emission is ALREADY in the vault (emission is ON by
    // default: the real create_oracle minted it), so capture the REAL amount
    // rather than seeding one. Then snapshot supply + bond_pool before the burn.
    let emission = ctx.oracle(oracle).reward_emission;
    assert!(emission > 0, "genesis create minted a real emission");
    let sum_stakes =
        2 * FVD_BOND + FVD_AGREED_SUB + FVD_AGREED_VOTE + FVD_REJECTED_SUB + FVD_REJECTED_VOTE;
    assert_eq!(
        ctx.token_balance(vault),
        sum_stakes + emission,
        "vault holds Σ stakes + emission before the terminal burn"
    );
    let supply_before = ctx.mint_supply(ctx.kass_mint);
    let bond_pool = ctx.oracle(oracle).bond_pool;

    // ---- finalize_oracle → InvalidDeadend (burns bond_pool + emission) -----
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(ctx.finalize_oracle_ix(oracle, &proposer_pdas), &[])
        .expect("finalize_oracle");
    assert_eq!(ctx.oracle(oracle).phase, Phase::InvalidDeadend as u8);

    let proposers = authorities.into_iter().zip(proposer_pdas).collect();
    DrivenFactVoteDeadend {
        oracle,
        nonce,
        vault,
        proposers,
        agreed_fact,
        agreed_submitter,
        agreed_vote,
        agreed_voter,
        rejected_fact,
        rejected_submitter,
        rejected_vote,
        rejected_voter,
        emission,
        bond_pool,
        supply_before,
        sum_stakes,
    }
}

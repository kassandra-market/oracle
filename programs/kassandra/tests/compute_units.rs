//! Compute-unit (CU) metering + regression guard.
//!
//! Drives the full front-door lifecycle — `init_protocol` → `create_oracle` →
//! `propose` → `finalize_proposals` → `submit_fact` → `advance_phase` →
//! `vote_fact` → `finalize_facts` → `submit_ai_claim` → `finalize_ai_claims` →
//! `finalize_oracle` — through the REAL deployed program in LiteSVM, and records
//! the compute units each instruction consumes (the harness meters every
//! `TestCtx::send`, keyed by instruction discriminant).
//!
//! It then prints a per-instruction CU report (visible with
//! `cargo test -p kassandra-program compute_units -- --nocapture`) and GUARDS
//! each instruction against a budget ceiling, so a change that regresses an
//! instruction's compute cost fails the suite. The ceilings carry headroom over
//! the measured values — bump them deliberately (with the new number in the
//! commit) when an intentional change moves the cost.

mod common;
use common::*;

use kassandra_program::{
    config::{PHASE_WINDOW, PROPOSAL_WINDOW},
    instruction::Ix,
    state::{Phase, VOTE_APPROVE},
};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    signer::keypair::keypair_from_seed,
    system_program,
};
use spl_token::ID as TOKEN_PROGRAM_ID;

/// A DETERMINISTIC keypair from a fixed 32-byte seed. Using fixed keys makes the
/// derived PDAs (proposer / fact-vote / ai-claim) — and therefore the
/// `find_program_address` cost that dominates the account-creating instructions —
/// stable run-to-run, so the CU numbers and budgets below are reproducible.
fn kp(seed: u8) -> Keypair {
    keypair_from_seed(&[seed; 32]).expect("keypair_from_seed")
}

// ----- dispute-core instruction builders (mirror lifecycle_e2e.rs) -----------

fn submit_fact_payload(content_hash: &[u8; 32], stake: u64, uri: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(1 + 32 + 8 + 2 + uri.len());
    data.push(Ix::SubmitFact as u8);
    data.extend_from_slice(content_hash);
    data.extend_from_slice(&stake.to_le_bytes());
    data.extend_from_slice(&(uri.len() as u16).to_le_bytes());
    data.extend_from_slice(uri);
    data
}

fn submit_fact_ix(
    ctx: &TestCtx,
    oracle: Pubkey,
    fact: Pubkey,
    submitter: Pubkey,
    submitter_kass: Pubkey,
    vault: Pubkey,
    data: Vec<u8>,
) -> Instruction {
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(fact, false),
            AccountMeta::new(submitter, true),
            AccountMeta::new(submitter_kass, false),
            AccountMeta::new(vault, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data,
    }
}

fn advance_phase_ix(ctx: &TestCtx, oracle: Pubkey) -> Instruction {
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![AccountMeta::new(oracle, false)],
        data: vec![Ix::AdvancePhase as u8],
    }
}

fn vote_payload(kind: u8, stake: u64) -> Vec<u8> {
    let mut data = Vec::with_capacity(1 + 1 + 8);
    data.push(Ix::VoteFact as u8);
    data.push(kind);
    data.extend_from_slice(&stake.to_le_bytes());
    data
}

#[allow(clippy::too_many_arguments)]
fn vote_fact_ix(
    ctx: &TestCtx,
    oracle: Pubkey,
    fact: Pubkey,
    fact_vote: Pubkey,
    voter: Pubkey,
    voter_kass: Pubkey,
    vault: Pubkey,
    data: Vec<u8>,
) -> Instruction {
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(fact, false),
            AccountMeta::new(fact_vote, false),
            AccountMeta::new(voter, true),
            AccountMeta::new(voter_kass, false),
            AccountMeta::new(vault, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data,
    }
}

fn claim_pda(program_id: &Pubkey, oracle: &Pubkey, proposer: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"claim", oracle.as_ref(), proposer.as_ref()], program_id)
}

fn submit_ai_payload(option: u8) -> Vec<u8> {
    let mut data = Vec::with_capacity(1 + 32 + 32 + 32 + 1);
    data.push(Ix::SubmitAiClaim as u8);
    data.extend_from_slice(&[0xAA; 32]); // model_id
    data.extend_from_slice(&[0xBB; 32]); // params_hash
    data.extend_from_slice(&[0xCC; 32]); // io_hash
    data.push(option);
    data
}

fn submit_ai_claim_ix(
    ctx: &TestCtx,
    oracle: Pubkey,
    proposer: Pubkey,
    claim: Pubkey,
    authority: Pubkey,
    data: Vec<u8>,
) -> Instruction {
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(proposer, false),
            AccountMeta::new(claim, false),
            AccountMeta::new(authority, true),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data,
    }
}

fn finalize_ai_claims_ix(ctx: &TestCtx, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
    let mut accounts = Vec::with_capacity(1 + tail.len());
    accounts.push(AccountMeta::new(oracle, false));
    for k in tail {
        accounts.push(AccountMeta::new(*k, false));
    }
    Instruction {
        program_id: ctx.program_id,
        accounts,
        data: vec![Ix::FinalizeAiClaims as u8],
    }
}

/// Path to the committed golden CU snapshot the metering test compares against.
const SNAPSHOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/compute_units.snap");

/// Guards the hardcoded `guards::PROTOCOL_PDA` (which `load_protocol` uses to skip
/// a `find_program_address`) against program-id drift: re-derive it and compare.
#[test]
fn protocol_pda_const_matches_derivation() {
    let pid = Pubkey::new_from_array(kassandra_program::ID);
    let (derived, bump) = Pubkey::find_program_address(&[b"protocol"], &pid);
    assert_eq!(
        derived.to_bytes(),
        kassandra_program::processor::guards::PROTOCOL_PDA,
        "PROTOCOL_PDA const is stale — re-derive it (did the program id change?)",
    );
    assert_eq!(
        bump,
        kassandra_program::processor::guards::PROTOCOL_BUMP,
        "protocol PDA bump changed",
    );
}

#[test]
fn cu_metering_full_lifecycle_matches_snapshot() {
    let mut ctx = TestCtx::new();
    let bond = 1_000u64;

    // init_protocol + create_oracle → propose×2 (DISTINCT options) →
    // finalize_proposals => FactProposal — driven with FIXED keypairs (via the
    // low-level `ctx.propose`, not the random `propose_real`) so every metered
    // instruction is deterministic.
    let oracle = ctx.create_real_oracle(2, 600);
    let (vault, _) = TestCtx::stake_vault_pda(&ctx.program_id, &oracle);
    let auth0 = kp(1);
    let auth1 = kp(2);
    let (p0, r0) = ctx.propose(oracle, &auth0, 0, bond);
    r0.expect("propose option 0");
    let (p1, r1) = ctx.propose(oracle, &auth1, 1, bond);
    r1.expect("propose option 1");
    ctx.warp(PROPOSAL_WINDOW + 1);
    let fin = ctx.finalize_proposals_ix(oracle, &[p0, p1]);
    ctx.send(fin, &[]).expect("finalize_proposals");
    assert_eq!(ctx.oracle(oracle).phase, Phase::FactProposal.as_u8());
    let proposer_pdas = vec![p0, p1];
    let authorities = vec![auth0, auth1];

    // 1) submit_fact.
    let submitter = kp(10);
    ctx.svm.airdrop(&submitter.pubkey(), 1_000_000_000).unwrap();
    let submitter_kass = ctx.fund_kass(&submitter, 1_000_000);
    let content_hash = [0x07u8; 32];
    let (fact, _) = TestCtx::fact_pda(&ctx.program_id, &oracle, &content_hash);
    let ix = submit_fact_ix(
        &ctx,
        oracle,
        fact,
        submitter.pubkey(),
        submitter_kass,
        vault,
        submit_fact_payload(&content_hash, 100, b"ipfs://fact"),
    );
    ctx.send(ix, &[&submitter]).expect("submit_fact");

    // 2) advance_phase => FactVoting.
    ctx.warp(PHASE_WINDOW + 1);
    let ix = advance_phase_ix(&ctx, oracle);
    ctx.send(ix, &[]).expect("advance_phase");

    // 3) vote_fact (approve, clears the 2/3 quorum of dispute_bond_total = 2000).
    let voter = kp(20);
    ctx.svm.airdrop(&voter.pubkey(), 1_000_000_000).unwrap();
    let voter_kass = ctx.fund_kass(&voter, 10_000);
    let (fact_vote, _) = TestCtx::vote_pda(&ctx.program_id, &fact, &voter.pubkey());
    let ix = vote_fact_ix(
        &ctx,
        oracle,
        fact,
        fact_vote,
        voter.pubkey(),
        voter_kass,
        vault,
        vote_payload(VOTE_APPROVE, 2_000),
    );
    ctx.send(ix, &[&voter]).expect("vote_fact");

    // 4) finalize_facts => AiClaim.
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(ctx.finalize_facts_ix(oracle, &[fact]), &[])
        .expect("finalize_facts");
    assert_eq!(ctx.oracle(oracle).phase, Phase::AiClaim.as_u8());

    // 5) submit_ai_claim (each surviving proposer agrees on option 0).
    for (auth, pda) in authorities.iter().zip(&proposer_pdas) {
        ctx.svm.airdrop(&auth.pubkey(), 1_000_000_000).unwrap();
        let (claim, _) = claim_pda(&ctx.program_id, &oracle, pda);
        let ix = submit_ai_claim_ix(&ctx, oracle, *pda, claim, auth.pubkey(), submit_ai_payload(0));
        ctx.send(ix, &[auth]).expect("submit_ai_claim");
    }

    // 6) finalize_ai_claims => Challenge.
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(finalize_ai_claims_ix(&ctx, oracle, &proposer_pdas), &[])
        .expect("finalize_ai_claims");
    assert_eq!(ctx.oracle(oracle).phase, Phase::Challenge.as_u8());

    // 7) finalize_oracle => Resolved (no challenge opened).
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(ctx.finalize_oracle_ix(oracle, &proposer_pdas), &[])
        .expect("finalize_oracle");
    assert_eq!(ctx.oracle(oracle).phase, Phase::Resolved.as_u8());

    // 8) claim_proposer — the option-0 proposer is correct → bond + reward. Uses
    //    the shared verify_oracle_pda (create_program_address w/ the stored bump).
    let nonce = ctx.oracle_nonce(oracle);
    let dest0 = ctx.fund_kass(&authorities[0], 0);
    let claim = ctx.claim_proposer_ix(oracle, nonce, p0, dest0, vault, authorities[0].pubkey());
    ctx.send(claim, &[]).expect("claim_proposer");

    // --- compare the EXACT CU report against the committed snapshot -----------
    // No arbitrary ceilings: the run is deterministic, so we assert the exact
    // per-instruction CU against a golden file. Any drift — a regression OR an
    // improvement — fails, forcing an explicit re-bless that records the new
    // numbers in the diff:  BLESS_CU=1 cargo test -p kassandra-program \
    //     --test compute_units
    let report = ctx.cu_report();
    print!("{report}");

    if std::env::var_os("BLESS_CU").is_some() {
        std::fs::write(SNAPSHOT, &report).expect("write CU snapshot");
        eprintln!("blessed CU snapshot: {SNAPSHOT}");
        return;
    }
    let expected = std::fs::read_to_string(SNAPSHOT).unwrap_or_else(|_| {
        panic!("missing {SNAPSHOT} — create it with `BLESS_CU=1 cargo test -p kassandra-program --test compute_units`")
    });
    assert_eq!(
        report, expected,
        "\nCU metering changed vs tests/compute_units.snap (see the report above). \
         If intended, re-bless: BLESS_CU=1 cargo test -p kassandra-program --test compute_units\n",
    );
}

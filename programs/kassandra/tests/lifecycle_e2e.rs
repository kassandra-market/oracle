//! End-to-end lifecycle tests (Task H5).
//!
//! These drive the REAL deployed instructions in LiteSVM from the genuine front
//! door — `init_protocol` → `create_oracle` → `propose` → `finalize_proposals`
//! — and, on the conflict path, hand off to the already-built dispute core
//! (`submit_fact` → `advance_phase` → `vote_fact` → `finalize_facts` →
//! `submit_ai_claim` → `finalize_ai_claims` → `finalize_oracle`) all the way to
//! a terminal state. No `set_phase` shortcuts appear in the critical chain; only
//! `warp` advances time. The point is to prove the real entry point composes
//! with the dispute core end-to-end.
//!
//! They also assert the cheap lifecycle invariants along the way: phase
//! transitions happen in order, counters track the set, and KASS is conserved at
//! the proposal-phase boundary (`stake_vault == total_oracle_stake == Σ bonds`,
//! checked BEFORE any `submit_fact` accrues fact stakes into
//! `total_oracle_stake`).

mod common;
use common::*;

use kassandra_program::{
    config::PHASE_WINDOW,
    instruction::Ix,
    state::{Phase, VOTE_APPROVE},
};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    system_program,
};
use spl_token::ID as TOKEN_PROGRAM_ID;

// ----- dispute-core instruction builders (mirror the dedicated test files) ---

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

fn finalize_facts_ix(ctx: &TestCtx, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
    let mut accounts = Vec::with_capacity(1 + tail.len());
    accounts.push(AccountMeta::new(oracle, false));
    for k in tail {
        accounts.push(AccountMeta::new(*k, false));
    }
    Instruction {
        program_id: ctx.program_id,
        accounts,
        data: vec![Ix::FinalizeFacts as u8],
    }
}

/// AiClaim PDA seeds `[b"claim", oracle, proposer]`.
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

fn finalize_oracle_ix(ctx: &TestCtx, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
    // S3 account order (oracle, kass_mint, stake_vault, token program, tail) +
    // the oracle-nonce payload, via the shared harness builder.
    ctx.finalize_oracle_ix(oracle, tail)
}

// ----- E2E: happy (uncontested) path ----------------------------------------

#[test]
fn e2e_happy_uncontested_resolves() {
    let mut ctx = TestCtx::new();
    let payer_kass = ctx.payer_kass;
    let kass_mint = ctx.kass_mint;

    // Genesis creation: fee_ema starts at 0, so the dynamic creation fee is 0.
    let bal_before = ctx.token_balance(payer_kass);
    let supply_before = ctx.mint_supply(kass_mint);

    // init_protocol (once) + create_oracle + warp to the open proposal window.
    let oracle = ctx.create_real_oracle(3, 600);

    assert_eq!(
        ctx.token_balance(payer_kass),
        bal_before,
        "genesis creation fee is 0: no KASS burned from the creator"
    );
    assert_eq!(
        ctx.mint_supply(kass_mint),
        supply_before,
        "genesis creation fee is 0: KASS mint supply unchanged"
    );
    assert_eq!(ctx.oracle(oracle).phase, Phase::Proposal.as_u8());

    // Three proposers, ALL agreeing on option 1.
    let bond = 5_000u64;
    ctx.propose_real(oracle, 1, bond);
    ctx.propose_real(oracle, 1, bond);
    ctx.propose_real(oracle, 1, bond);

    // --- KASS conservation at the proposal boundary (no facts yet) ----------
    let (vault, _) = TestCtx::stake_vault_pda(&ctx.program_id, &oracle);
    let sum_bonds = 3 * bond;
    let o = ctx.oracle(oracle);
    assert_eq!(o.proposer_count, 3);
    assert_eq!(o.surviving_count, 3);
    assert_eq!(
        o.total_oracle_stake, sum_bonds,
        "total_oracle_stake == Σ bonds"
    );
    assert_eq!(
        ctx.token_balance(vault),
        sum_bonds,
        "stake_vault balance == Σ bonds"
    );
    assert_eq!(
        ctx.token_balance(vault),
        o.total_oracle_stake,
        "stake_vault balance == total_oracle_stake"
    );

    // finalize_proposals: all agree => Resolved with the agreed option.
    let res = ctx.finalize_proposals_real(oracle);
    assert!(res.is_ok(), "finalize_proposals should succeed: {res:?}");

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::Resolved.as_u8(), "all-agree => Resolved");
    assert_eq!(o.resolved_option, 1, "resolved_option == agreed option");
    assert_eq!(o.dispute_bond_total, 0, "no dispute opened");
    // No token CPI on the resolve path: the vault is untouched.
    assert_eq!(
        ctx.token_balance(vault),
        sum_bonds,
        "vault untouched on resolve"
    );
}

#[test]
fn e2e_second_oracle_fee_is_burned() {
    // The dynamic EMA fee is 0 at genesis but positive on the next creation in
    // the same context: assert the second oracle's fee was BURNED (the creator's
    // KASS balance AND the mint supply both drop by the same positive amount).
    let mut ctx = TestCtx::new();
    let payer_kass = ctx.payer_kass;
    let kass_mint = ctx.kass_mint;

    // First (genesis) oracle: free.
    let _first = ctx.create_real_oracle(2, 600);

    let bal_before = ctx.token_balance(payer_kass);
    let supply_before = ctx.mint_supply(kass_mint);

    // Second oracle in the same context: fee_ema is now positive => fee burned.
    let _second = ctx.create_real_oracle(2, 600);

    let bal_after = ctx.token_balance(payer_kass);
    let supply_after = ctx.mint_supply(kass_mint);
    let burned = bal_before - bal_after;
    assert!(burned > 0, "second creation must charge a positive fee");
    assert_eq!(
        supply_before - supply_after,
        burned,
        "the fee was BURNED: mint supply dropped by exactly the creator's balance drop"
    );
}

// ----- E2E: dispute path through the dispute core to a terminal state -------

#[test]
fn e2e_dispute_through_dispute_core_to_resolved() {
    let mut ctx = TestCtx::new();
    let bond = 1_000u64;

    // create_oracle → propose×2 (DISTINCT options) → finalize_proposals =>
    // FactProposal with dispute_bond_total set. Driven entirely by real ixs.
    let oracle = ctx.dispute_via_real_flow(&[
        ProposerSpec { option: 0, bond },
        ProposerSpec { option: 1, bond },
    ]);

    let (vault, _) = TestCtx::stake_vault_pda(&ctx.program_id, &oracle);

    // --- KASS conservation at the proposal boundary (BEFORE any submit_fact) -
    let sum_bonds = 2 * bond;
    let o = ctx.oracle(oracle);
    assert_eq!(
        o.phase,
        Phase::FactProposal.as_u8(),
        "conflict => FactProposal"
    );
    assert_eq!(o.proposer_count, 2);
    assert_eq!(
        o.total_oracle_stake, sum_bonds,
        "total_oracle_stake == Σ bonds"
    );
    assert_eq!(
        o.dispute_bond_total, sum_bonds,
        "dispute_bond_total == Σ bonds (fact-quorum denominator)"
    );
    assert_eq!(
        ctx.token_balance(vault),
        sum_bonds,
        "stake_vault == Σ bonds"
    );
    assert_eq!(
        ctx.token_balance(vault),
        o.total_oracle_stake,
        "stake_vault == total_oracle_stake"
    );

    // Capture proposer handles for the dispute core.
    let proposer_pdas: Vec<Pubkey> = ctx.proposers(oracle).iter().map(|p| p.pda).collect();
    let authorities: Vec<Keypair> = ctx
        .proposers(oracle)
        .iter()
        .map(|p| p.authority.insecure_clone())
        .collect();

    // 1) submit_fact (one fact) — FactProposal window still open.
    let submitter = Keypair::new();
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
    ctx.send(ix, &[&submitter])
        .expect("submit_fact should succeed");
    assert_eq!(ctx.oracle(oracle).fact_count, 1);
    assert_eq!(ctx.oracle(oracle).phase, Phase::FactProposal.as_u8());

    // 2) warp past the FactProposal window, advance_phase => FactVoting.
    ctx.warp(PHASE_WINDOW + 1);
    let ix = advance_phase_ix(&ctx, oracle);
    ctx.send(ix, &[]).expect("advance_phase should succeed");
    assert_eq!(ctx.oracle(oracle).phase, Phase::FactVoting.as_u8());

    // 3) vote_fact — approve well past the 2/3 quorum of dispute_bond_total
    //    (2000): approve 2000 clears `approve*3 >= 2000*2`.
    let voter = Keypair::new();
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
    ctx.send(ix, &[&voter]).expect("vote_fact should succeed");

    // 4) warp past the voting window, finalize_facts => AiClaim (fact agreed).
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(finalize_facts_ix(&ctx, oracle, &[fact]), &[])
        .expect("finalize_facts should succeed");
    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::AiClaim.as_u8());
    assert_eq!(ctx.fact(fact).agreed, 1, "the approved fact cleared quorum");

    // 5) submit_ai_claim for each surviving proposer, ALL claiming the SAME
    //    option (0) so the plurality has a clear winner. The proposer that
    //    originally proposed option 1 thereby flips (partial slash, but remains
    //    surviving); the option-0 proposer claims honestly.
    let agreed_option = 0u8;
    for (auth, pda) in authorities.iter().zip(&proposer_pdas) {
        ctx.svm.airdrop(&auth.pubkey(), 1_000_000_000).unwrap();
        let (claim, _) = claim_pda(&ctx.program_id, &oracle, pda);
        let ix = submit_ai_claim_ix(
            &ctx,
            oracle,
            *pda,
            claim,
            auth.pubkey(),
            submit_ai_payload(agreed_option),
        );
        ctx.send(ix, &[auth])
            .expect("submit_ai_claim should succeed");
    }

    // 6) warp past the AiClaim window, finalize_ai_claims => Challenge.
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(finalize_ai_claims_ix(&ctx, oracle, &proposer_pdas), &[])
        .expect("finalize_ai_claims should succeed");
    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::Challenge.as_u8());
    assert_eq!(o.ai_finalized_count, 2);
    // The flipper stays surviving (not disqualified) — both proposers survive.
    assert_eq!(o.surviving_count, 2);
    assert_eq!(o.open_challenge_count, 0, "no challenge opened");

    // 7) warp past the challenge window (open_challenge_count stays 0),
    //    finalize_oracle => Resolved with the agreed claim option.
    ctx.warp(PHASE_WINDOW + 1);
    ctx.send(finalize_oracle_ix(&ctx, oracle, &proposer_pdas), &[])
        .expect("finalize_oracle should succeed");
    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::Resolved.as_u8(), "terminal: Resolved");
    assert_eq!(
        o.resolved_option, agreed_option,
        "resolved_option == the agreed claim option"
    );
}

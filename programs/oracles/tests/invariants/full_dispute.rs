// ---------------------------------------------------------------------------
// Arm A: full randomized dispute flow (no MetaDAO challenge)
// ---------------------------------------------------------------------------

use super::*;

use kassandra_oracles_program::{
    error::KassandraError,
    state::{Phase, CLAIM_OPTION_NONE, VOTE_APPROVE, VOTE_DUPLICATE},
};
use solana_instruction_error::InstructionError;
use solana_keypair::Keypair;
use solana_signer::Signer;
use solana_transaction_error::TransactionError;

fn finalize_facts_ix(ctx: &TestCtx, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
    ctx.finalize_facts_ix(oracle, tail)
}

fn claim_pda(program_id: &Pubkey, oracle: &Pubkey, proposer: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"claim", oracle.as_ref(), proposer.as_ref()], program_id)
}

// ---------------------------------------------------------------------------
// Conservation check (§9 #3, asserted at every step)
// ---------------------------------------------------------------------------

/// At every step the vault balance must equal the on-chain `total_oracle_stake`
/// (no KASS created/destroyed; nothing leaves the vault in this milestone).
fn assert_conservation_step(
    ctx: &TestCtx,
    oracle: Pubkey,
    vault: Pubkey,
) -> Result<(), TestCaseError> {
    let o = ctx.oracle(oracle);
    prop_assert_eq!(
        ctx.token_balance(vault),
        o.total_oracle_stake,
        "vault balance must equal total_oracle_stake (KASS conservation, §9 #3)"
    );
    Ok(())
}

fn custom_err(e: KassandraError) -> TransactionError {
    TransactionError::InstructionError(0, InstructionError::Custom(e as u32))
}

fn run_full_dispute(s: &Scenario) -> Result<(), TestCaseError> {
    let model = ReferenceModel::compute(s);

    let mut ctx = TestCtx::new();
    let specs: Vec<ProposerSpec> = s
        .proposers
        .iter()
        .map(|p| ProposerSpec {
            option: p.original_option,
            bond: p.bond,
        })
        .collect();
    let oracle = ctx.seed_disputed_oracle(&specs);
    let vault = ctx.seeded(oracle).stake_vault;
    let proposer_pdas: Vec<Pubkey> = ctx.proposers(oracle).iter().map(|p| p.pda).collect();

    // Sanity: the harness and program agree on options_count / bond total.
    prop_assert_eq!(ctx.oracle(oracle).options_count, model.options_count);
    prop_assert_eq!(
        ctx.oracle(oracle).dispute_bond_total,
        s.proposers.iter().map(|p| p.bond).sum::<u64>()
    );
    assert_conservation_step(&ctx, oracle, vault)?;

    // §9 #1: an instruction attempted in the wrong phase fails. In FactProposal,
    // finalize_facts (which requires FactVoting) must be rejected.
    {
        let err = ctx
            .send(finalize_facts_ix(&ctx, oracle, &proposer_pdas), &[])
            .unwrap_err()
            .err;
        prop_assert_eq!(err, custom_err(KassandraError::WrongPhase));
    }

    // ---- submit facts (FactProposal window) -------------------------------
    let mut fact_pdas = Vec::with_capacity(s.facts.len());
    for (i, f) in s.facts.iter().enumerate() {
        let submitter = Keypair::new();
        ctx.svm.airdrop(&submitter.pubkey(), 1_000_000_000).unwrap();
        let submitter_kass = ctx.fund_kass(&submitter, f.stake);
        let content_hash = [(i as u8) + 1; 32];
        let (fact, _) = TestCtx::fact_pda(&ctx.program_id, &oracle, &content_hash);
        ctx.send(
            submit_fact_ix(
                &ctx,
                oracle,
                fact,
                submitter.pubkey(),
                submitter_kass,
                vault,
                submit_fact_payload(&content_hash, f.stake, b"ipfs://fact"),
            ),
            &[&submitter],
        )
        .map_err(|e| TestCaseError::fail(format!("submit_fact: {e:?}")))?;
        fact_pdas.push(fact);
        assert_conservation_step(&ctx, oracle, vault)?;
    }

    // ---- advance to FactVoting --------------------------------------------
    ctx.warp(WINDOW + 1);
    ctx.send(advance_phase_ix(&ctx, oracle), &[])
        .map_err(|e| TestCaseError::fail(format!("advance_phase: {e:?}")))?;
    prop_assert_eq!(ctx.oracle(oracle).phase, Phase::FactVoting as u8);

    // §9 #1 / disjoint windows: submitting a fact during FactVoting must fail.
    {
        let submitter = Keypair::new();
        ctx.svm.airdrop(&submitter.pubkey(), 1_000_000_000).unwrap();
        let submitter_kass = ctx.fund_kass(&submitter, 1_000);
        let content_hash = [0xEEu8; 32];
        let (fact, _) = TestCtx::fact_pda(&ctx.program_id, &oracle, &content_hash);
        let err = ctx
            .send(
                submit_fact_ix(
                    &ctx,
                    oracle,
                    fact,
                    submitter.pubkey(),
                    submitter_kass,
                    vault,
                    submit_fact_payload(&content_hash, 1_000, b"x"),
                ),
                &[&submitter],
            )
            .unwrap_err()
            .err;
        prop_assert_eq!(err, custom_err(KassandraError::WrongPhase));
    }

    // ---- cast votes (FactVoting window) -----------------------------------
    for (i, f) in s.facts.iter().enumerate() {
        let fact = fact_pdas[i];
        if f.approve > 0 {
            cast_vote(&mut ctx, oracle, vault, fact, VOTE_APPROVE, f.approve)?;
            assert_conservation_step(&ctx, oracle, vault)?;
        }
        if f.duplicate > 0 {
            cast_vote(&mut ctx, oracle, vault, fact, VOTE_DUPLICATE, f.duplicate)?;
            assert_conservation_step(&ctx, oracle, vault)?;
        }
    }

    // ---- finalize facts ----------------------------------------------------
    ctx.warp(WINDOW + 1);
    if s.facts.is_empty() {
        // No-facts dead-end: pass the full proposer set; terminates immediately.
        ctx.send(finalize_facts_ix(&ctx, oracle, &proposer_pdas), &[])
            .map_err(|e| TestCaseError::fail(format!("finalize_facts (no-facts): {e:?}")))?;

        let o = ctx.oracle(oracle);
        prop_assert_eq!(o.phase, Phase::InvalidDeadend as u8);
        prop_assert_eq!(o.surviving_count, 0);
        prop_assert_eq!(o.bond_pool, model.bond_pool);
        // §9 #3 / uniform identity: every proposer is fully slashed and its
        // slashed_amount == bond (the same value added to bond_pool) on the
        // no-facts dead-end path too — no longer scoped to the AiClaim path.
        let mut slash_total = 0u64;
        for (i, pda) in proposer_pdas.iter().enumerate() {
            let p = ctx.proposer(*pda);
            let (exp_slashed, exp_disq, exp_amount) = model.proposer_expect[i];
            prop_assert_eq!(p.slashed != 0, exp_slashed);
            prop_assert_eq!(p.disqualified != 0, exp_disq);
            prop_assert_eq!(p.slashed_amount, exp_amount);
            slash_total += p.slashed_amount;
        }
        // No facts => no rejected-fact stakes; bond_pool == Σ slashed_amount.
        prop_assert_eq!(o.bond_pool, slash_total);
        prop_assert_eq!(model.slash_total, slash_total);
        // §9 #3 conservation under the dead-end BURN: the no-facts terminal burns
        // the slashed bond_pool (= Σ bonds) out of the vault (a non-outcome
        // distributes nothing; the bonds have no recipient), so the vault drains
        // to 0 (no fact stakes exist on this path). `total_oracle_stake` is the
        // pre-burn accumulator and is unchanged. §9 #9 single terminal reached.
        prop_assert_eq!(
            ctx.token_balance(vault),
            0u64,
            "no-facts dead-end drains the vault (bond_pool burned)"
        );
        prop_assert_eq!(
            o.bond_pool,
            o.total_oracle_stake,
            "burned bond_pool == total_oracle_stake (every bond slashed)"
        );
        prop_assert_eq!(model.terminal, Terminal::DeadendNoFacts);
        prop_assert_eq!(model.total_in, o.total_oracle_stake);
        return Ok(());
    }

    ctx.send(finalize_facts_ix(&ctx, oracle, &fact_pdas), &[])
        .map_err(|e| TestCaseError::fail(format!("finalize_facts: {e:?}")))?;
    prop_assert_eq!(ctx.oracle(oracle).phase, Phase::AiClaim as u8);
    // §9 #6: per-fact classification matches the independent quorum rule.
    for (i, fact) in fact_pdas.iter().enumerate() {
        let f = ctx.fact(*fact);
        match model.fact_class[i] {
            FactClass::Agreed => {
                prop_assert_eq!(f.agreed, 1);
                prop_assert_eq!(f.duplicate, 0);
            }
            FactClass::DuplicateDominant => {
                prop_assert_eq!(f.duplicate, 1);
                prop_assert_eq!(f.agreed, 0);
            }
            FactClass::Rejected => {
                prop_assert_eq!(f.agreed, 0);
                prop_assert_eq!(f.duplicate, 0);
            }
        }
    }
    assert_conservation_step(&ctx, oracle, vault)?;

    // ---- submit AI claims (AiClaim window) --------------------------------
    for (i, p) in s.proposers.iter().enumerate() {
        if p.no_show {
            continue;
        }
        let authority = ctx.proposers(oracle)[i].authority.insecure_clone();
        ctx.svm.airdrop(&authority.pubkey(), 1_000_000_000).unwrap();
        let proposer_pda = proposer_pdas[i];
        let (claim, _) = claim_pda(&ctx.program_id, &oracle, &proposer_pda);
        let option = p.claim_raw % model.options_count;
        ctx.send(
            submit_ai_claim_ix(
                &ctx,
                oracle,
                proposer_pda,
                claim,
                authority.pubkey(),
                submit_ai_payload(option),
            ),
            &[&authority],
        )
        .map_err(|e| TestCaseError::fail(format!("submit_ai_claim: {e:?}")))?;
    }
    assert_conservation_step(&ctx, oracle, vault)?;

    // ---- finalize AI claims -----------------------------------------------
    ctx.warp(WINDOW + 1);
    ctx.send(finalize_ai_claims_ix(&ctx, oracle, &proposer_pdas), &[])
        .map_err(|e| TestCaseError::fail(format!("finalize_ai_claims: {e:?}")))?;
    let o = ctx.oracle(oracle);
    prop_assert_eq!(o.phase, Phase::Challenge as u8);
    prop_assert_eq!(o.surviving_count, model.surviving_count);

    // §9 #3 / #4 / #9: per-proposer slash outcome matches the reference; the
    // internal identity bond_pool == Σ slashed_amount + Σ rejected-fact stakes
    // holds (the slashed_amount identity is uniform across all slash paths).
    let mut on_chain_slash_total = 0u64;
    for (i, pda) in proposer_pdas.iter().enumerate() {
        let p = ctx.proposer(*pda);
        let (exp_slashed, exp_disq, exp_amount) = model.proposer_expect[i];
        prop_assert_eq!(p.slashed != 0, exp_slashed);
        prop_assert_eq!(p.disqualified != 0, exp_disq);
        prop_assert_eq!(p.slashed_amount, exp_amount);
        on_chain_slash_total += p.slashed_amount;
    }
    let rejected_stakes: u64 = s
        .facts
        .iter()
        .zip(&model.fact_class)
        .filter(|(_, &c)| c == FactClass::Rejected)
        .map(|(f, _)| f.stake)
        .sum();
    prop_assert_eq!(o.bond_pool, model.bond_pool);
    prop_assert_eq!(
        o.bond_pool,
        on_chain_slash_total + rejected_stakes,
        "bond_pool == Σ slashed_amount + Σ rejected-fact stakes"
    );
    prop_assert_eq!(model.slash_total, on_chain_slash_total);
    assert_conservation_step(&ctx, oracle, vault)?;

    // §9 #1: finalize_oracle attempted before Challenge window elapsed must fail
    // with WindowNotElapsed (we have NOT warped past the challenge window yet).
    {
        let err = ctx
            .send(finalize_oracle_ix(&ctx, oracle, &proposer_pdas), &[])
            .unwrap_err()
            .err;
        prop_assert_eq!(err, custom_err(KassandraError::WindowNotElapsed));
    }

    // ---- finalize oracle (terminal) ---------------------------------------
    ctx.warp(WINDOW + 1);
    ctx.send(finalize_oracle_ix(&ctx, oracle, &proposer_pdas), &[])
        .map_err(|e| TestCaseError::fail(format!("finalize_oracle: {e:?}")))?;

    let o = ctx.oracle(oracle);
    // §9 #7 + #9: terminal state and resolved_option match the reference.
    match &model.terminal {
        Terminal::Resolved(opt) => {
            prop_assert_eq!(o.phase, Phase::Resolved as u8);
            prop_assert_eq!(o.resolved_option, *opt);
            prop_assert!(
                (o.resolved_option as u16) < model.options_count as u16,
                "resolved option must be in range"
            );
        }
        Terminal::DeadendFromOracle => {
            prop_assert_eq!(o.phase, Phase::InvalidDeadend as u8);
            prop_assert_eq!(o.resolved_option, CLAIM_OPTION_NONE);
        }
        Terminal::DeadendNoFacts => unreachable!("facts present on this path"),
    }

    // §9 #9 terminal exclusivity: phase is exactly one terminal state. On
    // Resolved no KASS moved (bonds remain escrowed counters); on InvalidDeadend
    // finalize_oracle BURNED the slashed bond_pool out of the vault (emission is
    // disabled in this arm), leaving exactly the returnable principal.
    prop_assert!(o.phase == Phase::Resolved as u8 || o.phase == Phase::InvalidDeadend as u8);
    if o.phase == Phase::Resolved as u8 {
        assert_conservation_step(&ctx, oracle, vault)?;
    } else {
        prop_assert_eq!(
            ctx.token_balance(vault) + o.bond_pool,
            o.total_oracle_stake,
            "dead-end burned bond_pool out of the vault (vault + bond_pool == total)"
        );
    }
    prop_assert_eq!(model.total_in, o.total_oracle_stake);

    // §9 #1 idempotency / wrong-phase: a second finalize_oracle now fails.
    {
        let err = ctx
            .send(finalize_oracle_ix(&ctx, oracle, &proposer_pdas), &[])
            .unwrap_err()
            .err;
        prop_assert_eq!(err, custom_err(KassandraError::WrongPhase));
    }

    Ok(())
}

/// Cast a vote of `kind`/`stake` on `fact` from a fresh funded voter.
fn cast_vote(
    ctx: &mut TestCtx,
    oracle: Pubkey,
    vault: Pubkey,
    fact: Pubkey,
    kind: u8,
    stake: u64,
) -> Result<(), TestCaseError> {
    let voter = Keypair::new();
    ctx.svm.airdrop(&voter.pubkey(), 1_000_000_000).unwrap();
    let voter_kass = ctx.fund_kass(&voter, stake);
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
            vote_payload(kind, stake),
        ),
        &[&voter],
    )
    .map_err(|e| TestCaseError::fail(format!("vote_fact: {e:?}")))?;
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 96,
        max_shrink_iters: 128,
        .. ProptestConfig::default()
    })]

    /// Arm A — full randomized dispute flow asserts §9 #1/#2/#3/#6/#7/#9.
    #[test]
    fn full_dispute_invariants(s in scenario_strategy()) {
        run_full_dispute(&s)?;
    }
}

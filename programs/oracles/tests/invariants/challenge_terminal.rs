// ---------------------------------------------------------------------------
// Arm B: plurality / terminal fuzz over an arbitrary post-settlement set
// ---------------------------------------------------------------------------
//
// Arm A can only disqualify via no-show. To fuzz `finalize_oracle`'s plurality
// over an ARBITRARY surviving set (including proposers that submitted a claim but
// were later disqualified by a challenge market), this arm seeds the oracle
// directly into Challenge with a chosen disqualified/surviving partition and
// chosen claim options, then asserts the terminal decision against the same
// independent `ref_plurality`. This broadens §9 #7 / #9 coverage cheaply.

use super::*;

use kassandra_oracles_program::state::{Phase, CLAIM_OPTION_NONE};

#[derive(Clone, Copy, Debug)]
struct ChallengeProposerGen {
    claim: u8,
    disqualified: bool,
}

fn challenge_proposer_strategy() -> impl Strategy<Value = ChallengeProposerGen> {
    (0u8..4, any::<bool>()).prop_map(|(claim, disqualified)| ChallengeProposerGen {
        claim,
        disqualified,
    })
}

fn run_challenge_terminal(proposers: &[ChallengeProposerGen]) -> Result<(), TestCaseError> {
    let mut ctx = TestCtx::new();
    let specs: Vec<ProposerSpec> = proposers
        .iter()
        .map(|p| ProposerSpec {
            option: p.claim.min(2),
            bond: 1_000,
        })
        .collect();
    let oracle = ctx.seed_disputed_oracle(&specs);
    let vault = ctx.seeded(oracle).stake_vault;
    let pdas: Vec<Pubkey> = ctx.proposers(oracle).iter().map(|p| p.pda).collect();

    let mut surviving_claims = Vec::new();
    let mut surviving_count: u16 = 0;
    for (pda, p) in pdas.iter().zip(proposers) {
        if p.disqualified {
            // Disqualified proposers are skipped by finalize_oracle; mark them and
            // give the no-show sentinel (their claim is irrelevant).
            ctx.set_proposer_disqualified(*pda);
            ctx.set_proposer_claim_option(*pda, CLAIM_OPTION_NONE);
        } else {
            ctx.set_proposer_claim_option(*pda, p.claim);
            surviving_claims.push(p.claim);
            surviving_count += 1;
        }
    }
    ctx.set_surviving_count(oracle, surviving_count);
    ctx.set_phase(oracle, Phase::Challenge);

    ctx.warp(WINDOW + 1);
    ctx.send(finalize_oracle_ix(&ctx, oracle, &pdas), &[])
        .map_err(|e| TestCaseError::fail(format!("finalize_oracle: {e:?}")))?;

    let o = ctx.oracle(oracle);
    match ref_plurality(&surviving_claims) {
        Some(opt) => {
            prop_assert_eq!(o.phase, Phase::Resolved as u8);
            prop_assert_eq!(o.resolved_option, opt);
        }
        None => {
            prop_assert_eq!(o.phase, Phase::InvalidDeadend as u8);
            prop_assert_eq!(o.resolved_option, CLAIM_OPTION_NONE);
        }
    }
    // §9 #9: exactly one terminal phase, vault untouched (counter-only).
    prop_assert!(o.phase == Phase::Resolved as u8 || o.phase == Phase::InvalidDeadend as u8);
    prop_assert_eq!(ctx.token_balance(vault), o.total_oracle_stake);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 160,
        max_shrink_iters: 256,
        .. ProptestConfig::default()
    })]

    /// Arm B — plurality / terminal exclusivity over an arbitrary surviving set
    /// (§9 #7/#9), modelling an arbitrary post-challenge disqualification set.
    #[test]
    fn challenge_terminal_invariants(
        proposers in prop::collection::vec(challenge_proposer_strategy(), 2..=6)
    ) {
        run_challenge_terminal(&proposers)?;
    }
}

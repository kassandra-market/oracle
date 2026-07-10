use super::*;

use kassandra_oracles_program::state::CLAIM_OPTION_NONE;

// ===========================================================================
// Tests 6-7 (DS2) — end-to-end FACT/VOTE dead-end conservation, REAL DRIVEN.
// ===========================================================================
//
// The DS1 conservation arms (deadend_settlement.rs + invariants Arm F) proved
// the dead-end burn over PROPOSER slashes (challenge-disqualify / flip). The
// FACT/VOTE dead-end claim path — a REJECTED fact (its submitter stake → the
// burned bond_pool) AND a SLASHED approve-voter on that rejected fact (the
// slashed fraction → the burned bond_pool), alongside an AGREED fact (returned)
// — was only validated by the harness mirror in Arm E (a seeded post-burn
// vault), NOT by a real on-chain finalize-burn-then-claim.
//
// These two tests close that gap. They DRIVE a real dispute through the genuine
// front door (create_oracle → propose → finalize_proposals → submit_fact×2 →
// advance_phase → vote_fact×2 → finalize_facts → submit_ai_claim×2 →
// finalize_ai_claims → finalize_oracle; only `warp` moves time) to a Tie
// dead-end that carries:
//   * an AGREED fact (submitter + approve-voter both reclaim full stake),
//   * a REJECTED fact whose submitter forfeits (→ 0) and whose lone approve-voter
//     is slashed (the on-chain rejected-fact voter slash, fact_vote_slash 1/2),
// then assert the REAL `finalize_oracle` burned the slashed `bond_pool` (= the
// rejected submitter stake + the FLOOR aggregate voter slash) + the emission, and
// the REAL S2 claims pay the per-actor matrix and drain the vault to bounded dust.
//
// The rejected-fact approve stake is ODD (501) ON PURPOSE: `finalize_facts`
// credits `bond_pool` with the FLOOR aggregate `floor(501·1/2) = 250`, but
// `claim_fact_vote` slashes the lone voter `ceil(501·1/2) = 251`. That floor-vs-
// ceil asymmetry is the conservation safety margin — the vault retains exactly
// `ceil − floor = 1` as conservation-SAFE dust (never short). This proves the
// rounding choice end-to-end on a real driven path, not just by the argument in
// `claims::slash_amount`. Test 6 is a plain InvalidDeadend; test 7 is the
// governance-resolved (`resolve_deadend` → Resolved) path, which MUST pay
// IDENTICALLY (the no-marker insight, here on the fact/vote path).

/// `ceil(value · num / den)` — the per-voter rejected-fact slash (mirrors
/// `claims::slash_amount`).
fn ceil_slash(value: u64, num: u64, den: u64) -> u64 {
    (value as u128 * num as u128).div_ceil(den as u128) as u64
}

/// Run the full S2 claim sweep over a driven fact/vote dead-end and assert the
/// per-actor matrix + the floor-vs-ceil conservation-safe dust + the full
/// conservation equation. Works for BOTH terminal phases (plain InvalidDeadend
/// and governance-resolved Resolved) — the payouts are identical because
/// `reward_pool == 0` on both.
fn assert_fact_vote_deadend_drains(ctx: &mut TestCtx, d: &DrivenFactVoteDeadend) {
    let o = ctx.oracle(d.oracle);
    assert_eq!(o.reward_pool, 0, "no reward distribution out of a dead-end");

    // The slashed bond_pool + the emission were BURNED back to the reservoir.
    assert_eq!(
        ctx.mint_supply(ctx.kass_mint),
        d.supply_before - d.bond_pool - d.emission,
        "bond_pool (rejected stake + voter floor-slash) + emission burned"
    );
    // Post-burn vault == Σ stakes − bond_pool (the returnable non-slashed
    // principal; the emission was burned too, netting to Σ stakes − bond_pool).
    let vault_after_burn = ctx.token_balance(d.vault);
    assert_eq!(vault_after_burn, d.sum_stakes - d.bond_pool);

    let mut returned = 0u64;

    // --- AGREED fact: approve-voter then submitter both reclaim full stake ---
    let av = ctx.fact_vote(d.agreed_vote);
    let dest = ctx.fund_kass(&d.agreed_voter, 0);
    ctx.send(
        ctx.claim_fact_vote_ix(
            d.oracle,
            d.nonce,
            d.agreed_vote,
            d.agreed_fact,
            dest,
            d.vault,
            d.agreed_voter.pubkey(),
        ),
        &[],
    )
    .expect("claim_fact_vote (agreed)");
    assert_eq!(
        ctx.token_balance(dest),
        av.stake,
        "agreed approve-voter: full stake (no reward, dead-end)"
    );
    returned += av.stake;

    let af = ctx.fact(d.agreed_fact);
    let dest = ctx.fund_kass(&d.agreed_submitter, 0);
    ctx.send(
        ctx.claim_fact_ix(
            d.oracle,
            d.nonce,
            d.agreed_fact,
            dest,
            d.vault,
            d.agreed_submitter.pubkey(),
        ),
        &[],
    )
    .expect("claim_fact (agreed)");
    assert_eq!(
        ctx.token_balance(dest),
        af.stake,
        "agreed submitter: full stake (no reward, dead-end)"
    );
    assert!(ctx.is_closed(d.agreed_fact));
    returned += af.stake;

    // --- REJECTED fact: voter slashed ceil; submitter forfeits to 0 ----------
    let rv = ctx.fact_vote(d.rejected_vote);
    let voter_slash = ceil_slash(rv.stake, FACT_VOTE_SLASH_NUM, FACT_VOTE_SLASH_DEN);
    let dest = ctx.fund_kass(&d.rejected_voter, 0);
    ctx.send(
        ctx.claim_fact_vote_ix(
            d.oracle,
            d.nonce,
            d.rejected_vote,
            d.rejected_fact,
            dest,
            d.vault,
            d.rejected_voter.pubkey(),
        ),
        &[],
    )
    .expect("claim_fact_vote (rejected)");
    assert_eq!(
        ctx.token_balance(dest),
        rv.stake - voter_slash,
        "rejected approve-voter: stake − ceil(stake·1/2)"
    );
    returned += rv.stake - voter_slash;

    let dest = ctx.fund_kass(&d.rejected_submitter, 0);
    ctx.send(
        ctx.claim_fact_ix(
            d.oracle,
            d.nonce,
            d.rejected_fact,
            dest,
            d.vault,
            d.rejected_submitter.pubkey(),
        ),
        &[],
    )
    .expect("claim_fact (rejected)");
    assert_eq!(
        ctx.token_balance(dest),
        0,
        "rejected submitter forfeits (stake funded the burned bond_pool)"
    );
    assert!(ctx.is_closed(d.rejected_fact), "rejected fact still closes");
    // returned += 0;

    // --- proposers: bond − slashed_amount (no flip slash here → full bond) ---
    for (auth, pda) in &d.proposers {
        let p = ctx.proposer(*pda);
        let expected = p.bond - p.slashed_amount;
        let dest = ctx.fund_kass(auth, 0);
        ctx.send(
            ctx.claim_proposer_ix(d.oracle, d.nonce, *pda, dest, d.vault, auth.pubkey()),
            &[],
        )
        .expect("claim_proposer");
        assert_eq!(
            ctx.token_balance(dest),
            expected,
            "survivor: bond − slashed"
        );
        assert!(ctx.is_closed(*pda));
        returned += expected;
    }

    // --- conservation: dust is exactly the floor-vs-ceil voter-slash margin --
    let dust = ctx.token_balance(d.vault);
    let ceil_margin = voter_slash - floor_slash(rv.stake, FACT_VOTE_SLASH_NUM, FACT_VOTE_SLASH_DEN);
    assert_eq!(
        returned + dust,
        vault_after_burn,
        "Σ returned + dust == post-burn vault"
    );
    assert_eq!(
        dust, ceil_margin,
        "dust == ceil(stake·1/2) − floor(stake·1/2) (conservation-safe, never short)"
    );
    // Full equation: Σ returned + dust + bond_pool_burned + emission_burned
    //   == Σ stakes + emission.
    assert_eq!(
        returned + dust + d.bond_pool + d.emission,
        d.sum_stakes + d.emission,
        "Σ returned + dust + bond_pool_burned + emission_burned == Σ stakes + emission"
    );
}

// ---------------------------------------------------------------------------
// Test 6 — plain InvalidDeadend, real fact/vote dead-end conservation.
// ---------------------------------------------------------------------------

#[test]
fn e2e_fact_vote_deadend_burns_and_drains_real_dispute() {
    let mut ctx = TestCtx::new();
    let d = drive_real_fact_vote_deadend(&mut ctx);
    assert_eq!(
        ctx.oracle(d.oracle).resolved_option,
        CLAIM_OPTION_NONE,
        "plain dead-end carries the sentinel"
    );
    assert_fact_vote_deadend_drains(&mut ctx, &d);
}

// ---------------------------------------------------------------------------
// Test 7 — governance-resolved (resolve_deadend → Resolved) fact/vote dead-end:
// pays IDENTICALLY to the plain InvalidDeadend (reward_pool == 0 ⇒ no rewards),
// drains the same, no marker / claim-path divergence on the fact/vote path.
// ---------------------------------------------------------------------------

#[test]
fn e2e_fact_vote_deadend_governance_resolved_pays_identically() {
    let mut ctx = TestCtx::new();
    let d = drive_real_fact_vote_deadend(&mut ctx);

    // Governance force-resolves the dead-end to option 1 → Resolved (the burn
    // already happened at finalize; resolve_deadend moves no tokens).
    let dao = Keypair::new();
    ctx.airdrop(&dao, 1_000_000_000);
    let (_da, kass_dao) = TestCtx::stand_in_governance(0x44);
    ctx.force_governance(dao.pubkey(), kass_dao);
    let (_p, res) = ctx.resolve_deadend(d.oracle, &dao, 1);
    assert!(res.is_ok(), "resolve_deadend should succeed: {res:?}");
    let o = ctx.oracle(d.oracle);
    assert_eq!(o.phase, Phase::Resolved as u8, "phase flipped to Resolved");
    assert_eq!(o.resolved_option, 1, "governance option recorded");
    assert_eq!(
        o.reward_pool, 0,
        "still no reward pool on a resolved-from-dead-end"
    );

    // Same claim sweep, same payouts, same dust — proving the no-marker insight
    // holds on the FACT/VOTE dead-end path too.
    assert_fact_vote_deadend_drains(&mut ctx, &d);
}

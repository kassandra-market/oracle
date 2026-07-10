//! DS1 — dead-end economic settlement: a terminal `InvalidDeadend` oracle (and a
//! governance-resolved-from-dead-end oracle) FULLY DRAINS its stake vault.
//!
//! These exercise the burns added at the InvalidDeadend finalize sites:
//! * `finalize_no_facts` burns the slashed `bond_pool` (= Σ bonds) AND the
//!   `reward_emission` when a no-facts dispute terminates (user decision: the
//!   propose-conflict-then-abandon bonds are burned, not redistributed).
//! * `finalize_oracle` (tie / no survivors) burns the slashed `bond_pool` (incl.
//!   proposer flip / challenge slashes) AND the `reward_emission`.
//!
//! After the burns the vault holds EXACTLY the returnable non-slashed principal,
//! so the S2 claims drain it to dust — on BOTH a plain `InvalidDeadend` and a
//! governance-resolved (`resolve_deadend` → `Resolved`) oracle, with NO marker /
//! layout / claim-path divergence (verified here: both drain identically).

mod common;
use common::*;

use kassandra_oracles_program::state::{Phase, CLAIM_OPTION_NONE};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

// ---------------------------------------------------------------------------
// Arm 1 — no-facts dead-end: bonds + emission BURNED, every proposer claims 0.
// ---------------------------------------------------------------------------

#[test]
fn no_facts_deadend_burns_bonds_and_emission_full_drain() {
    let mut ctx = TestCtx::new();
    // Two conflicting proposers (distinct options), no facts ever submitted.
    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: 1_000,
        },
        ProposerSpec {
            option: 1,
            bond: 2_000,
        },
    ]);
    let pdas: Vec<Pubkey> = ctx.proposers(oracle).iter().map(|p| p.pda).collect();
    let auths: Vec<Keypair> = ctx
        .proposers(oracle)
        .iter()
        .map(|p| p.authority.insecure_clone())
        .collect();
    let total_bonds = 3_000u64;
    let emission = 444u64;
    ctx.set_reward_emission(oracle, emission);
    // Drive the no-facts dead-end: enter FactVoting (fact_count == 0) then finalize.
    ctx.set_phase(oracle, Phase::FactVoting);

    let vault = ctx.seeded(oracle).stake_vault;
    let nonce = ctx.seeded(oracle).nonce;
    let supply_before = ctx.mint_supply(ctx.kass_mint);
    assert_eq!(ctx.token_balance(vault), total_bonds + emission);

    ctx.warp(WINDOW + 1);
    ctx.send(ctx.finalize_facts_ix(oracle, &pdas), &[])
        .expect("finalize_facts (no-facts) should succeed");

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::InvalidDeadend as u8);
    assert_eq!(o.surviving_count, 0);
    assert_eq!(
        o.bond_pool, total_bonds,
        "every bond slashed into bond_pool"
    );
    // The bond_pool (= Σ bonds) AND the emission were BURNED: supply drops by both.
    assert_eq!(
        ctx.mint_supply(ctx.kass_mint),
        supply_before - total_bonds - emission,
        "bonds + emission burned back to the reservoir"
    );
    // Vault drained: a no-facts dead-end has NO returnable principal (every bond
    // was misbehavior-slashed; the emission funds no reward).
    assert_eq!(
        ctx.token_balance(vault),
        0,
        "no-facts dead-end vault fully drained by the burn"
    );

    // Every (now disqualified) proposer's claim returns 0 — its bond was burned.
    for (auth, pda) in auths.iter().zip(&pdas) {
        let p = ctx.proposer(*pda);
        assert!(p.disqualified != 0 && p.slashed != 0);
        assert_eq!(p.slashed_amount, p.bond, "no-facts slash == full bond");
        let dest = ctx.fund_kass(auth, 0);
        ctx.send(
            ctx.claim_proposer_ix(oracle, nonce, *pda, dest, vault, auth.pubkey()),
            &[],
        )
        .expect("claim_proposer should succeed (and close)");
        assert_eq!(
            ctx.token_balance(dest),
            0,
            "disqualified no-facts proposer claims 0 (bond burned)"
        );
        assert!(ctx.is_closed(*pda), "claimant account closed");
    }
    assert_eq!(ctx.token_balance(vault), 0, "vault → dust (fully drained)");
}

// ---------------------------------------------------------------------------
// Arm 2 — tie dead-end WITH slashes (finalize_oracle): survivors get
// bond − slashed_amount, the slashed bond_pool is burned, the vault drains.
// ---------------------------------------------------------------------------

/// Seed a 3-proposer dispute where P0 was challenge-disqualified (kass_fee left
/// the vault), P1 is a flip-slashed SURVIVOR, P2 is an honest survivor, and the
/// two survivors claim DISTINCT options → the surviving plurality ties →
/// InvalidDeadend. Returns `(ctx, oracle, pdas, auths, flip_slash, kass_fee)`.
fn seed_tie_with_slashes() -> (TestCtx, Pubkey, Vec<Pubkey>, Vec<Keypair>, u64, u64) {
    let mut ctx = TestCtx::new();
    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: 1_000,
        },
        ProposerSpec {
            option: 0,
            bond: 2_000,
        },
        ProposerSpec {
            option: 1,
            bond: 2_000,
        },
    ]);
    let pdas: Vec<Pubkey> = ctx.proposers(oracle).iter().map(|p| p.pda).collect();
    let auths: Vec<Keypair> = ctx
        .proposers(oracle)
        .iter()
        .map(|p| p.authority.insecure_clone())
        .collect();

    // P0: challenge-disqualified (kass_fee=100 left the vault; slashed 900).
    let kass_fee = 100u64;
    ctx.seed_challenge_disqualify(oracle, pdas[0], kass_fee);
    // P1: flip-slashed but SURVIVING (slash 400 into bond_pool).
    let flip_slash = 400u64;
    ctx.set_proposer_prior_slash(oracle, pdas[1], flip_slash);
    // Survivors P1/P2 claim distinct options → tie → dead-end.
    ctx.set_proposer_claim_option(pdas[1], 0);
    ctx.set_proposer_claim_option(pdas[2], 1);
    ctx.set_phase(oracle, Phase::Challenge);
    (ctx, oracle, pdas, auths, flip_slash, kass_fee)
}

#[test]
fn tie_deadend_with_slashes_burns_bond_pool_full_drain() {
    let (mut ctx, oracle, pdas, auths, flip_slash, kass_fee) = seed_tie_with_slashes();
    let emission = 333u64;
    ctx.set_reward_emission(oracle, emission);

    let vault = ctx.seeded(oracle).stake_vault;
    let nonce = ctx.seeded(oracle).nonce;
    let supply_before = ctx.mint_supply(ctx.kass_mint);
    let bond_pool = ctx.oracle(oracle).bond_pool;
    // bond_pool = P0 slash (900) + P1 flip slash (400).
    assert_eq!(bond_pool, (1_000 - kass_fee) + flip_slash);
    // Vault = Σ bonds (5000) − kass_fee (100) + emission.
    assert_eq!(ctx.token_balance(vault), 5_000 - kass_fee + emission);

    ctx.warp(WINDOW + 1);
    ctx.send(ctx.finalize_oracle_ix(oracle, &pdas), &[])
        .expect("finalize_oracle should succeed");

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::InvalidDeadend as u8);
    assert_eq!(o.resolved_option, CLAIM_OPTION_NONE);
    assert_eq!(o.reward_pool, 0, "no reward distribution out of a dead-end");
    // Both the slashed bond_pool AND the emission were burned.
    assert_eq!(
        ctx.mint_supply(ctx.kass_mint),
        supply_before - bond_pool - emission,
        "slashed bond_pool + emission burned"
    );
    // Vault now holds EXACTLY the survivors' returnable principal:
    //   P1: 2000 − 400 (flip) = 1600 ; P2: 2000. (P0's 900 burned; kass_fee gone.)
    let returnable = (2_000 - flip_slash) + 2_000;
    assert_eq!(ctx.token_balance(vault), returnable);

    // Claims: P0 (disqualified) → 0; P1 → bond − flip_slash; P2 → full bond.
    let mut total_claimed = 0u64;
    for (i, (auth, pda)) in auths.iter().zip(&pdas).enumerate() {
        let p = ctx.proposer(*pda);
        let expected = if p.disqualified != 0 {
            0
        } else {
            p.bond - p.slashed_amount
        };
        match i {
            0 => assert_eq!(expected, 0, "disqualified P0 forfeits"),
            1 => assert_eq!(expected, 2_000 - flip_slash, "flip survivor: bond − slash"),
            _ => assert_eq!(expected, 2_000, "honest survivor: full bond"),
        }
        let dest = ctx.fund_kass(auth, 0);
        ctx.send(
            ctx.claim_proposer_ix(oracle, nonce, *pda, dest, vault, auth.pubkey()),
            &[],
        )
        .expect("claim_proposer");
        assert_eq!(ctx.token_balance(dest), expected);
        total_claimed += expected;
    }
    assert_eq!(
        total_claimed, returnable,
        "Σ payouts == returnable principal"
    );
    assert_eq!(ctx.token_balance(vault), 0, "vault → dust (fully drained)");
}

// ---------------------------------------------------------------------------
// Arm 3 — governance-resolved dead-end: resolve_deadend → Resolved pays the SAME
// (non-slashed principal only, no reward), vault still drains, option recorded.
// ---------------------------------------------------------------------------

#[test]
fn governance_resolved_deadend_pays_identically_and_drains() {
    let (mut ctx, oracle, pdas, auths, flip_slash, _kass_fee) = seed_tie_with_slashes();
    // Hand governance off to a signable DAO keypair (mirrors resolve_deadend.rs).
    ctx.ensure_protocol();
    let dao = Keypair::new();
    ctx.airdrop(&dao, 1_000_000_000);
    let (_da, kass_dao) = TestCtx::stand_in_governance(0x44);
    ctx.force_governance(dao.pubkey(), kass_dao);

    let vault = ctx.seeded(oracle).stake_vault;
    let nonce = ctx.seeded(oracle).nonce;
    let bond_pool = ctx.oracle(oracle).bond_pool;

    // Drive the dead-end (finalize_oracle burns bond_pool — no emission here).
    ctx.warp(WINDOW + 1);
    ctx.send(ctx.finalize_oracle_ix(oracle, &pdas), &[])
        .expect("finalize_oracle");
    assert_eq!(ctx.oracle(oracle).phase, Phase::InvalidDeadend as u8);
    let returnable = (2_000 - flip_slash) + 2_000;
    assert_eq!(ctx.token_balance(vault), returnable, "post-burn vault");

    // Governance force-resolves the dead-end to option 1 → Resolved.
    let (_p, res) = ctx.resolve_deadend(oracle, &dao, 1);
    assert!(res.is_ok(), "resolve_deadend should succeed: {res:?}");
    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::Resolved as u8, "phase flipped to Resolved");
    assert_eq!(o.resolved_option, 1, "governance option recorded");
    assert_eq!(
        o.reward_pool, 0,
        "still no reward pool on a resolved-from-dead-end"
    );
    // The burn already happened at finalize; resolve_deadend moves no tokens.
    assert_eq!(
        ctx.token_balance(vault),
        returnable,
        "vault unchanged by resolve"
    );

    // Claims on the Resolved-from-dead-end pay IDENTICALLY to InvalidDeadend:
    // non-slashed principal, ZERO reward (reward_pool == 0 ⇒ all reward terms 0),
    // even for a survivor whose claim_option == the governance-chosen option.
    let mut total_claimed = 0u64;
    for (i, (auth, pda)) in auths.iter().zip(&pdas).enumerate() {
        let p = ctx.proposer(*pda);
        let expected = if p.disqualified != 0 {
            0
        } else {
            p.bond - p.slashed_amount
        };
        // P2 claimed option 1 == resolved_option, yet earns NO reward (pool 0).
        if i == 2 {
            assert_eq!(p.claim_option, o.resolved_option);
            assert_eq!(
                expected, 2_000,
                "correct-option survivor still gets only its bond"
            );
        }
        let dest = ctx.fund_kass(auth, 0);
        ctx.send(
            ctx.claim_proposer_ix(oracle, nonce, *pda, dest, vault, auth.pubkey()),
            &[],
        )
        .expect("claim_proposer");
        assert_eq!(ctx.token_balance(dest), expected);
        total_claimed += expected;
    }
    assert_eq!(total_claimed, returnable);
    assert_eq!(
        ctx.token_balance(vault),
        0,
        "governance-resolved dead-end ALSO fully drains (no marker needed)"
    );
    // The burned bond_pool record is unchanged by resolution.
    assert_eq!(ctx.oracle(oracle).bond_pool, bond_pool);
}

use super::*;

use kassandra_oracles_program::state::Phase;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

// ---------------------------------------------------------------------------
// Test 3 — emission ENABLED, Resolved: real finalize folds emission into the
// reward pool, claims reflect the emission-boosted reward, conservation includes
// the emission. (Dispute SEEDED; finalize_oracle fold + claims + emission REAL.)
// ---------------------------------------------------------------------------

#[test]
fn e2e_resolved_with_emission_real_finalize_and_claims() {
    let mut ctx = TestCtx::new();
    // Two proposers, both claim option 1 (the winner). No flip, no slash, so
    // bond_pool == 0 and the WHOLE reward pool is the creation-time emission.
    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 1,
            bond: 1_000,
        },
        ProposerSpec {
            option: 1,
            bond: 3_000,
        },
    ]);
    let pdas: Vec<Pubkey> = ctx.proposers(oracle).iter().map(|p| p.pda).collect();
    let auths: Vec<Keypair> = ctx
        .proposers(oracle)
        .iter()
        .map(|p| p.authority.insecure_clone())
        .collect();
    for p in &pdas {
        ctx.set_proposer_claim_option(*p, 1);
    }
    ctx.set_phase(oracle, Phase::Challenge);

    // The creation-time emission (placed in the vault, backed by supply). The REAL
    // finalize_oracle folds it into reward_pool below.
    let emission = 900u64;
    ctx.set_reward_emission(oracle, emission);

    let vault = ctx.seeded(oracle).stake_vault;
    let nonce = ctx.seeded(oracle).nonce;
    let vault_initial = ctx.token_balance(vault);
    assert_eq!(vault_initial, 4_000 + emission, "Σ bonds + emission");

    // REAL finalize_oracle → Resolved, folding the emission into reward_pool.
    ctx.warp(WINDOW + 1);
    ctx.send(ctx.finalize_oracle_ix(oracle, &pdas), &[])
        .expect("finalize_oracle");
    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::Resolved as u8);
    assert_eq!(
        o.reward_pool,
        o.bond_pool + emission,
        "reward_pool folds emission"
    );
    assert_eq!(
        o.reward_pool, emission,
        "bond_pool 0 → pool is pure emission"
    );
    assert_eq!(o.total_correct_proposer_stake, 4_000);

    let pbucket = proposer_bucket_of(&o);
    let mut total_claimed = 0u64;
    for (auth, pda) in auths.iter().zip(&pdas) {
        let p = ctx.proposer(*pda);
        let reward = reward::proposer_reward(p.bond, pbucket, o.total_correct_proposer_stake);
        assert!(reward > 0, "emission funds a positive proposer reward");
        let expected = p.bond + reward;
        let dest = ctx.fund_kass(auth, 0);
        let ix = ctx.claim_proposer_ix(oracle, nonce, *pda, dest, vault, auth.pubkey());
        ctx.send(ix, &[]).expect("claim_proposer");
        assert_eq!(
            ctx.token_balance(dest),
            expected,
            "bond + emission-funded reward"
        );
        total_claimed += expected;
    }

    // Conservation INCLUDES the emission: Σ claims + dust == Σ stakes + emission.
    let dust = ctx.token_balance(vault);
    assert_eq!(
        total_claimed + dust,
        vault_initial,
        "Σ claims + dust == Σ stakes + emission"
    );
    assert!(dust <= emission, "dust ≤ emission (floor remainder)");
}

// ---------------------------------------------------------------------------
// Test 4 — emission ENABLED, InvalidDeadend: real finalize BURNS the emission
// back (supply returns), every staker reclaims full stake. (Dispute SEEDED;
// finalize_oracle burn + claims REAL.)
// ---------------------------------------------------------------------------

#[test]
fn e2e_invalid_deadend_emission_burned_full_returns() {
    let mut ctx = TestCtx::new();
    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: 1_500,
        },
        ProposerSpec {
            option: 1,
            bond: 2_500,
        },
    ]);
    let pdas: Vec<Pubkey> = ctx.proposers(oracle).iter().map(|p| p.pda).collect();
    let auths: Vec<Keypair> = ctx
        .proposers(oracle)
        .iter()
        .map(|p| p.authority.insecure_clone())
        .collect();
    ctx.set_proposer_claim_option(pdas[0], 0);
    ctx.set_proposer_claim_option(pdas[1], 1); // tie → InvalidDeadend
    ctx.set_phase(oracle, Phase::Challenge);

    let emission = 777u64;
    ctx.set_reward_emission(oracle, emission);

    let vault = ctx.seeded(oracle).stake_vault;
    let nonce = ctx.seeded(oracle).nonce;
    let supply_before = ctx.mint_supply(ctx.kass_mint);
    assert_eq!(ctx.token_balance(vault), 4_000 + emission);

    // REAL finalize_oracle → InvalidDeadend, burning the emission back.
    ctx.warp(WINDOW + 1);
    ctx.send(ctx.finalize_oracle_ix(oracle, &pdas), &[])
        .expect("finalize_oracle");
    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::InvalidDeadend as u8);
    assert_eq!(o.reward_pool, 0);
    assert_eq!(
        ctx.token_balance(vault),
        4_000,
        "emission burned out of the vault"
    );
    assert_eq!(
        ctx.mint_supply(ctx.kass_mint),
        supply_before - emission,
        "burn-back returned the emission to the reservoir"
    );

    // Full returns: every proposer reclaims its whole bond, vault drains to 0.
    let vault_after_burn = ctx.token_balance(vault);
    let mut total_claimed = 0u64;
    for (auth, pda) in auths.iter().zip(&pdas) {
        let bond = ctx.proposer(*pda).bond;
        let dest = ctx.fund_kass(auth, 0);
        let ix = ctx.claim_proposer_ix(oracle, nonce, *pda, dest, vault, auth.pubkey());
        ctx.send(ix, &[]).expect("claim_proposer");
        assert_eq!(ctx.token_balance(dest), bond, "full bond back on dead-end");
        total_claimed += bond;
    }
    assert_eq!(total_claimed, vault_after_burn, "Σ payouts == Σ stakes");
    assert_eq!(ctx.token_balance(vault), 0, "vault fully drained");
}

// ---------------------------------------------------------------------------
// Test 5 — the S3-flagged combination: InvalidDeadend AFTER a settled challenge,
// WITH emission present. Verifies the burn-back of BOTH emission AND the slashed
// bond_pool (with NO double-count of the kass_fee already paid out at settle) +
// full survivor returns + the forfeit of the challenge-disqualified proposer all
// conserve and FULLY DRAIN the vault (no stranding), plus close_market /
// close_ai_claim. (Dispute + challenge SEEDED to the post-settle state; finalize
// burn + claims + closes REAL.)
// ---------------------------------------------------------------------------

#[test]
fn e2e_deadend_after_settled_challenge_with_emission() {
    let mut ctx = TestCtx::new();
    // Three proposers. P0 was successfully challenged (disqualified, kass_fee left
    // the vault to the challenger). P1/P2 survive but claim DISTINCT options → the
    // surviving plurality ties → InvalidDeadend.
    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: 1_000,
        },
        ProposerSpec {
            option: 0,
            bond: 1_000,
        },
        ProposerSpec {
            option: 1,
            bond: 1_000,
        },
    ]);
    let pdas: Vec<Pubkey> = ctx.proposers(oracle).iter().map(|p| p.pda).collect();
    let auths: Vec<Keypair> = ctx
        .proposers(oracle)
        .iter()
        .map(|p| p.authority.insecure_clone())
        .collect();

    // P0: settled-challenge disqualify. kass_fee = 100 left the vault; bond_pool
    // gains bond − kass_fee == 900; surviving_count drops to 2.
    let kass_fee = 100u64;
    ctx.seed_challenge_disqualify(oracle, pdas[0], kass_fee);

    // Survivors tie: P1 claims 0, P2 claims 1.
    ctx.set_proposer_claim_option(pdas[1], 0);
    ctx.set_proposer_claim_option(pdas[2], 1);
    ctx.set_phase(oracle, Phase::Challenge);

    // Emission present at creation (placed in the vault, backed by supply).
    let emission = 555u64;
    ctx.set_reward_emission(oracle, emission);

    let vault = ctx.seeded(oracle).stake_vault;
    let nonce = ctx.seeded(oracle).nonce;
    let supply_before = ctx.mint_supply(ctx.kass_mint);
    // P0's settled-challenge slash sits in bond_pool (= bond − kass_fee == 900).
    let bond_pool = ctx.oracle(oracle).bond_pool;
    assert_eq!(bond_pool, 1_000 - kass_fee, "P0 slash in bond_pool");
    // Vault = Σ bonds (3000) − kass_fee (100) + emission (555).
    let vault_initial = ctx.token_balance(vault);
    assert_eq!(vault_initial, 3_000 - kass_fee + emission);

    // Seed a SETTLED Market + empty escrow + an AiClaim for the disqualified P0,
    // so the closes have something to reclaim.
    let challenger = Keypair::new();
    ctx.airdrop(&challenger, 1_000_000_000);
    let escrow = ctx.seed_usdc_escrow(oracle, 0);
    let market = ctx.seed_market(oracle, challenger.pubkey(), escrow, true);
    let ai_claim = ctx.seed_ai_claim(oracle, pdas[0], auths[0].pubkey());

    // REAL finalize_oracle → InvalidDeadend, burning BOTH the emission AND the
    // slashed bond_pool (P0's 900) back. Crucially NO double-count: the kass_fee
    // (100) already left the vault to the challenger at settle time and was
    // recorded as `bond − kass_fee` in bond_pool, so burning bond_pool burns only
    // the 900 still physically in the vault.
    ctx.warp(WINDOW + 1);
    ctx.send(ctx.finalize_oracle_ix(oracle, &pdas), &[])
        .expect("finalize_oracle");
    let o = ctx.oracle(oracle);
    assert_eq!(
        o.phase,
        Phase::InvalidDeadend as u8,
        "deadend after settled challenge"
    );
    assert_eq!(o.reward_pool, 0);
    assert_eq!(
        ctx.mint_supply(ctx.kass_mint),
        supply_before - emission - bond_pool,
        "emission AND slashed bond_pool burned back (no double-count of the kass_fee)"
    );
    let vault_after_burn = ctx.token_balance(vault);
    assert_eq!(
        vault_after_burn,
        3_000 - kass_fee - bond_pool,
        "vault = Σ bonds − kass_fee_out − burned bond_pool == survivors' returnable principal"
    );
    assert_eq!(
        vault_after_burn, 2_000,
        "exactly P1 + P2's returnable bonds"
    );

    // Claims: P0 (disqualified) forfeits (0); P1/P2 reclaim full bonds.
    let mut total_claimed = 0u64;
    for (i, (auth, pda)) in auths.iter().zip(&pdas).enumerate() {
        let p = ctx.proposer(*pda);
        let expected = if p.disqualified != 0 { 0 } else { p.bond };
        if i == 0 {
            assert_eq!(expected, 0, "disqualified P0 forfeits the whole bond");
        }
        let dest = ctx.fund_kass(auth, 0);
        let ix = ctx.claim_proposer_ix(oracle, nonce, *pda, dest, vault, auth.pubkey());
        ctx.send(ix, &[]).expect("claim_proposer");
        assert_eq!(ctx.token_balance(dest), expected);
        assert!(ctx.is_closed(*pda));
        total_claimed += expected;
    }

    // The disqualified P0's `bond − kass_fee` (900) was BURNED (it funded the now-
    // burned bond_pool), so unlike before it is NOT stranded as dust: the vault
    // fully drains to 0. The kass_fee (100) had already left to the challenger at
    // settle time.
    let dust = ctx.token_balance(vault);
    assert_eq!(total_claimed, 2_000, "P1 + P2 full bonds");
    assert_eq!(
        dust, 0,
        "no stranding: P0's forfeited bond_pool was burned, vault drained"
    );
    // KASS conservation across the WHOLE settled-challenge dead-end:
    //   vault_after_burn == Σ payouts + dust.
    assert_eq!(
        total_claimed + dust,
        vault_after_burn,
        "Σ payouts + dust == post-burn vault"
    );
    assert_eq!(
        total_claimed + dust + kass_fee + emission + bond_pool,
        3_000 + emission,
        "full KASS accounting: payouts + dust + kass_fee_out + emission_burned + bond_pool_burned == Σ bonds + emission",
    );

    // ---- REAL closes: AiClaim + Market + escrow rent reclamation ---------------
    let p0_auth_before = ctx.lamports(auths[0].pubkey());
    let ai_rent = ctx.lamports(ai_claim);
    ctx.send(
        ctx.close_ai_claim_ix(oracle, ai_claim, auths[0].pubkey()),
        &[],
    )
    .expect("close_ai_claim");
    assert!(ctx.is_closed(ai_claim));
    assert_eq!(
        ctx.lamports(auths[0].pubkey()),
        p0_auth_before + ai_rent,
        "AiClaim rent → authority"
    );

    let chal_before = ctx.lamports(challenger.pubkey());
    let market_rent = ctx.lamports(market);
    let escrow_rent = ctx.lamports(escrow);
    ctx.send(
        ctx.close_market_ix(oracle, nonce, market, escrow, challenger.pubkey()),
        &[],
    )
    .expect("close_market");
    assert!(ctx.is_closed(market));
    assert!(ctx.is_closed(escrow));
    assert_eq!(
        ctx.lamports(challenger.pubkey()),
        chal_before + market_rent + escrow_rent,
        "Market + escrow rents → challenger",
    );
}

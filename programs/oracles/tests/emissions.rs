//! Task S3 emission tests: the KASS mint-authority bootstrap, mint-at-creation
//! from the supply reservoir, the InvalidDeadend burn-back, and the
//! mint-authority guard.
//!
//! `create_oracle` mints `reward_emission = (total_supply_cap − kass_supply) ·
//! emission_num/den` into the new oracle's `stake_vault` AFTER the EMA fee burn
//! (so burning boosts the same-tx reservoir), program-signed by the mint-
//! authority PDA. On `Resolved`, `finalize_oracle` folds it into `reward_pool`;
//! on `InvalidDeadend`, it burns it back. Emission is disabled (no mint) at
//! genesis (`total_supply_cap == 0`) and enabled by governance via `set_config`.

mod common;
use common::*;

use kassandra_oracles_program::{
    error::KassandraError,
    reward,
    state::{Phase, CLAIM_OPTION_NONE},
};
use solana_instruction_error::InstructionError;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction_error::TransactionError;

/// Recommended-curve test values: cap 2e15 (the harness funds the payer 1e15, so
/// the reservoir is ~1e15) at rate 1/1_000_000 → a clean 1e9-scale emission.
const CAP: u64 = 2_000_000_000_000_000;
const NUM: u64 = 1;
const DEN: u64 = 1_000_000;

/// floor((cap − supply)·num/den), the on-chain emission formula.
fn emission_for(supply: u64, cap: u64, num: u64, den: u64) -> u64 {
    ((cap as u128 - supply as u128) * num as u128 / den as u128) as u64
}

/// init_protocol + governance handoff (dao_authority = payer) + a `set_config`
/// that OVERWRITES the emission params with a chosen `(cap, num, den)`.
/// `init_protocol` now enables emission by default (the `config.rs` consts); this
/// helper lets a test pin an EXACT curve for deterministic emission sizing (or
/// DISABLE emission by passing `cap == 0` / `num == 0`).
fn enable_emission(ctx: &mut TestCtx, cap: u64, num: u64, den: u64) {
    let (_p, res) = ctx.init_protocol();
    assert!(res.is_ok(), "init_protocol: {res:?}");
    // Record the payer (a SIGNABLE key) as `dao_authority` directly so it can
    // sign the set_config below; the Task G1-hardened handoff only accepts the
    // derived (unsignable) Squads vault PDA.
    let payer = ctx.payer.insecure_clone();
    ctx.force_governance(payer.pubkey(), Pubkey::new_unique());
    let mut params = ConfigParams::defaults();
    params.total_supply_cap = cap;
    params.emission_num = num;
    params.emission_den = den;
    let (_p, res) = ctx.set_config(&payer, params);
    assert!(res.is_ok(), "set_config: {res:?}");
}

#[test]
fn create_oracle_mints_emission_into_vault() {
    let mut ctx = TestCtx::new();
    enable_emission(&mut ctx, CAP, NUM, DEN);

    let supply_before = ctx.mint_supply(ctx.kass_mint);
    let deadline = ctx.now() + 1_000;
    let (oracle, res) = ctx.create_oracle(0, 2, deadline, 600);
    assert!(res.is_ok(), "create_oracle: {res:?}");

    // Genesis creation → fee 0 → no burn; emission on the full reservoir.
    let expected = emission_for(supply_before, CAP, NUM, DEN);
    assert!(expected > 0, "test must exercise a positive emission");
    let o = ctx.oracle(oracle);
    assert_eq!(
        o.reward_emission, expected,
        "oracle.reward_emission recorded"
    );

    // Supply rose by exactly the emission (the reservoir shrank by it).
    assert_eq!(ctx.mint_supply(ctx.kass_mint), supply_before + expected);
    assert_eq!(
        CAP - ctx.mint_supply(ctx.kass_mint),
        CAP - supply_before - expected
    );

    // The minted KASS is physically in the stake_vault.
    let (vault, _) = TestCtx::stake_vault_pda(&ctx.program_id, &oracle);
    assert_eq!(ctx.token_balance(vault), expected);
}

#[test]
fn fee_burn_boosts_emission() {
    let mut ctx = TestCtx::new();
    enable_emission(&mut ctx, CAP, NUM, DEN);

    // Genesis creation is free (no burn).
    let deadline = ctx.now() + 1_000_000;
    let (_o0, res) = ctx.create_oracle(0, 2, deadline, 600);
    assert!(res.is_ok(), "genesis create: {res:?}");

    // Second rapid creation: a positive fee is burned BEFORE the emission mint.
    let bal_pre = ctx.token_balance(ctx.payer_kass);
    let supply_pre = ctx.mint_supply(ctx.kass_mint);
    let (o1, res) = ctx.create_oracle(1, 2, deadline, 600);
    assert!(res.is_ok(), "second create: {res:?}");

    let fee = bal_pre - ctx.token_balance(ctx.payer_kass);
    assert!(fee > 0, "a second rapid creation burns a fee");

    let e1 = ctx.oracle(o1).reward_emission;
    // Emission is computed on the POST-burn supply (`supply_pre − fee`): the burn
    // enlarged the same-tx reservoir.
    let expected_post = emission_for(supply_pre - fee, CAP, NUM, DEN);
    assert_eq!(e1, expected_post, "emission uses the post-burn supply");
    // Strictly more than the pre-burn reservoir would have yielded — proving the
    // burn ran before the mint (else e1 would equal `expected_pre`).
    let expected_pre = emission_for(supply_pre, CAP, NUM, DEN);
    assert!(
        e1 > expected_pre,
        "burning first boosts the emission: {e1} <= {expected_pre}"
    );
}

#[test]
fn resolved_folds_emission_into_reward_pool_and_claim() {
    let mut ctx = TestCtx::new();
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
    for p in &pdas {
        ctx.set_proposer_claim_option(*p, 1);
    }
    ctx.set_phase(oracle, Phase::Challenge);

    let emission = 600u64;
    ctx.set_reward_emission(oracle, emission);

    let vault = ctx.seeded(oracle).stake_vault;
    let vault_before = ctx.token_balance(vault);
    assert_eq!(
        vault_before,
        4_000 + emission,
        "Σ bonds + emission in vault"
    );

    ctx.warp(WINDOW + 1);
    let ix = ctx.finalize_oracle_ix(oracle, &pdas);
    ctx.send(ix, &[]).expect("finalize should succeed");

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::Resolved as u8);
    // reward_pool folds the emission in: bond_pool (0 here) + reward_emission.
    assert_eq!(o.reward_pool, o.bond_pool + emission);
    assert_eq!(o.reward_pool, emission);
    assert_eq!(o.total_correct_proposer_stake, 4_000);
    // No burn on Resolved: the emission stays in the vault for the reward claims.
    assert_eq!(ctx.token_balance(vault), vault_before);

    // Chain S1→S2: a correct proposer's claim reflects the emission-boosted pool.
    let (pbucket, _) = reward::reward_buckets(
        o.reward_pool,
        o.reward_proposer_weight,
        o.reward_fact_weight,
        o.total_correct_proposer_stake,
        o.total_approved_fact_stake,
    );
    let auth0 = ctx.proposers(oracle)[0].authority.insecure_clone();
    let bond0 = ctx.proposers(oracle)[0].bond;
    let pda0 = ctx.proposers(oracle)[0].pda;
    let nonce = ctx.seeded(oracle).nonce;
    let dest = ctx.fund_kass(&auth0, 0);
    let ix = ctx.claim_proposer_ix(oracle, nonce, pda0, dest, vault, auth0.pubkey());
    ctx.send(ix, &[]).expect("claim should succeed");

    let expected_reward = reward::proposer_reward(bond0, pbucket, o.total_correct_proposer_stake);
    assert!(expected_reward > 0, "emission funds a positive reward");
    assert_eq!(
        ctx.token_balance(dest),
        bond0 + expected_reward,
        "claim = bond + emission-funded reward"
    );
}

#[test]
fn invalid_deadend_burns_emission_back() {
    let mut ctx = TestCtx::new();
    let oracle = ctx.seed_disputed_oracle(&[
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
    ctx.set_proposer_claim_option(pdas[0], 0);
    ctx.set_proposer_claim_option(pdas[1], 1); // tie → InvalidDeadend
    ctx.set_phase(oracle, Phase::Challenge);

    let emission = 700u64;
    ctx.set_reward_emission(oracle, emission);

    let vault = ctx.seeded(oracle).stake_vault;
    let supply_before = ctx.mint_supply(ctx.kass_mint);
    assert_eq!(ctx.token_balance(vault), 2_000 + emission);

    ctx.warp(WINDOW + 1);
    let ix = ctx.finalize_oracle_ix(oracle, &pdas);
    ctx.send(ix, &[]).expect("finalize should succeed");

    let o = ctx.oracle(oracle);
    assert_eq!(o.phase, Phase::InvalidDeadend as u8);
    assert_eq!(o.resolved_option, CLAIM_OPTION_NONE);
    assert_eq!(o.reward_pool, 0, "no reward distribution out of a dead-end");
    // The emission was burned back: vault returns to Σ stakes, supply drops by it.
    assert_eq!(
        ctx.token_balance(vault),
        2_000,
        "emission burned out of the vault"
    );
    assert_eq!(
        ctx.mint_supply(ctx.kass_mint),
        supply_before - emission,
        "burn-back returned the emission to the reservoir"
    );
    // The stamp is left as the durable record of what was minted then burned.
    assert_eq!(o.reward_emission, emission);
}

#[test]
fn mint_authority_mismatch_rejected() {
    let mut ctx = TestCtx::new();
    enable_emission(&mut ctx, CAP, NUM, DEN);
    // Point the canonical KASS mint's authority at a non-PDA key: emission can no
    // longer be trusted, so the mint at create_oracle is rejected.
    let payer = ctx.payer.pubkey();
    ctx.set_kass_mint_authority(payer);

    let deadline = ctx.now() + 1_000;
    let (_o, res) = ctx.create_oracle(0, 2, deadline, 600);
    let err = res.unwrap_err().err;
    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(KassandraError::BadMintAuthority as u32),
        ),
    );
}

#[test]
fn cap_zero_emits_nothing() {
    // Emission is ON by default now, so the DISABLED path must be configured
    // explicitly: governance `set_config` with total_supply_cap == 0 →
    // `compute_reward_emission` short-circuits to 0 (harmless). The
    // mint-authority guard is never reached (no mint), so this also proves a
    // disabled-emission create_oracle is unaffected by the PDA mint authority.
    let mut ctx = TestCtx::new();
    enable_emission(&mut ctx, 0, NUM, DEN);

    let supply_before = ctx.mint_supply(ctx.kass_mint);
    let deadline = ctx.now() + 1_000;
    let (oracle, res) = ctx.create_oracle(0, 2, deadline, 600);
    assert!(res.is_ok(), "create with cap 0: {res:?}");

    assert_eq!(ctx.oracle(oracle).reward_emission, 0);
    assert_eq!(
        ctx.mint_supply(ctx.kass_mint),
        supply_before,
        "supply unchanged"
    );
    let (vault, _) = TestCtx::stake_vault_pda(&ctx.program_id, &oracle);
    assert_eq!(ctx.token_balance(vault), 0, "no emission minted");
}

#[test]
fn emission_num_zero_emits_nothing() {
    // A non-zero cap but emission_num == 0 → disabled (the other disabled knob).
    let mut ctx = TestCtx::new();
    enable_emission(&mut ctx, CAP, 0, DEN);

    let supply_before = ctx.mint_supply(ctx.kass_mint);
    let deadline = ctx.now() + 1_000;
    let (oracle, res) = ctx.create_oracle(0, 2, deadline, 600);
    assert!(res.is_ok(), "create with emission_num 0: {res:?}");

    assert_eq!(ctx.oracle(oracle).reward_emission, 0);
    assert_eq!(ctx.mint_supply(ctx.kass_mint), supply_before);
}

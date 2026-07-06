//! Task SW1 — `sweep_oracle` (Ix 22): permissionless, grace-gated dust sweep +
//! terminal `Oracle`/`stake_vault` closure.
//!
//! Once a terminal oracle is past `phase_ends_at + SWEEP_GRACE` and governance
//! is set, anyone may crank the sweep: the ENTIRE residual vault balance (dust —
//! or a no-show staker's forfeited principal) is transferred to the DAO treasury
//! (the KASS ATA of `dao_authority`), then the vault + oracle are closed with
//! both rents refunded to `oracle.creator`. These tests cover the happy path,
//! every gate (grace / governance / treasury / creator / vault / phase), the
//! stark FORFEITURE trade-off (a no-show's principal → treasury, later claim
//! fails), and idempotency by closure.

mod common;
use common::*;

use kassandra_program::{config::SWEEP_GRACE, error::KassandraError, state::Phase};
use solana_instruction_error::InstructionError;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction_error::TransactionError;

/// The custom-error expectation for a failed instruction at index 0.
fn custom(e: KassandraError) -> TransactionError {
    TransactionError::InstructionError(0, InstructionError::Custom(e as u32))
}

/// A sweepable fixture: a terminal `Resolved` oracle whose vault holds
/// `vault_dust` KASS, governance set to a fresh `dao_authority`, its treasury
/// ATA fabricated, a fresh (non-payer) `creator` recorded as rent recipient, and
/// the clock warped PAST `phase_ends_at + SWEEP_GRACE`. Returns the pieces the
/// tests need.
struct Sweepable {
    seed: TerminalSeed,
    protocol: Pubkey,
    treasury: Pubkey,
    creator: Keypair,
}

impl Sweepable {
    /// `proposers` lets a test seed an UNCLAIMED staker (the forfeiture arm); an
    /// empty slice yields a pure-dust vault. `warp` toggles whether the clock is
    /// advanced past the grace (false drives the before-grace rejection).
    fn build(
        ctx: &mut TestCtx,
        proposers: &[ClaimProposerSpec],
        vault_dust: u64,
        governance: bool,
        warp: bool,
    ) -> Self {
        let seed = ctx.seed_terminal_oracle(Phase::Resolved, 1, proposers, &[], 1, 2);
        if vault_dust > 0 {
            ctx.fund_vault(seed.oracle, vault_dust);
        }

        // Fresh rent recipient (distinct from the fee payer, so lamport deltas
        // are exact).
        let creator = Keypair::new();
        ctx.airdrop(&creator, 1_000_000_000);
        ctx.set_creator(seed.oracle, creator.pubkey());

        // Governance handoff: record a fresh dao_authority + fabricate its KASS
        // treasury ATA.
        ctx.ensure_protocol();
        let dao_authority = Pubkey::new_unique();
        let protocol = if governance {
            let kass_dao = Pubkey::new_unique();
            ctx.force_governance(dao_authority, kass_dao)
        } else {
            let (p, _) = TestCtx::protocol_pda(&ctx.program_id);
            p
        };
        let treasury = ctx.seed_kass_treasury(dao_authority);

        if warp {
            ctx.warp(SWEEP_GRACE + 1);
        }

        Self {
            seed,
            protocol,
            treasury,
            creator,
        }
    }

    fn sweep_ix(&self, ctx: &TestCtx) -> solana_instruction::Instruction {
        ctx.sweep_oracle_ix(
            self.seed.oracle,
            self.seed.nonce,
            self.seed.stake_vault,
            self.protocol,
            self.treasury,
            self.creator.pubkey(),
            None,
        )
    }
}

// ---------------------------------------------------------------------------
// Happy path
// ---------------------------------------------------------------------------

#[test]
fn sweep_after_grace_routes_dust_and_closes() {
    let mut ctx = TestCtx::new();
    let dust = 7u64;
    let f = Sweepable::build(&mut ctx, &[], dust, true, true);

    let vault_rent = ctx.lamports(f.seed.stake_vault);
    let oracle_rent = ctx.lamports(f.seed.oracle);
    let creator_before = ctx.lamports(f.creator.pubkey());
    assert!(vault_rent > 0 && oracle_rent > 0);
    assert_eq!(ctx.token_balance(f.seed.stake_vault), dust);
    assert_eq!(ctx.token_balance(f.treasury), 0);

    let ix = f.sweep_ix(&ctx);
    ctx.send(ix, &[]).unwrap();

    // Dust routed to the treasury ATA; both accounts closed; rent → creator.
    assert_eq!(ctx.token_balance(f.treasury), dust, "dust → treasury ATA");
    assert!(ctx.is_closed(f.seed.stake_vault), "vault closed");
    assert!(ctx.is_closed(f.seed.oracle), "oracle closed");
    assert_eq!(
        ctx.lamports(f.creator.pubkey()),
        creator_before + vault_rent + oracle_rent,
        "both rents → creator",
    );
}

#[test]
fn sweep_empty_vault_still_closes() {
    // A vault with zero residual (no dust) still closes both accounts (the
    // Transfer is a no-op).
    let mut ctx = TestCtx::new();
    let f = Sweepable::build(&mut ctx, &[], 0, true, true);
    assert_eq!(ctx.token_balance(f.seed.stake_vault), 0);

    let ix = f.sweep_ix(&ctx);
    ctx.send(ix, &[]).unwrap();

    assert_eq!(ctx.token_balance(f.treasury), 0);
    assert!(ctx.is_closed(f.seed.stake_vault));
    assert!(ctx.is_closed(f.seed.oracle));
}

// ---------------------------------------------------------------------------
// Gates
// ---------------------------------------------------------------------------

#[test]
fn sweep_before_grace_fails() {
    let mut ctx = TestCtx::new();
    // Governance set, dust present, but NOT warped past the grace.
    let f = Sweepable::build(&mut ctx, &[], 7, true, false);

    let ix = f.sweep_ix(&ctx);
    assert_eq!(
        ctx.send(ix, &[]).unwrap_err().err,
        custom(KassandraError::SweepGraceNotElapsed),
    );
}

#[test]
fn sweep_governance_not_set_fails() {
    let mut ctx = TestCtx::new();
    // Past grace, but governance NOT set.
    let f = Sweepable::build(&mut ctx, &[], 7, false, true);

    let ix = f.sweep_ix(&ctx);
    assert_eq!(
        ctx.send(ix, &[]).unwrap_err().err,
        custom(KassandraError::GovernanceNotSet),
    );
}

#[test]
fn sweep_wrong_treasury_fails() {
    let mut ctx = TestCtx::new();
    let f = Sweepable::build(&mut ctx, &[], 7, true, true);

    // A KASS ATA of a DIFFERENT owner — not ATA(dao_authority, kass_mint).
    let wrong = ctx.seed_kass_treasury(Pubkey::new_unique());
    let ix = ctx.sweep_oracle_ix(
        f.seed.oracle,
        f.seed.nonce,
        f.seed.stake_vault,
        f.protocol,
        wrong,
        f.creator.pubkey(),
        None,
    );
    assert_eq!(
        ctx.send(ix, &[]).unwrap_err().err,
        custom(KassandraError::InvalidTreasury),
    );
}

#[test]
fn sweep_wrong_creator_fails() {
    let mut ctx = TestCtx::new();
    let f = Sweepable::build(&mut ctx, &[], 7, true, true);

    let ix = ctx.sweep_oracle_ix(
        f.seed.oracle,
        f.seed.nonce,
        f.seed.stake_vault,
        f.protocol,
        f.treasury,
        Pubkey::new_unique(), // not oracle.creator
        None,
    );
    assert_eq!(
        ctx.send(ix, &[]).unwrap_err().err,
        custom(KassandraError::InvalidAccount),
    );
}

#[test]
fn sweep_wrong_stake_vault_fails() {
    let mut ctx = TestCtx::new();
    let f = Sweepable::build(&mut ctx, &[], 7, true, true);

    let ix = ctx.sweep_oracle_ix(
        f.seed.oracle,
        f.seed.nonce,
        Pubkey::new_unique(), // not oracle.stake_vault
        f.protocol,
        f.treasury,
        f.creator.pubkey(),
        None,
    );
    assert_eq!(
        ctx.send(ix, &[]).unwrap_err().err,
        custom(KassandraError::InvalidAccount),
    );
}

#[test]
fn sweep_non_terminal_fails() {
    let mut ctx = TestCtx::new();
    let f = Sweepable::build(&mut ctx, &[], 7, true, true);
    // Force a non-terminal phase.
    ctx.set_phase(f.seed.oracle, Phase::Challenge);

    let ix = f.sweep_ix(&ctx);
    assert_eq!(
        ctx.send(ix, &[]).unwrap_err().err,
        custom(KassandraError::WrongPhase),
    );
}

// ---------------------------------------------------------------------------
// Forfeiture trade-off + idempotency
// ---------------------------------------------------------------------------

#[test]
fn sweep_forfeits_unclaimed_principal_then_claim_fails() {
    // A terminal oracle with an UNCLAIMED staker: the vault holds their full
    // principal. Swept after grace, the FULL balance (principal + dust) goes to
    // the treasury, the oracle + vault close, and the no-show's subsequent claim
    // fails on the closed oracle. This is the starkly-documented trade-off.
    let mut ctx = TestCtx::new();
    let bond = 1_000u64;
    let dust = 3u64;
    let f = Sweepable::build(
        &mut ctx,
        &[ClaimProposerSpec {
            bond,
            claim_option: 1,
            disqualified: false,
            slashed_amount: 0,
        }],
        dust,
        true,
        true,
    );

    // The vault holds the unclaimed proposer's principal + the added dust.
    let vault_before = ctx.token_balance(f.seed.stake_vault);
    assert_eq!(vault_before, bond + dust);

    let proposer = &f.seed.proposers[0];
    let p_account = proposer.account;
    let p_dest = proposer.dest_kass;
    let p_authority = proposer.authority.pubkey();

    let ix = f.sweep_ix(&ctx);
    ctx.send(ix, &[]).unwrap();

    // FULL balance forfeited to the treasury; both accounts closed.
    assert_eq!(
        ctx.token_balance(f.treasury),
        bond + dust,
        "no-show principal + dust → treasury",
    );
    assert!(ctx.is_closed(f.seed.oracle));
    assert!(ctx.is_closed(f.seed.stake_vault));

    // The late claimant can no longer claim — the oracle is gone.
    let claim = ctx.claim_proposer_ix(
        f.seed.oracle,
        f.seed.nonce,
        p_account,
        p_dest,
        f.seed.stake_vault,
        p_authority,
    );
    assert_eq!(
        ctx.send(claim, &[]).unwrap_err().err,
        custom(KassandraError::InvalidAccount),
        "claim on a swept (closed) oracle fails",
    );
}

#[test]
fn sweep_idempotent_second_call_fails() {
    let mut ctx = TestCtx::new();
    let f = Sweepable::build(&mut ctx, &[], 7, true, true);

    let ix = f.sweep_ix(&ctx);
    ctx.send(ix, &[]).unwrap();
    assert!(ctx.is_closed(f.seed.oracle));

    // Second sweep: the oracle is reaped → load guard fails.
    let ix2 = f.sweep_ix(&ctx);
    assert_eq!(
        ctx.send(ix2, &[]).unwrap_err().err,
        custom(KassandraError::InvalidAccount),
    );
}

//! `create_oracle`: stand up a new oracle in [`Phase::Proposal`] with a future
//! `deadline` plus its program-controlled stake vault.
//!
//! The stake vault is an SPL token account on the canonical KASS mint, created
//! at PDA `[b"vault", oracle]` and program-signed; its SPL authority is the
//! oracle PDA, so later instructions (`propose`/`open_challenge`/...) can sign
//! transfers out of it via the oracle seeds. The canonical mints are pinned from
//! the [`Protocol`] singleton, so an oracle cannot be created against a spoofed
//! KASS mint (this is what makes the Task H2 fee-burn trustworthy).
//!
//! # Creation fee (Task H2 / design §8)
//! A KASS fee proportional to an EMA of recent creation activity is BURNED from
//! the creator's KASS token account. The [`Protocol`] carries the fixed-point
//! `fee_ema` accumulator: on each creation we decay it toward 0 by the elapsed
//! idle time, charge `fee = FEE_PER_EMA_UNIT * decayed_ema / FEE_EMA_SCALE`,
//! burn it (creator signs as the burn authority), then bump the EMA by one
//! creation unit and stamp `last_creation_unix`. The first-ever creation has
//! `fee_ema == 0` → fee 0 (genesis is free). See [`crate::fee`] / [`crate::config`].
//!
//! # PDA seeds (CONTRACT)
//! * Oracle: `[b"oracle", &nonce.to_le_bytes()]`, program = [`crate::ID`].
//! * Stake vault: `[b"vault", oracle_pubkey]`, program = [`crate::ID`].
//!
//! # Accounts
//! 0. protocol            — writable; pins the canonical mints + holds/updates `fee_ema`
//! 1. oracle PDA          — writable, uninitialized (created here)
//! 2. stake_vault PDA     — writable, uninitialized (created + initialized here)
//! 3. creator             — signer, writable; pays rent, recorded as `creator`, burn authority
//! 4. kass_mint           — writable (burn decrements supply); must equal `protocol.kass_mint`
//! 5. usdc_mint           — must equal `protocol.usdc_mint`
//! 6. token program
//! 7. system program
//! 8. creator_kass_token  — writable; KASS token account on `kass_mint` the fee is burned from
//!
//! # Instruction payload (after the 1-byte discriminant), exactly 57 bytes
//! `nonce: u64 LE` ++ `prompt_hash: [u8; 32]` ++ `options_count: u8` ++
//! `deadline: i64 LE` ++ `twap_window: i64 LE`.

use bytemuck::Zeroable;
use pinocchio::{
    account_info::AccountInfo,
    instruction::Seed,
    program_error::ProgramError,
    pubkey::{find_program_address, Pubkey},
    sysvars::{rent::Rent, Sysvar},
    ProgramResult,
};
use pinocchio_token::instructions::{Burn, InitializeAccount3};

use crate::{
    clock::now,
    error::KassandraError,
    fee::{bumped_fee_ema, decay_fee_ema, fee_for_ema},
    processor::guards::{assert_key, assert_signer, create_pda, load_protocol},
    state::{AccountType, Oracle, Phase, Protocol},
};

/// Exact payload length: nonce[8] ++ prompt_hash[32] ++ options_count[1] ++
/// deadline[8] ++ twap_window[8].
const PAYLOAD_LEN: usize = 57;

/// SPL token account size (`spl_token::state::Account::LEN`).
const SPL_TOKEN_ACCOUNT_LEN: usize = 165;

pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], payload: &[u8]) -> ProgramResult {
    // --- payload parse (exact length) --------------------------------------
    if payload.len() != PAYLOAD_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let nonce = u64::from_le_bytes(payload[0..8].try_into().unwrap());
    let mut prompt_hash = [0u8; 32];
    prompt_hash.copy_from_slice(&payload[8..40]);
    let options_count = payload[40];
    let deadline = i64::from_le_bytes(payload[41..49].try_into().unwrap());
    let twap_window = i64::from_le_bytes(payload[49..57].try_into().unwrap());

    let [protocol_ai, oracle_ai, stake_vault_ai, creator_ai, kass_mint_ai, usdc_mint_ai, token_prog_ai, system_prog_ai, creator_kass_ai, ..] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // --- signer + program ids ----------------------------------------------
    assert_signer(creator_ai)?;
    assert_key(token_prog_ai, &pinocchio_token::ID)?;
    assert_key(system_prog_ai, &pinocchio_system::ID)?;

    // --- canonical mints pinned from the protocol singleton ----------------
    let protocol = load_protocol(protocol_ai, program_id)?;
    assert_key(kass_mint_ai, &protocol.kass_mint)?;
    assert_key(usdc_mint_ai, &protocol.usdc_mint)?;

    // --- semantic validations ----------------------------------------------
    let now_ts = now()?;
    if options_count < 2 {
        return Err(KassandraError::InvalidOptionsCount.into());
    }
    if deadline < now_ts {
        return Err(KassandraError::InvalidDeadline.into());
    }
    if twap_window <= 0 {
        return Err(ProgramError::InvalidInstructionData);
    }

    // --- PDA derivations ----------------------------------------------------
    let nonce_le = nonce.to_le_bytes();
    let (expected_oracle, oracle_bump) = find_program_address(&[b"oracle", &nonce_le], program_id);
    assert_key(oracle_ai, &expected_oracle)?;

    let (expected_vault, vault_bump) =
        find_program_address(&[b"vault", oracle_ai.key().as_ref()], program_id);
    assert_key(stake_vault_ai, &expected_vault)?;

    // Reject if the oracle PDA already exists (a duplicate nonce).
    if oracle_ai.lamports() != 0 || !oracle_ai.data_is_empty() {
        return Err(KassandraError::InvalidAccount.into());
    }

    // --- dynamic EMA creation fee (burned in KASS) -------------------------
    // Decay the stored activity EMA toward 0 by the idle time since the last
    // creation, charge a fee proportional to it, burn it, then record the bumped
    // EMA + timestamp. Genesis (`fee_ema == 0`) decays to 0 → fee 0 → no burn.
    let decayed_ema = decay_fee_ema(protocol.fee_ema, protocol.last_creation_unix, now_ts);
    let fee = fee_for_ema(decayed_ema);
    if fee > 0 {
        // The burn source must be a KASS token account; the SPL Burn additionally
        // proves the creator (signer) is its owner/delegate.
        let kass_token_mint = {
            let data = creator_kass_ai.try_borrow_data()?;
            if data.len() < 32 {
                return Err(KassandraError::InvalidAccount.into());
            }
            let mut m = [0u8; 32];
            m.copy_from_slice(&data[0..32]);
            m
        };
        if kass_token_mint != *kass_mint_ai.key() {
            return Err(KassandraError::InvalidAccount.into());
        }
        Burn {
            account: creator_kass_ai,
            mint: kass_mint_ai,
            authority: creator_ai,
            amount: fee,
        }
        .invoke()?;
    }
    // Persist the new EMA state (protocol is writable).
    {
        let mut protocol_mut = protocol;
        protocol_mut.fee_ema = bumped_fee_ema(decayed_ema);
        protocol_mut.last_creation_unix = now_ts;
        let mut data = protocol_ai.try_borrow_mut_data()?;
        data[..Protocol::LEN].copy_from_slice(bytemuck::bytes_of(&protocol_mut));
    }

    // --- create the stake vault (program-signed) ---------------------------
    // Create the bare SPL token account at the vault PDA, then initialize it on
    // the KASS mint with the oracle PDA as its token authority.
    let vault_rent = Rent::get()?.minimum_balance(SPL_TOKEN_ACCOUNT_LEN);
    let vault_bump_seed = [vault_bump];
    let vault_seeds = [
        Seed::from(b"vault".as_ref()),
        Seed::from(oracle_ai.key().as_ref()),
        Seed::from(&vault_bump_seed),
    ];
    create_pda(
        creator_ai,
        stake_vault_ai,
        &vault_seeds,
        vault_rent,
        SPL_TOKEN_ACCOUNT_LEN,
        &pinocchio_token::ID,
    )?;
    InitializeAccount3 {
        account: stake_vault_ai,
        mint: kass_mint_ai,
        owner: oracle_ai.key(),
    }
    .invoke()?;

    // --- create + initialize the Oracle (program-signed) -------------------
    let oracle_rent = Rent::get()?.minimum_balance(Oracle::LEN);
    let oracle_bump_seed = [oracle_bump];
    let oracle_seeds = [
        Seed::from(b"oracle".as_ref()),
        Seed::from(nonce_le.as_ref()),
        Seed::from(&oracle_bump_seed),
    ];
    create_pda(
        creator_ai,
        oracle_ai,
        &oracle_seeds,
        oracle_rent,
        Oracle::LEN,
        program_id,
    )?;

    // Use the snapshotted proposal window (== PROPOSAL_WINDOW by default) so the
    // window and the per-oracle snapshot below stay consistent.
    let phase_ends_at = deadline
        .checked_add(protocol.proposal_window)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    let mut oracle = Oracle::zeroed();
    oracle.account_type = AccountType::Oracle.as_u8();
    oracle.creator = *creator_ai.key();
    oracle.kass_mint = protocol.kass_mint;
    oracle.usdc_mint = protocol.usdc_mint;
    oracle.stake_vault = *stake_vault_ai.key();
    oracle.deadline = deadline;
    oracle.phase_ends_at = phase_ends_at;
    oracle.twap_window = twap_window;
    oracle.options_count = options_count;
    oracle.set_phase(Phase::Proposal);
    oracle.proposer_count = 0;
    oracle.surviving_count = 0;
    oracle.fact_count = 0;
    oracle.total_oracle_stake = 0;
    oracle.bond_pool = 0;
    oracle.dispute_bond_total = 0;
    oracle.settled_count = 0;
    oracle.ai_finalized_count = 0;
    oracle.resolved_option = 0;
    oracle.open_challenge_count = 0;
    oracle.prompt_hash = prompt_hash;
    oracle.bump = oracle_bump;
    // Snapshot the governable behavioral params from the Protocol (F2). The
    // downstream processors read these from the Oracle, so an in-flight oracle
    // keeps its snapshot even if governance retunes the Protocol mid-dispute.
    oracle.threshold_num = protocol.threshold_num;
    oracle.threshold_den = protocol.threshold_den;
    oracle.market_threshold_num = protocol.market_threshold_num;
    oracle.market_threshold_den = protocol.market_threshold_den;
    oracle.flip_slash_num = protocol.flip_slash_num;
    oracle.flip_slash_den = protocol.flip_slash_den;
    oracle.phase_window = protocol.phase_window;
    oracle.proposal_window = protocol.proposal_window;
    oracle.fact_vote_slash_num = protocol.fact_vote_slash_num;
    oracle.fact_vote_slash_den = protocol.fact_vote_slash_den;
    oracle.reward_proposer_weight = protocol.reward_proposer_weight;
    oracle.reward_fact_weight = protocol.reward_fact_weight;
    // Snapshot the challenge-fee config (C1) too.
    oracle.challenge_fail_usdc_fee_num = protocol.challenge_fail_usdc_fee_num;
    oracle.challenge_fail_usdc_fee_den = protocol.challenge_fail_usdc_fee_den;
    oracle.challenge_success_kass_fee_num = protocol.challenge_success_kass_fee_num;
    oracle.challenge_success_kass_fee_den = protocol.challenge_success_kass_fee_den;
    {
        let mut data = oracle_ai.try_borrow_mut_data()?;
        data.copy_from_slice(bytemuck::bytes_of(&oracle));
    }

    Ok(())
}

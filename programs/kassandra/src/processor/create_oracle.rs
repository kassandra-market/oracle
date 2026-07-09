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
//! # Emission minted at creation (Task S3 / design "Emissions")
//! After the fee burn, the program computes `reward_emission = (total_supply_cap
//! − kass_supply) · emission_num/den` (u128, floor; 0 when emission is disabled)
//! and, if positive, MINTS it into the new oracle's `stake_vault` — program-
//! signed by the **mint-authority PDA** `[b"mint_authority"]`, which MUST be the
//! `kass_mint`'s SPL mint authority (else [`KassandraError::BadMintAuthority`]).
//! Reading supply AFTER the burn means the burn boosts the same-tx reservoir. The
//! amount is recorded as `Oracle.reward_emission`: `finalize_oracle` folds it into
//! `reward_pool` on Resolved and burns it back on InvalidDeadend.
//!
//! # Accounts
//! 0. protocol            — writable; pins the canonical mints + holds/updates `fee_ema`
//! 1. oracle PDA          — writable, uninitialized (created here)
//! 2. stake_vault PDA     — writable, uninitialized (created + initialized here; emission minted in)
//! 3. creator             — signer, writable; pays rent, recorded as `creator`, burn authority
//! 4. kass_mint           — writable (burn decrements / emission increments supply); == `protocol.kass_mint`
//! 5. usdc_mint           — must equal `protocol.usdc_mint`
//! 6. token program
//! 7. system program
//! 8. creator_kass_token  — writable; KASS token account on `kass_mint` the fee is burned from
//! 9. mint_authority PDA  — `[b"mint_authority"]`; program-signs the emission `MintTo` (must
//!    equal the `kass_mint`'s SPL mint authority)
//!
//! # Instruction payload (after the 1-byte discriminant), exactly 57 bytes
//! `nonce: u64 LE` ++ `options_count: u8` ++ `deadline: i64 LE` ++
//! `twap_window: i64 LE`. (The former `prompt_hash` was removed — the plaintext
//! subject now lives on-chain in the companion `oracle_meta` account.)

use bytemuck::Zeroable;
use pinocchio::{
    account::AccountView as AccountInfo,
    address::Address as Pubkey,
    cpi::{Seed, Signer},
    error::ProgramError,
    ProgramResult,
};
use pinocchio_token::instructions::{Burn, InitializeAccount3, MintTo};
use pinocchio_token::state::Account as TokenAccount;

use crate::{
    clock::now,
    config::MINT_AUTHORITY_SEED,
    error::KassandraError,
    fee::{bumped_fee_ema, decay_fee_ema, fee_for_ema},
    processor::guards::{assert_key, assert_signer, create_pda, load_protocol},
    rent::minimum_rent,
    state::{AccountType, Oracle, Phase, Protocol},
};

/// Exact payload length: nonce[8] ++ options_count[1] ++ deadline[8] ++
/// twap_window[8].
const PAYLOAD_LEN: usize = 25;

pub fn process(program_id: &Pubkey, accounts: &mut [AccountInfo], payload: &[u8]) -> ProgramResult {
    // --- payload parse (exact length) --------------------------------------
    if payload.len() != PAYLOAD_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let nonce = u64::from_le_bytes(payload[0..8].try_into().unwrap());
    let options_count = payload[8];
    let deadline = i64::from_le_bytes(payload[9..17].try_into().unwrap());
    let twap_window = i64::from_le_bytes(payload[17..25].try_into().unwrap());

    let [protocol_ai, oracle_ai, stake_vault_ai, creator_ai, kass_mint_ai, usdc_mint_ai, token_prog_ai, system_prog_ai, creator_kass_ai, mint_authority_ai, ..] =
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
    let (expected_oracle, oracle_bump) =
        Pubkey::find_program_address(&[b"oracle", &nonce_le], program_id);
    assert_key(oracle_ai, &expected_oracle)?;

    let (expected_vault, vault_bump) =
        Pubkey::find_program_address(&[b"vault", oracle_ai.address().as_ref()], program_id);
    assert_key(stake_vault_ai, &expected_vault)?;

    // Reject if the oracle PDA already exists (a duplicate nonce).
    if oracle_ai.lamports() != 0 || !oracle_ai.is_data_empty() {
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
            let data = creator_kass_ai.try_borrow()?;
            if data.len() < 32 {
                return Err(KassandraError::InvalidAccount.into());
            }
            let mut m = [0u8; 32];
            m.copy_from_slice(&data[0..32]);
            m
        };
        if kass_token_mint != kass_mint_ai.address().to_bytes() {
            return Err(KassandraError::InvalidAccount.into());
        }
        Burn::new(creator_kass_ai, kass_mint_ai, creator_ai, fee).invoke()?;
    }
    // Persist the new EMA state (protocol is writable).
    {
        let mut protocol_mut = protocol;
        protocol_mut.fee_ema = bumped_fee_ema(decayed_ema);
        protocol_mut.last_creation_unix = now_ts;
        let mut data = protocol_ai.try_borrow_mut()?;
        data[..Protocol::LEN].copy_from_slice(bytemuck::bytes_of(&protocol_mut));
    }

    // --- create the stake vault (program-signed) ---------------------------
    // Create the bare SPL token account at the vault PDA, then initialize it on
    // the KASS mint with the oracle PDA as its token authority.
    let vault_rent = minimum_rent(TokenAccount::LEN)?;
    let vault_bump_seed = [vault_bump];
    let vault_seeds = [
        Seed::from(b"vault".as_ref()),
        Seed::from(oracle_ai.address().as_ref()),
        Seed::from(&vault_bump_seed),
    ];
    create_pda(
        creator_ai,
        stake_vault_ai,
        &vault_seeds,
        vault_rent,
        TokenAccount::LEN,
        &pinocchio_token::ID,
    )?;
    InitializeAccount3 {
        account: stake_vault_ai,
        mint: kass_mint_ai,
        owner: oracle_ai.address(),
    }
    .invoke()?;

    // --- emission minted at creation from the reservoir (Task S3) ----------
    // Read the circulating supply AFTER the burn (so the burn boosts the same-tx
    // reservoir), compute `reward_emission = (cap − supply)·num/den` (u128, floor;
    // 0 when emission is disabled), and — if positive — mint it into stake_vault,
    // program-signed by the mint-authority PDA. The PDA MUST be the kass_mint's
    // SPL mint authority, else minting could be spoofed.
    let reward_emission = compute_reward_emission(
        kass_mint_ai,
        protocol.total_supply_cap,
        protocol.emission_num,
        protocol.emission_den,
    )?;
    if reward_emission > 0 {
        // Verify + derive the mint-authority PDA, then assert it is the kass_mint's
        // SPL mint authority (the bootstrapping requirement).
        let (expected_mint_auth, mint_auth_bump) =
            Pubkey::find_program_address(&[MINT_AUTHORITY_SEED], program_id);
        assert_key(mint_authority_ai, &expected_mint_auth)?;
        assert_mint_authority_is(kass_mint_ai, &expected_mint_auth)?;

        let mint_auth_bump_seed = [mint_auth_bump];
        let mint_auth_seeds = [
            Seed::from(MINT_AUTHORITY_SEED),
            Seed::from(&mint_auth_bump_seed),
        ];
        MintTo::new(
            kass_mint_ai,
            stake_vault_ai,
            mint_authority_ai,
            reward_emission,
        )
        .invoke_signed(&[Signer::from(&mint_auth_seeds)])?;
    }

    // --- create + initialize the Oracle (program-signed) -------------------
    let oracle_rent = minimum_rent(Oracle::LEN)?;
    let oracle_bump_seed = [oracle_bump];
    let oracle_seeds = Oracle::signer_seeds(&nonce_le, &oracle_bump_seed);
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
    oracle.creator = *creator_ai.address();
    oracle.kass_mint = protocol.kass_mint;
    oracle.usdc_mint = protocol.usdc_mint;
    oracle.stake_vault = *stake_vault_ai.address();
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
    oracle.bump = oracle_bump;
    // S3: record the KASS minted into stake_vault at creation. finalize_oracle
    // folds it into reward_pool on Resolved / burns it back on InvalidDeadend.
    oracle.reward_emission = reward_emission;
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
    // Bootstrapping: snapshot the activity-scaled minimum-stake floor from the SAME
    // decayed fee-EMA used for the creation fee (so the floor tracks recent creation
    // activity). 0 at genesis / low activity or while disabled → free participation.
    oracle.min_stake = crate::stake_floor::stake_floor(
        decayed_ema,
        protocol.stake_floor_ema_threshold,
        protocol.stake_floor_ema_cap,
        protocol.stake_floor_max,
    );
    {
        let mut data = oracle_ai.try_borrow_mut()?;
        data.copy_from_slice(bytemuck::bytes_of(&oracle));
    }

    Ok(())
}

/// SPL `Mint` byte offsets (mirrors `spl_token::state::Mint`): the
/// `mint_authority` is a `COption<Pubkey>` — a 4-byte LE tag (`1` == `Some`)
/// followed by the 32-byte pubkey — and the `supply: u64` immediately follows.
const MINT_AUTHORITY_TAG_OFFSET: usize = 0;
const MINT_AUTHORITY_KEY_OFFSET: usize = 4;
const MINT_SUPPLY_OFFSET: usize = 36;

/// Compute `reward_emission = (total_supply_cap − supply) · num / den` (u128
/// intermediate, floor → u64), reading the circulating `supply` from the
/// (post-burn) `kass_mint` account. Returns 0 when emission is disabled
/// (`num == 0` or `cap <= supply`). Overflow-safe: the reservoir and the product
/// are computed in u128; the floor result fits u64 because it never exceeds the
/// reservoir (`num <= den`). `den == 0` is impossible (set_config / init keep it
/// positive) but is defended as 0.
fn compute_reward_emission(
    kass_mint_ai: &AccountInfo,
    total_supply_cap: u64,
    emission_num: u64,
    emission_den: u64,
) -> Result<u64, ProgramError> {
    if emission_num == 0 || emission_den == 0 || total_supply_cap == 0 {
        return Ok(0);
    }
    let data = kass_mint_ai.try_borrow()?;
    if data.len() < MINT_SUPPLY_OFFSET + 8 {
        return Err(KassandraError::InvalidAccount.into());
    }
    let supply = u64::from_le_bytes(
        data[MINT_SUPPLY_OFFSET..MINT_SUPPLY_OFFSET + 8]
            .try_into()
            .unwrap(),
    );
    let reservoir = (total_supply_cap as u128).saturating_sub(supply as u128);
    // Clamp to the reservoir so the `as u64` cast is self-protecting: for any
    // valid config `num <= den` keeps `emission <= reservoir`, but a future bad
    // config (`num > den`) could otherwise mint MORE than the reservoir holds.
    // The `.min(reservoir)` makes the bound local rather than relying on the
    // non-local set_config invariant; a no-op for every valid config.
    let emission = (reservoir * (emission_num as u128) / (emission_den as u128)).min(reservoir);
    // `emission <= reservoir <= total_supply_cap <= u64::MAX`, so this fits u64.
    Ok(emission as u64)
}

/// Assert the SPL `kass_mint`'s `mint_authority` is `Some(expected)`, else
/// [`KassandraError::BadMintAuthority`]. A `None` authority (a fixed-supply mint)
/// or any other key is rejected — emission requires the program PDA be the sole
/// minter.
fn assert_mint_authority_is(kass_mint_ai: &AccountInfo, expected: &Pubkey) -> ProgramResult {
    let data = kass_mint_ai.try_borrow()?;
    if data.len() < MINT_AUTHORITY_KEY_OFFSET + 32 {
        return Err(KassandraError::BadMintAuthority.into());
    }
    let tag = u32::from_le_bytes(
        data[MINT_AUTHORITY_TAG_OFFSET..MINT_AUTHORITY_TAG_OFFSET + 4]
            .try_into()
            .unwrap(),
    );
    if tag != 1 {
        return Err(KassandraError::BadMintAuthority.into());
    }
    if &data[MINT_AUTHORITY_KEY_OFFSET..MINT_AUTHORITY_KEY_OFFSET + 32] != expected.as_ref() {
        return Err(KassandraError::BadMintAuthority.into());
    }
    Ok(())
}

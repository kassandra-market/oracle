//! `init_protocol`: one-time creation of the [`Protocol`] singleton.
//!
//! Creates the `[b"protocol"]` PDA recording the admin and the canonical
//! KASS/USDC mints (so a later `create_oracle` fee-burn cannot be spoofed with a
//! fake KASS mint), with the fee-EMA state zeroed (genesis is free; the dynamic
//! fee is Task H2).
//!
//! # Bootstrap-DoS resistance: create-or-adopt (Allocate + Assign)
//! The `[b"protocol"]` address is deterministic and known at deploy time, so an
//! attacker could pre-fund it with 1 lamport before the first `init_protocol`.
//! A plain system `CreateAccount` FAILS on any already-funded account, which
//! would brick the entire program permanently (every `create_oracle` needs the
//! Protocol account). To tolerate a pre-funded singleton we instead **adopt**
//! it: top the balance up to rent-exempt with a system `Transfer` (only if it is
//! short), then system `Allocate` the space and `Assign` ownership to this
//! program — both signed by the protocol PDA seeds. A system-owned, attacker-
//! pre-funded account carries no data and cannot have been `Allocate`d by anyone
//! but the PDA signer, so adoption always succeeds.
//!
//! Idempotency is enforced by the **account-type tag, not lamports**: a real
//! second init finds an account ALREADY owned by this program and stamped
//! [`AccountType::Protocol`], and fails [`KassandraError::AlreadyInitialized`].
//!
//! # Protocol PDA seeds (CONTRACT)
//! `[b"protocol"]` (singleton), program = [`crate::ID`].
//!
//! # Accounts
//! 0. protocol PDA   — writable; created-or-adopted here, signs via its seeds
//! 1. admin          — signer, writable; tops up rent, recorded as `admin`
//! 2. kass_mint      — canonical KASS mint (owned by the SPL token program)
//! 3. usdc_mint      — canonical USDC mint (owned by the SPL token program)
//! 4. system program
//!
//! # Instruction payload
//! None (any trailing bytes are ignored).

use bytemuck::Zeroable;
use pinocchio::{
    account::AccountView as AccountInfo,
    address::Address as Pubkey,
    cpi::{Seed, Signer},
    error::ProgramError,
    ProgramResult,
};
use pinocchio_system::instructions::{Allocate, Assign, Transfer};

use crate::{
    error::KassandraError,
    processor::guards::{assert_key, assert_signer, PROTOCOL_BUMP, PROTOCOL_PDA},
    rent::minimum_rent,
    state::{AccountType, Protocol},
};

pub fn process(
    program_id: &Pubkey,
    accounts: &mut [AccountInfo],
    _payload: &[u8],
) -> ProgramResult {
    let [protocol_ai, admin_ai, kass_mint_ai, usdc_mint_ai, system_prog_ai, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // --- account validation -------------------------------------------------
    assert_signer(admin_ai)?;
    assert_key(system_prog_ai, &pinocchio_system::ID)?;

    // The protocol PDA must be exactly the singleton address — compare against
    // the precomputed const + use its known bump, skipping `find_program_address`.
    assert_key(protocol_ai, &PROTOCOL_PDA)?;
    let bump = PROTOCOL_BUMP;

    // Re-init guard via the account-type TAG (not lamports): a genuine second
    // init finds the account already owned by this program AND stamped
    // `Protocol`. A freshly-created or attacker-pre-funded-but-system-owned
    // account is not yet program-owned, so adoption below proceeds.
    if protocol_ai.owned_by(program_id) {
        let data = protocol_ai.try_borrow()?;
        if data.len() >= Protocol::LEN && data[0] == AccountType::Protocol.as_u8() {
            return Err(KassandraError::AlreadyInitialized.into());
        }
    }

    // Cheap defense-in-depth: the recorded mints must be SPL token-program
    // accounts (not arbitrary keys), so H1/H2 can trust them as canonical mints.
    if !kass_mint_ai.owned_by(&pinocchio_token::ID) || !usdc_mint_ai.owned_by(&pinocchio_token::ID)
    {
        return Err(KassandraError::InvalidAccount.into());
    }

    // --- create-or-adopt the Protocol account (program-signed) --------------
    let rent = minimum_rent(Protocol::LEN)?;
    let bump_seed = [bump];
    let signer_seeds = [Seed::from(b"protocol".as_ref()), Seed::from(&bump_seed)];

    // Top the (possibly pre-funded) account up to rent-exempt, only if short.
    let current = protocol_ai.lamports();
    if current < rent {
        Transfer {
            from: admin_ai,
            to: protocol_ai,
            lamports: rent - current,
        }
        .invoke()?;
    }
    // Allocate the data and take ownership — both signed by the PDA. Tolerates a
    // pre-funded account where a plain CreateAccount would fail.
    Allocate {
        account: protocol_ai,
        space: Protocol::LEN as u64,
    }
    .invoke_signed(&[Signer::from(&signer_seeds)])?;
    Assign {
        account: protocol_ai,
        owner: program_id,
    }
    .invoke_signed(&[Signer::from(&signer_seeds)])?;

    // --- initialize the Protocol --------------------------------------------
    let mut protocol = Protocol::zeroed();
    protocol.account_type = AccountType::Protocol.as_u8();
    protocol.admin = *admin_ai.address();
    protocol.kass_mint = *kass_mint_ai.address();
    protocol.usdc_mint = *usdc_mint_ai.address();
    protocol.fee_ema = 0;
    protocol.last_creation_unix = 0;
    protocol.bump = bump;
    // Governance linkage unset until the one-time `set_governance` handoff.
    protocol.governance_set = 0;
    // dao_authority / kass_dao stay zeroed (set by `set_governance`).
    //
    // Emission is ON by default (config consts). `create_oracle` mints
    // `reward_emission = (total_supply_cap − kass_supply)·num/den` into each new
    // oracle's stake_vault, program-signed by the mint-authority PDA (which MUST
    // be the kass_mint's SPL authority). Resolved oracles distribute it to the
    // correct proposer cohort; InvalidDeadend burns it back to the reservoir.
    protocol.emission_num = crate::config::EMISSION_NUM;
    protocol.emission_den = crate::config::EMISSION_DEN;
    protocol.total_supply_cap = crate::config::TOTAL_SUPPLY_CAP;
    protocol.fee_ema_halflife = crate::config::FEE_EMA_HALFLIFE_SECS;
    protocol.fee_per_ema_unit = crate::config::FEE_PER_EMA_UNIT;
    protocol.fee_ema_increment = crate::config::FEE_EMA_INCREMENT;
    // Governable behavioral params (F2): default to the current `config.rs`
    // consts so the snapshot onto each Oracle reproduces today's behavior
    // exactly. MARKET_THRESHOLD_* are `u128` consts stored as u64 (their values
    // fit; widened back to u128 on use in settle_challenge).
    protocol.threshold_num = crate::config::THRESHOLD_NUM;
    protocol.threshold_den = crate::config::THRESHOLD_DEN;
    protocol.market_threshold_num = crate::config::MARKET_THRESHOLD_NUM as u64;
    protocol.market_threshold_den = crate::config::MARKET_THRESHOLD_DEN as u64;
    protocol.flip_slash_num = crate::config::FLIP_SLASH_NUM;
    protocol.flip_slash_den = crate::config::FLIP_SLASH_DEN;
    protocol.phase_window = crate::config::PHASE_WINDOW;
    protocol.proposal_window = crate::config::PROPOSAL_WINDOW;
    // Staker-settlement reward economics (S1): default to the config consts.
    // Reward weights PW/FW = 2/1 (PW > FW); approve-voter rejected-fact slash
    // 1/2. These satisfy the `set_config` bounds (at least one reward weight > 0;
    // fact_vote_slash den > 0 and num <= den) and are snapshotted onto each
    // Oracle at create_oracle.
    protocol.fact_vote_slash_num = crate::config::FACT_VOTE_SLASH_NUM;
    protocol.fact_vote_slash_den = crate::config::FACT_VOTE_SLASH_DEN;
    protocol.reward_proposer_weight = crate::config::REWARD_PROPOSER_WEIGHT;
    protocol.reward_fact_weight = crate::config::REWARD_FACT_WEIGHT;
    // Challenge-fee config (C1): default to the config consts (1/100 each).
    protocol.challenge_fail_usdc_fee_num = crate::config::CHALLENGE_FAIL_USDC_FEE_NUM;
    protocol.challenge_fail_usdc_fee_den = crate::config::CHALLENGE_FAIL_USDC_FEE_DEN;
    protocol.challenge_success_kass_fee_num = crate::config::CHALLENGE_SUCCESS_KASS_FEE_NUM;
    protocol.challenge_success_kass_fee_den = crate::config::CHALLENGE_SUCCESS_KASS_FEE_DEN;
    {
        let mut data = protocol_ai.try_borrow_mut()?;
        data.copy_from_slice(bytemuck::bytes_of(&protocol));
    }

    Ok(())
}
